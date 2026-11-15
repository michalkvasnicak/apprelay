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
    eprintln!("Pairing workflow queued: pair-device-{device_id}");
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

pub struct PairingActivities {
    pool: PgPool,
    agent: Client,
    pairings: PairingStore,
    codes: PairingCodeRepository,
}

#[activities]
impl PairingActivities {
    #[activity]
    pub async fn wait_for_connection(
        self: Arc<Self>,
        ctx: ActivityContext,
        device_id: String,
    ) -> Result<(), ActivityError> {
        let device_id = Uuid::parse_str(&device_id)?;
        let operation = async {
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
            eprintln!("Waiting for device {device_id} to establish its WireGuard connection");
            loop {
                if Utc::now() >= expires_at {
                    return Err(io::Error::other("device expired while waiting for VPN"));
                }
                if self.agent.require_connected_peer(public_key).await.is_ok() {
                    eprintln!("WireGuard connection confirmed for device {device_id}");
                    return Ok(());
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
        let device_id = Uuid::parse_str(&device_id)?;
        let deadline = Instant::now() + SESSION_LIMIT;
        let prepare = async {
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
            let remaining = (expires_at - Utc::now())
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
                .begin(device_id, expires_at, attempt_started_at, code_expiry)
                .await?;
            eprintln!("Starting pairing activity for device {device_id} at {peer}");
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
                    attempt_id,
                    expires_at,
                    attempt_started_at,
                    &record,
                )
                .await?;
            eprintln!(
                "Encrypted pairing saved for device {device_id}; iOS completion is unverified"
            );
            Ok(())
        };
        let result = wait(&ctx, operation, deadline, "pairing session expired").await;
        // Normal completion, cancellation, and timeout all withdraw the proxy service.
        // A crash still expires its last announcement after the ten-second DNS TTL.
        session.stop().await;
        // Best effort on cancellation as well; expiry also handles process crashes.
        let ended = tokio::time::timeout(Duration::from_secs(2), self.codes.end(attempt_id)).await;
        if !matches!(ended, Ok(Ok(()))) {
            eprintln!("Could not invalidate pairing code; it will expire at the session deadline");
        }
        result
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
            result = &mut operation => return result.map_err(Into::into),
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
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<PairDeviceWorkflow>()?
        .register_activities(PairingActivities {
            pool,
            agent,
            pairings,
            codes,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(1))
        .build();
    let worker = Worker::new(&runtime, client().await?, options)?;
    eprintln!(
        "Pairing worker listening on {TASK_QUEUE}: at most 10 activities, up to 120 seconds for VPN readiness, then 60 seconds for pairing"
    );
    worker
        .run_until(async {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await?;
    Ok(())
}
