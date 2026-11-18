//! Temporal orchestration for native, VPN-bound pairing sessions.

use std::{future::Future, io, net::Ipv4Addr, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use sqlx::PgPool;
use temporalio_client::{
    Client as TemporalClient, ClientOptions, Connection, ConnectionOptions,
    WorkflowIdConflictPolicy, WorkflowIdReusePolicy, WorkflowStartOptions,
};
use temporalio_common::RetryPolicy;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, Runtime, Worker, WorkerOptions, WorkflowContext, WorkflowResult,
    activities::{ActivityContext, ActivityError},
    runtime::worker_tuner::{FixedSizeSlotSupplier, TunerHolder},
};
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until};
use uuid::Uuid;

use crate::{
    agent::{Client, VpnState},
    db::{self, pairing_codes::PairingCodeRepository, pairings::PairingStore},
    pairing::Session,
};

const TASK_QUEUE: &str = "apprelay-pairing";
const CONNECTION_LIMIT: Duration = Duration::from_secs(120);
const SESSION_LIMIT: Duration = Duration::from_secs(60);

type Error = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn client() -> Result<TemporalClient, Error> {
    let address = setting("APPRELAY_TEMPORAL_ADDRESS", "http://127.0.0.1:7233")?;
    let namespace = setting("APPRELAY_TEMPORAL_NAMESPACE", "default")?;
    let address = address.parse::<temporalio_client::Url>()?;
    let connection = Connection::connect(ConnectionOptions::new(address).build()).await?;
    Ok(TemporalClient::new(
        connection,
        ClientOptions::new(namespace).build(),
    )?)
}

fn setting(name: &str, default: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(error) => Err(error),
    }
}

/// The same device cannot have two open workflows. Closed sessions may be rerun.
pub async fn start(device_id: Uuid) -> Result<(), Error> {
    client()
        .await?
        .start_workflow(
            PairDeviceWorkflow::run,
            device_id.to_string(),
            WorkflowStartOptions::new(TASK_QUEUE, format!("pair-device-{device_id}"))
                .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                .id_reuse_policy(WorkflowIdReusePolicy::AllowDuplicate)
                .build(),
        )
        .await?;
    tracing::info!("Pairing workflow queued");
    Ok(())
}

#[workflow]
#[derive(Default)]
pub struct PairDeviceWorkflow;

#[workflow_methods]
impl PairDeviceWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, device_id: String) -> WorkflowResult<()> {
        // Waiting for the tunnel is separate from the interactive pairing window.
        // No listener, advertisement, PIN, or pairing identity exists yet.
        ctx.execute_activity(
            PairingActivities::wait_for_connection,
            device_id.clone(),
            ActivityOptions::with_start_to_close_timeout(CONNECTION_LIMIT)
                .heartbeat_timeout(Duration::from_secs(10))
                .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                .build(),
        )
        .await?;
        ctx.execute_activity(
            PairingActivities::pair_device,
            device_id,
            ActivityOptions::with_start_to_close_timeout(SESSION_LIMIT)
                .heartbeat_timeout(Duration::from_secs(10))
                // Interactive pairing must not restart with a new PIN/identity on retry.
                .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                .build(),
        )
        .await?;
        Ok(())
    }
}

// Separate workflow type preserves command history for existing CLI workflows.
#[workflow]
#[derive(Default)]
pub struct PairAttemptWorkflow;

#[workflow_methods]
impl PairAttemptWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: String) -> WorkflowResult<()> {
        let result = async {
            ctx.execute_activity(
                PairingActivities::wait_for_connection,
                input.clone(),
                ActivityOptions::with_start_to_close_timeout(CONNECTION_LIMIT)
                    .heartbeat_timeout(Duration::from_secs(10))
                    .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                    .build(),
            )
            .await?;
            ctx.execute_activity(
                PairingActivities::pair_device,
                input.clone(),
                ActivityOptions::with_start_to_close_timeout(SESSION_LIMIT)
                    .heartbeat_timeout(Duration::from_secs(10))
                    .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                    .build(),
            )
            .await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            ctx.execute_activity(
                PairingActivities::fail_attempt,
                input,
                ActivityOptions::with_start_to_close_timeout(Duration::from_secs(10))
                    .retry_policy(RetryPolicy::builder().maximum_attempts(3).build())
                    .build(),
            )
            .await?;
        }
        result
    }
}

fn activity_ids(input: &str) -> io::Result<(Uuid, Option<Uuid>)> {
    let (device, attempt) = match input.split_once('/') {
        Some((device, attempt)) => (device, Some(attempt)),
        None => (input, None),
    };
    Ok((
        Uuid::parse_str(device).map_err(io::Error::other)?,
        attempt
            .map(Uuid::parse_str)
            .transpose()
            .map_err(io::Error::other)?,
    ))
}

pub struct PairingActivities {
    pool: PgPool,
    agent: Client,
    pairings: PairingStore,
    codes: PairingCodeRepository,
}

#[activities]
impl PairingActivities {
    #[activity]
    pub async fn fail_attempt(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: String,
    ) -> Result<(), ActivityError> {
        let (_, attempt) = activity_ids(&input)?;
        sqlx::query(
            "UPDATE public_pairing_attempts SET status = 'failed',
             reason = CASE WHEN expires_at <= clock_timestamp() THEN 'timeout' ELSE 'pairing_failed' END
             WHERE id = $1 AND status = 'pairing'",
        )
        .bind(attempt)
        .execute(&self.pool)
        .await
        .inspect_err(|_| crate::telemetry::report("Could not finish pairing attempt"))
        .map_err(|_| io::Error::other("could not finish pairing attempt"))?;
        Ok(())
    }

    #[activity]
    pub async fn wait_for_connection(
        self: Arc<Self>,
        ctx: ActivityContext,
        device_id: String,
    ) -> Result<(), ActivityError> {
        let (device_id, public_attempt_id) = activity_ids(&device_id)?;
        let operation = async {
            let attempt_expiry = self.require_attempt(device_id, public_attempt_id).await?;
            let (public_key, expires_at) = sqlx::query_as::<_, (Vec<u8>, DateTime<Utc>)>(
                "SELECT wireguard_public_key, expires_at FROM devices
                     WHERE id = $1 AND expires_at > statement_timestamp()",
            )
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("device is missing or expired"))?;
            let public_key: [u8; 32] = public_key
                .try_into()
                .map_err(|_| io::Error::other("invalid device WireGuard public key"))?;
            tracing::info!("Waiting for device to establish its WireGuard connection");
            let mut last_error = None;
            loop {
                if Utc::now() >= attempt_expiry.unwrap_or(expires_at).min(expires_at) {
                    return Err(io::Error::other("device expired while waiting for VPN"));
                }
                match self.agent.require_connected_peer(public_key).await {
                    Ok(()) => {
                        tracing::info!("WireGuard connection confirmed");
                        return Ok(());
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_ref() != Some(&message) {
                            tracing::debug!(error_kind = ?error.kind(), "WireGuard peer is not ready");
                            last_error = Some(message);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        };
        wait(
            &ctx,
            operation,
            Instant::now() + CONNECTION_LIMIT,
            "VPN connection was not established within 120 seconds",
        )
        .await
    }

    #[activity]
    pub async fn pair_device(
        self: Arc<Self>,
        ctx: ActivityContext,
        device_id: String,
    ) -> Result<(), ActivityError> {
        let (device_id, public_attempt_id) = activity_ids(&device_id)?;
        let deadline = Instant::now() + SESSION_LIMIT;
        let prepare = async {
            let attempt_expiry = self.require_attempt(device_id, public_attempt_id).await?;
            // Resolve the address at execution time, never from a queued IP payload.
            let (address, expires_at, attempt_started_at, public_key) =
                sqlx::query_as::<_, (String, DateTime<Utc>, DateTime<Utc>, Vec<u8>)>(
                    "SELECT host(address), expires_at, statement_timestamp(), wireguard_public_key FROM devices
                     WHERE id = $1 AND expires_at > statement_timestamp()",
                )
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("device is missing or expired"))?;
            let peer = address.parse::<Ipv4Addr>().map_err(io::Error::other)?;
            let status = self.agent.status().await?;
            let subnet =
                Ipv4Net::new(status.address, status.prefix_len).map_err(io::Error::other)?;
            if status.state != VpnState::Running
                || !subnet.contains(&peer)
                || peer == status.address
                || peer == subnet.network()
                || peer == subnet.broadcast()
            {
                return Err(io::Error::other(
                    "device address does not belong to the running VPN",
                ));
            }
            let remaining = (attempt_expiry.unwrap_or(expires_at).min(expires_at) - Utc::now())
                .to_std()
                .map_err(io::Error::other)?;
            let lease_deadline = Instant::now() + remaining;
            // Recheck at activity execution time in case it was queued after the
            // readiness activity completed. Do not create a session for a stale peer.
            let public_key: [u8; 32] = public_key
                .try_into()
                .map_err(|_| io::Error::other("invalid device WireGuard public key"))?;
            self.agent.require_connected_peer(public_key).await?;
            let session = Session::new(&status, peer).await?;
            let code_expiry = Utc::now()
                + chrono::Duration::from_std(
                    deadline
                        .min(lease_deadline)
                        .saturating_duration_since(Instant::now()),
                )
                .map_err(io::Error::other)?;
            let attempt_id = self
                .codes
                .begin(
                    device_id,
                    expires_at,
                    attempt_started_at,
                    code_expiry,
                    public_attempt_id,
                )
                .await?;
            tracing::info!("Starting pairing activity");
            Ok((
                session,
                deadline.min(lease_deadline),
                expires_at,
                attempt_started_at,
                attempt_id,
            ))
        };
        let (mut session, deadline, expires_at, attempt_started_at, attempt_id) =
            wait(&ctx, prepare, deadline, "pairing session expired").await?;
        let operation = async {
            let codes = &self.codes;
            let record = session
                .run(move |pin| async move {
                    match pin {
                        Some(pin) => codes.publish(attempt_id, device_id, pin).await,
                        None => codes.clear(attempt_id).await,
                    }
                })
                .await?;
            // Persist inside the activity: credentials must not enter Temporal history.
            self.pairings
                .save(
                    device_id,
                    (attempt_id, public_attempt_id),
                    expires_at,
                    attempt_started_at,
                    &record,
                )
                .await?;
            tracing::info!("Encrypted pairing saved; iOS completion is unverified");
            Ok(())
        };
        let result = wait(&ctx, operation, deadline, "pairing session expired").await;
        // Normal completion, cancellation, and timeout all withdraw the proxy service.
        // A crash still expires its last announcement after the ten-second DNS TTL.
        session.stop().await;
        // Best effort on cancellation as well; expiry also handles process crashes.
        let ended = tokio::time::timeout(Duration::from_secs(2), self.codes.end(attempt_id)).await;
        if !matches!(ended, Ok(Ok(()))) {
            crate::telemetry::report("Could not invalidate pairing code");
            tracing::warn!(
                "Could not invalidate pairing code; it will expire at the session deadline"
            );
        }
        result
    }
}

impl PairingActivities {
    async fn require_attempt(
        &self,
        device: Uuid,
        attempt: Option<Uuid>,
    ) -> io::Result<Option<DateTime<Utc>>> {
        let valid = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT a.expires_at FROM public_pairing_attempts a JOIN devices d ON d.id = a.device_id
                WHERE a.device_id = $1 AND ($2::uuid IS NULL OR a.id = $2)
                  AND a.status = 'pairing' AND a.expires_at > clock_timestamp()
                  AND d.expires_at = a.device_expires_at AND d.expires_at > clock_timestamp()",
        )
        .bind(device)
        .bind(attempt)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| io::Error::other("could not validate pairing attempt"))?;
        // CLI sessions cannot race a web-owned session.
        if valid.is_some() != attempt.is_some() {
            return Err(io::Error::other(
                "pairing attempt expired, changed, or device is busy",
            ));
        }
        Ok(valid)
    }
}

// A committed claim survives HTTP cancellation and Temporal outages. Workflow IDs
// deduplicate uncertain enqueue results, including already completed workflows.
async fn dispatch_attempts(pool: PgPool) {
    let mut was_unavailable = false;
    loop {
        let mut dispatch_failed = false;
        let result = async {
            sqlx::query("UPDATE public_pairing_attempts SET status = 'expired'
                WHERE status IN ('pending', 'pairing') AND expires_at <= clock_timestamp()")
                .execute(&pool).await?;
            let rows = sqlx::query_as::<_, (Uuid, Uuid)>(
                "SELECT id, device_id FROM public_pairing_attempts
                 WHERE status = 'pairing' AND queued_at IS NULL AND expires_at > clock_timestamp()
                 ORDER BY expires_at LIMIT 100",
            ).fetch_all(&pool).await?;
            if rows.is_empty() {
                return Ok::<(), Error>(());
            }
            let client = client().await?;
            for (attempt, device) in rows {
                let started = client.start_workflow(
                    PairAttemptWorkflow::run,
                    format!("{device}/{attempt}"),
                    WorkflowStartOptions::new(TASK_QUEUE, format!("pair-attempt-{attempt}"))
                        .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
                        .build(),
                ).await;
                if started.is_err() {
                    dispatch_failed = true;
                    tracing::warn!("Could not enqueue pairing attempt; will retry");
                    continue;
                }
                sqlx::query("UPDATE public_pairing_attempts SET queued_at = clock_timestamp() WHERE id = $1")
                    .bind(attempt).execute(&pool).await?;
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            tracing::warn!("Pairing attempt dispatch unavailable; durable claims will be retried");
        }
        let unavailable = result.is_err() || dispatch_failed;
        if unavailable && !was_unavailable {
            crate::telemetry::report("Pairing attempt dispatch unavailable");
        }
        was_unavailable = unavailable;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Heartbeat even during database/agent setup, and drop network futures on cancel.
async fn wait<T>(
    ctx: &ActivityContext,
    operation: impl Future<Output = io::Result<T>>,
    deadline: Instant,
    timeout_message: &'static str,
) -> Result<T, ActivityError> {
    let mut heartbeats = interval(Duration::from_secs(2));
    heartbeats.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            _ = ctx.cancelled() => return Err(ActivityError::cancelled()),
            _ = sleep_until(deadline) => return Err(io::Error::new(
                io::ErrorKind::TimedOut, timeout_message,
            ).into()),
            result = &mut operation => return result.inspect_err(|error| {
                if error.kind() != io::ErrorKind::TimedOut {
                    crate::telemetry::report("Pairing activity failed");
                }
            }).map_err(Into::into),
            _ = heartbeats.tick() => ctx.record_heartbeat(()).await?,
        }
    }
}

pub async fn run_worker() -> Result<(), Error> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
    let pairings = PairingStore::from_env(pool.clone())?;
    let codes = PairingCodeRepository::from_env(pool.clone())?;
    db::MIGRATOR.run(&pool).await?;
    let agent = Client::from_env()?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(20))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<PairDeviceWorkflow>()?
        .register_workflow::<PairAttemptWorkflow>()?
        .register_activities(PairingActivities {
            pool: pool.clone(),
            agent,
            pairings,
            codes,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(1))
        .build();
    let worker = Worker::new(&runtime, client().await?, options)?;
    tracing::info!(task_queue = TASK_QUEUE, "Pairing worker listening");
    let dispatcher = tokio::spawn(dispatch_attempts(pool));
    let result = worker
        .run_until(async {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await;
    dispatcher.abort();
    let _ = dispatcher.await;
    result?;
    Ok(())
}
