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
    db,
    pairing::Session,
};

const TASK_QUEUE: &str = "apprelay-pairing";
const SESSION_LIMIT: Duration = Duration::from_secs(60);

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn client() -> Result<TemporalClient, Error> {
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
}

#[activities]
impl PairingActivities {
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
            let (address, expires_at) = sqlx::query_as::<_, (String, DateTime<Utc>)>(
                "SELECT host(address), expires_at FROM devices
                 WHERE id = $1 AND address IS NOT NULL AND expires_at > statement_timestamp()",
            )
            .bind(device_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("device is missing, expired, or has no VPN address"))?;
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
            let session = Session::new(&status, peer).await?;
            eprintln!("Starting pairing activity for device {device_id} at {peer}");
            Ok((session, deadline.min(lease_deadline)))
        };
        let (mut session, deadline) = wait(&ctx, prepare, deadline).await?;
        let result = wait(&ctx, session.run(), deadline).await;
        // Normal completion, cancellation, and timeout all withdraw the proxy service.
        // A crash still expires its last announcement after the ten-second DNS TTL.
        session.stop().await;
        result
    }
}

/// Heartbeat even during database/agent setup, and drop network futures on cancel.
async fn wait<T>(
    ctx: &ActivityContext,
    operation: impl Future<Output = io::Result<T>>,
    deadline: Instant,
) -> Result<T, ActivityError> {
    let mut heartbeats = interval(Duration::from_secs(2));
    heartbeats.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            _ = ctx.cancelled() => return Err(ActivityError::cancelled()),
            _ = sleep_until(deadline) => return Err(io::Error::new(
                io::ErrorKind::TimedOut, "pairing session expired",
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
        .register_activities(PairingActivities { pool, agent })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(1))
        .build();
    let worker = Worker::new(&runtime, client().await?, options)?;
    eprintln!(
        "Pairing worker listening on {TASK_QUEUE}: at most 10 active sessions, 60 seconds each"
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
