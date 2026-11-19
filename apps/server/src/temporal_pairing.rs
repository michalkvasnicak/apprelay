//! Temporal orchestration for native, VPN-bound pairing sessions.

use std::{future::Future, io, net::Ipv4Addr, sync::Arc, time::Duration};

use chrono::Utc;
use ipnet::Ipv4Net;
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
    db::{
        self,
        devices::{DevicePeer, PairingDeviceRepository, PairingTarget},
        pairing_attempts::{PairingAttemptRepository, PendingPairingAttempt},
        pairing_codes::PairingCodeRepository,
        pairings::PairingStore,
    },
    error::OperationError,
    iphone::pairing::{SetupSession, verify_saved_pairing},
    telemetry,
    vpn::agent::{Client, VpnState},
};

const TASK_QUEUE: &str = "apprelay-pairing";
const CONNECTION_LIMIT: Duration = Duration::from_secs(120);
const SESSION_LIMIT: Duration = Duration::from_secs(60);
// The VPN extension discovers and reports the port once per minute.
const VERIFICATION_LIMIT: Duration = Duration::from_secs(90);
const PAIRING_ACTIVITY_LIMIT: Duration =
    Duration::from_secs(SESSION_LIMIT.as_secs() + VERIFICATION_LIMIT.as_secs() + 5);

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
            ActivityOptions::with_start_to_close_timeout(PAIRING_ACTIVITY_LIMIT)
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
                ActivityOptions::with_start_to_close_timeout(PAIRING_ACTIVITY_LIMIT)
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

fn activity_ids(input: &str) -> Result<(Uuid, Option<Uuid>), ActivityError> {
    let (device, attempt) = match input.split_once('/') {
        Some((device, attempt)) => (device, Some(attempt)),
        None => (input, None),
    };
    Ok((
        Uuid::parse_str(device)
            .map_err(OperationError::at("parse_pairing_device_id"))
            .map_err(activity_failure)?,
        attempt
            .map(Uuid::parse_str)
            .transpose()
            .map_err(OperationError::at("parse_public_pairing_attempt_id"))
            .map_err(activity_failure)?,
    ))
}

pub struct PairingActivities {
    devices: PairingDeviceRepository,
    attempts: PairingAttemptRepository,
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
        self.attempts
            .mark_failed(attempt)
            .await
            .map_err(|error| activity_failure(error.into()))?;
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
            if let Some(attempt_id) = public_attempt_id {
                self.attempts
                    .mark_worker_started(attempt_id, device_id)
                    .await?;
            }
            let attempt_expiry = self
                .attempts
                .require_attempt(device_id, public_attempt_id)
                .await?;
            let DevicePeer {
                wireguard_public_key: public_key,
                expires_at,
            } = self
                .devices
                .active_peer(device_id)
                .await?
                .ok_or_else(|| OperationError::without_source("pairing_device_unavailable"))?;
            let public_key: [u8; 32] = public_key
                .try_into()
                .map_err(|_| OperationError::without_source("invalid_pairing_wireguard_key"))?;
            tracing::info!("Waiting for device to establish its WireGuard connection");
            let mut last_error = None;
            loop {
                if Utc::now() >= attempt_expiry.unwrap_or(expires_at).min(expires_at) {
                    return Err(OperationError::without_source("pairing_device_expired"));
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
            let attempt_expiry = self
                .attempts
                .require_attempt(device_id, public_attempt_id)
                .await?;
            // Resolve the address at execution time, never from a queued IP payload.
            let PairingTarget {
                address,
                expires_at,
                attempt_started_at,
                wireguard_public_key: public_key,
            } = self
                .devices
                .pairing_target(device_id)
                .await?
                .ok_or_else(|| OperationError::without_source("pairing_device_unavailable"))?;
            let peer = address
                .parse::<Ipv4Addr>()
                .map_err(OperationError::at("parse_pairing_device_address"))?;
            let status = self
                .agent
                .status()
                .await
                .map_err(OperationError::at("read_pairing_vpn_status"))?;
            let subnet = Ipv4Net::new(status.address, status.prefix_len)
                .map_err(OperationError::at("read_pairing_vpn_subnet"))?;
            if status.state != VpnState::Running
                || !subnet.contains(&peer)
                || peer == status.address
                || peer == subnet.network()
                || peer == subnet.broadcast()
            {
                return Err(OperationError::without_source("pairing_device_outside_vpn"));
            }
            let remaining = (attempt_expiry.unwrap_or(expires_at).min(expires_at) - Utc::now())
                .to_std()
                .map_err(OperationError::at("pairing_lease_expired"))?;
            let lease_deadline = Instant::now() + remaining;
            // Recheck at activity execution time in case it was queued after the
            // readiness activity completed. Do not create a session for a stale peer.
            let public_key: [u8; 32] = public_key
                .try_into()
                .map_err(|_| OperationError::without_source("invalid_pairing_wireguard_key"))?;
            self.agent
                .require_connected_peer(public_key)
                .await
                .map_err(OperationError::at("check_pairing_vpn_peer"))?;
            let session = SetupSession::new(&status, peer)
                .await
                .map_err(OperationError::at("open_pairing_session"))?;
            let code_expiry = Utc::now()
                + chrono::Duration::from_std(
                    deadline
                        .min(lease_deadline)
                        .saturating_duration_since(Instant::now()),
                )
                .map_err(OperationError::at("compute_pairing_code_expiry"))?;
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
                peer,
                lease_deadline,
                expires_at,
                attempt_started_at,
                attempt_id,
            ))
        };
        let (mut session, peer, lease_deadline, expires_at, attempt_started_at, attempt_id) =
            wait(&ctx, prepare, deadline, "pairing session expired").await?;
        let deadline = deadline.min(lease_deadline);
        let operation = async {
            let codes = &self.codes;
            let record = session
                .run(move |pin| async move {
                    match pin {
                        Some(pin) => codes
                            .publish(attempt_id, device_id, pin)
                            .await
                            .map_err(Into::into),
                        None => codes.clear(attempt_id).await.map_err(Into::into),
                    }
                })
                .await?;
            let verification_deadline = (Instant::now() + VERIFICATION_LIMIT).min(lease_deadline);
            let verification_expires_at = Utc::now()
                + chrono::Duration::from_std(
                    verification_deadline.saturating_duration_since(Instant::now()),
                )
                .map_err(OperationError::at("compute_pairing_verification_expiry"))?;
            // Persist inside the activity: credentials must not enter Temporal history.
            self.pairings
                .save_pending(
                    device_id,
                    attempt_id,
                    expires_at,
                    attempt_started_at,
                    verification_expires_at,
                    &record,
                )
                .await?;
            Ok((record, verification_deadline))
        };
        let saved = wait(&ctx, operation, deadline, "pairing session expired").await;
        // Withdraw the proxy service before waiting for the extension's report.
        // A crash still expires its last announcement after the ten-second DNS TTL.
        session.stop().await;
        drop(session);
        let result = match saved {
            Ok((mut record, verification_deadline)) => {
                tracing::info!(
                    "Encrypted pairing saved; waiting for the VPN extension's port report"
                );
                let verify = async {
                    loop {
                        if let Some(port) = self
                            .pairings
                            .reported_port(device_id, expires_at, attempt_started_at)
                            .await?
                        {
                            let port = u16::try_from(port)
                                .map_err(OperationError::at("target_invalid_port"))?;
                            tracing::info!("Remote pairing port reported; verifying saved pairing");
                            tokio::time::timeout(
                                Duration::from_secs(20),
                                verify_saved_pairing(
                                    peer,
                                    port,
                                    &mut record.pairing_file,
                                    &record.peer_public_key,
                                ),
                            )
                            .await
                            .map_err(OperationError::at("pairing-preflight-timeout"))??;
                            if self
                                .pairings
                                .complete_verified_pairing(
                                    device_id,
                                    (attempt_id, public_attempt_id),
                                    expires_at,
                                    attempt_started_at,
                                    port,
                                    &record,
                                )
                                .await?
                            {
                                tracing::info!("Remote pairing verified");
                                return Ok(());
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                };
                wait(
                    &ctx,
                    verify,
                    verification_deadline,
                    "Remote pairing was not verified before the deadline",
                )
                .await
            }
            Err(error) => Err(error),
        };
        if result.is_ok() {
            return result;
        }
        // Best effort on cancellation as well; expiry also handles process crashes.
        let ended =
            match tokio::time::timeout(Duration::from_secs(2), self.codes.end(attempt_id)).await {
                Ok(result) => result.map_err(Into::into),
                Err(error) => Err(OperationError::at("end_pairing_attempt_timeout")(error)),
            };
        if let Err(error) = ended {
            report_failure(
                "Could not invalidate pairing code; it will expire at the session deadline",
                &error,
            );
        }
        result
    }
}

// A committed claim survives HTTP cancellation and Temporal outages. Workflow IDs
// deduplicate uncertain enqueue results, including already completed workflows.
async fn dispatch_attempts(attempts: PairingAttemptRepository) {
    let mut was_unavailable = false;
    loop {
        let result = async {
            attempts.expire_overdue().await?;
            let pending = attempts.pending_dispatch().await?;
            if pending.is_empty() {
                return Ok(());
            }
            let client = client()
                .await
                .map_err(OperationError::at("connect_temporal"))?;
            let mut enqueue_error = None;
            for PendingPairingAttempt {
                id: attempt,
                device_id: device,
            } in pending
            {
                let started = client
                    .start_workflow(
                        PairAttemptWorkflow::run,
                        format!("{device}/{attempt}"),
                        WorkflowStartOptions::new(TASK_QUEUE, format!("pair-attempt-{attempt}"))
                            .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                            .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
                            .build(),
                    )
                    .await;
                if let Err(error) = started {
                    enqueue_error.get_or_insert_with(|| {
                        OperationError::at("enqueue_pairing_attempt")(error)
                    });
                    continue;
                }
                attempts.mark_queued(attempt).await?;
            }
            enqueue_error.map_or(Ok(()), Err)
        }
        .await;
        if let Err(error) = &result
            && !was_unavailable
        {
            report_failure(
                "Pairing attempt dispatch unavailable; durable claims will be retried",
                error,
            );
        }
        was_unavailable = result.is_err();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub(crate) fn report_failure(message: &'static str, error: &OperationError) {
    let operation = error.operation;
    let cause = error.cause_code();
    tracing::warn!(operation, cause, "{message}");
    telemetry::report_operation(message, operation, cause);
}

pub(crate) fn activity_failure(error: OperationError) -> ActivityError {
    report_failure("Activity failed", &error);
    // Only the fixed operation code belongs in Temporal history.
    io::Error::other(error.operation).into()
}

/// Heartbeat even during database/agent setup, and drop network futures on cancel.
async fn wait<T>(
    ctx: &ActivityContext,
    operation: impl Future<Output = Result<T, OperationError>>,
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
            result = &mut operation => return result.map_err(activity_failure),
            _ = heartbeats.tick() => ctx.record_heartbeat(()).await
                .map_err(OperationError::at("record_pairing_heartbeat"))
                .map_err(activity_failure)?,
        }
    }
}

pub async fn run_worker() -> Result<(), OperationError> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(OperationError::at("register_sigint"))?;
    let mut terminate =
        signal(SignalKind::terminate()).map_err(OperationError::at("register_sigterm"))?;
    let database_url =
        std::env::var("DATABASE_URL").map_err(OperationError::at("read_database_url"))?;
    let pool = db::connect(&database_url)
        .await
        .map_err(OperationError::at("connect_database"))?;
    let pairings = PairingStore::from_env(pool.clone())?;
    let codes = PairingCodeRepository::from_env(pool.clone())?;
    db::migrate(&pool)
        .await
        .map_err(OperationError::at("migrate_database"))?;
    let devices = PairingDeviceRepository::new(pool.clone());
    let attempts = PairingAttemptRepository::new(pool);
    let agent = Client::from_env().map_err(OperationError::at("configure_vpn_agent"))?;
    let runtime = Runtime::from_current_tokio(Default::default())
        .map_err(OperationError::at("build_temporal_runtime"))?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(20))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<PairDeviceWorkflow>()
        .map_err(OperationError::at("register_pair_device_workflow"))?
        .register_workflow::<PairAttemptWorkflow>()
        .map_err(OperationError::at("register_pair_attempt_workflow"))?
        .register_activities(PairingActivities {
            devices,
            attempts: attempts.clone(),
            agent,
            pairings,
            codes,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(1))
        .build();
    let temporal = client()
        .await
        .map_err(OperationError::at("connect_temporal"))?;
    let worker = Worker::new(&runtime, temporal, options)
        .map_err(OperationError::at("build_pairing_worker"))?;
    tracing::info!(task_queue = TASK_QUEUE, "Pairing worker listening");
    let dispatcher = tokio::spawn(dispatch_attempts(attempts));
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
    result.map_err(OperationError::at("run_pairing_worker"))
}
