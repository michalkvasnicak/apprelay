use std::{io, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use temporalio_client::{WorkflowIdConflictPolicy, WorkflowIdReusePolicy, WorkflowStartOptions};
use temporalio_common::RetryPolicy;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, Runtime, Worker, WorkerOptions, WorkflowContext, WorkflowResult,
    activities::{ActivityContext, ActivityError},
    runtime::worker_tuner::{FixedSizeSlotSupplier, TunerHolder},
};
use uuid::Uuid;

use crate::{
    apns::{ApnsClient, SendOutcome},
    db::{
        self,
        install_jobs::object_key,
        pairing_codes::{Notification, NotificationContent},
        pairings::PairingStore,
    },
    http::ArtifactApi,
    install::{Application, deliver},
    r2::R2Storage,
    temporal_pairing::client,
};

const TASK_QUEUE: &str = "apprelay-install";

#[workflow]
#[derive(Default)]
pub struct InstallWorkflow;

#[derive(Clone, Deserialize, Serialize)]
pub struct InstallNotice {
    id: Uuid,
    outcome: Option<bool>,
}

#[workflow_methods]
impl InstallWorkflow {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, id: Uuid) -> WorkflowResult<()> {
        let _ = ctx
            .execute_activity(
                InstallActivities::notify,
                InstallNotice { id, outcome: None },
                notice_options(),
            )
            .await;
        let installed = ctx
            .execute_activity(
                InstallActivities::install,
                id,
                ActivityOptions::with_schedule_to_close_timeout(Duration::from_secs(660))
                    .heartbeat_timeout(Duration::from_secs(20))
                    // Once InstallationProxy has acted, a lost response is ambiguous.
                    .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                    .build(),
            )
            .await;
        let terminal = InstallNotice {
            id,
            outcome: Some(installed.is_ok()),
        };
        ctx.execute_activity(
            InstallActivities::record_outcome,
            terminal.clone(),
            notice_options(),
        )
        .await?;
        let _ = ctx
            .execute_activity(InstallActivities::notify, terminal, notice_options())
            .await;
        installed?;
        Ok(())
    }
}

fn notice_options() -> ActivityOptions {
    ActivityOptions::with_schedule_to_close_timeout(Duration::from_secs(30))
        .retry_policy(RetryPolicy::builder().maximum_attempts(3).build())
        .build()
}

pub struct InstallActivities {
    pool: PgPool,
    storage: R2Storage,
    pairings: PairingStore,
    apns: Option<ApnsClient>,
}

fn database(_: sqlx::Error) -> io::Error {
    io::Error::other("install job storage unavailable")
}

#[derive(sqlx::FromRow)]
struct PushTarget {
    device_id: Uuid,
    token: Vec<u8>,
    environment: String,
    token_updated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

#[activities]
impl InstallActivities {
    #[activity]
    pub async fn install(
        self: Arc<Self>,
        ctx: ActivityContext,
        id: Uuid,
    ) -> Result<(), ActivityError> {
        let operation = async {
            let (size, checksum) = sqlx::query_as::<_, (i64, String)>(
                "SELECT size_bytes, sha256 FROM install_jobs WHERE id = $1 AND queued_at IS NOT NULL
                 AND finished_at IS NULL AND expires_at > clock_timestamp()",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(database)?
            .ok_or_else(|| io::Error::other("install job expired"))?;
            // Check that the authorized pairing is still usable before downloading.
            self.pairings.install_target(id).await?;
            let bytes = tokio::time::timeout(
                Duration::from_secs(60),
                self.storage.read_signed(&object_key(id), size, &checksum),
            )
            .await
            .map_err(|_| io::Error::other("signed IPA download timed out"))?
            .map_err(|_| io::Error::other("signed IPA missing or failed integrity check"))?;
            let app = tokio::task::spawn_blocking(move || Application::unpack(bytes))
                .await
                .map_err(|_| io::Error::other("IPA staging interrupted"))?
                .map_err(io::Error::other)?;
            // Refresh the leased endpoint and confirm no re-pairing during preparation.
            let target = self.pairings.install_target(id).await?;
            let remaining = (target.expires_at - Utc::now())
                .to_std()
                .map_err(|_| io::Error::other("install authorization expired"))?;
            tokio::time::timeout(
                remaining.min(Duration::from_secs(540)),
                deliver(id, target, app),
            )
            .await
            .map_err(|_| io::Error::other("phone installation timed out"))?
            .map_err(io::Error::other)
        };
        tokio::pin!(operation);
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = ctx.cancelled() => return Err(ActivityError::cancelled()),
                _ = ticks.tick() => ctx.record_heartbeat(()).await?,
                result = &mut operation => return result.map_err(Into::into),
            }
        }
    }

    #[activity]
    pub async fn record_outcome(
        self: Arc<Self>,
        _ctx: ActivityContext,
        notice: InstallNotice,
    ) -> Result<(), ActivityError> {
        let Some(installed) = notice.outcome else {
            return Err(io::Error::other("missing install outcome").into());
        };
        sqlx::query(
            "UPDATE install_jobs SET outcome = COALESCE(outcome, $2),
                 finished_at = COALESCE(finished_at, clock_timestamp())
             WHERE id = $1 AND queued_at IS NOT NULL",
        )
        .bind(notice.id)
        .bind(if installed { "installed" } else { "failed" })
        .execute(&self.pool)
        .await
        .map_err(database)?;
        Ok(())
    }

    #[activity]
    pub async fn notify(
        self: Arc<Self>,
        _ctx: ActivityContext,
        notice: InstallNotice,
    ) -> Result<(), ActivityError> {
        let state = match notice.outcome {
            Some(true) => "installed",
            Some(false) => "failed",
            None => "starting",
        };
        let Some(apns) = &self.apns else {
            return Ok(());
        };
        let Some(target) = sqlx::query_as::<_, PushTarget>(
            "SELECT d.id AS device_id, t.token, t.environment, t.updated_at AS token_updated_at,
                 LEAST(j.expires_at, d.expires_at, clock_timestamp() + INTERVAL '5 minutes') AS expires_at
             FROM install_jobs j
             JOIN devices d ON d.id = j.device_id AND d.expires_at = j.device_expires_at
             JOIN device_pairings p ON p.device_id = d.id AND p.attempt_started_at = j.pairing_started_at
             JOIN device_push_tokens t ON t.device_id = d.id
             WHERE j.id = $1 AND j.queued_at IS NOT NULL
               AND j.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND (($2 = 'starting' AND j.finished_at IS NULL) OR j.outcome = $2)",
        ).bind(notice.id).bind(state).fetch_optional(&self.pool).await.map_err(database)?
        else { return Ok(()); };
        let notification = Notification {
            content: NotificationContent::Install {
                job_id: notice.id,
                outcome: notice.outcome,
            },
            expires_at: target.expires_at,
            device_id: target.device_id,
            token: target.token,
            environment: target.environment,
            token_updated_at: target.token_updated_at,
        };
        // Deterministic for APNs retries, distinct for start and terminal events.
        let push_id =
            Uuid::from_u128(notice.id.as_u128() ^ if notice.outcome.is_some() { 1 } else { 0 });
        match apns.send(push_id, &notification).await? {
            SendOutcome::Accepted | SendOutcome::Expired => {}
            SendOutcome::Rejected => tracing::warn!("APNs rejected install notification"),
            SendOutcome::InvalidToken(invalidated_at) => {
                if invalidated_at
                    .is_none_or(|time| notification.token_updated_at.timestamp_millis() <= time)
                {
                    sqlx::query("DELETE FROM device_push_tokens
                        WHERE device_id = $1 AND updated_at = $2 AND token = $3 AND environment = $4")
                        .bind(notification.device_id).bind(notification.token_updated_at)
                        .bind(&notification.token).bind(&notification.environment)
                        .execute(&self.pool).await.map_err(database)?;
                }
            }
        }
        Ok(())
    }
}

async fn dispatch(pool: PgPool, temporal: temporalio_client::Client) {
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut unavailable = false;
    loop {
        ticks.tick().await;
        let result = async {
            let pending = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM install_jobs WHERE queued_at IS NOT NULL AND dispatched_at IS NULL
                   AND finished_at IS NULL AND expires_at > clock_timestamp() ORDER BY queued_at LIMIT 20",
            ).fetch_all(&pool).await.map_err(database)?;
            let mut failed = false;
            for id in pending {
                let start = temporal.start_workflow(
                    InstallWorkflow::run,
                    id,
                    WorkflowStartOptions::new(TASK_QUEUE, format!("install-job-{id}"))
                        .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
                        .build(),
                );
                if !matches!(tokio::time::timeout(Duration::from_secs(2), start).await, Ok(Ok(_))) {
                    failed = true;
                    continue;
                }
                sqlx::query("UPDATE install_jobs SET dispatched_at = clock_timestamp() WHERE id = $1")
                    .bind(id).execute(&pool).await.map_err(database)?;
            }
            Ok::<_, io::Error>(failed)
        }.await;
        let failed = !matches!(result, Ok(false));
        if failed && !unavailable {
            crate::telemetry::report("Install workflow dispatch unavailable");
            tracing::warn!("Install workflow dispatch unavailable; queued jobs will be retried");
        }
        unavailable = failed;
    }
}

pub async fn run_worker() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
    db::MIGRATOR.run(&pool).await?;
    let storage = ArtifactApi::from_env(pool.clone())?
        .ok_or_else(|| io::Error::other("install worker requires R2 storage configuration"))?
        .storage;
    let pairings = PairingStore::from_env(pool.clone())?;
    let apns = std::env::var_os("APPRELAY_APNS_SIGNING_KEY_FILE")
        .map(|_| ApnsClient::from_env())
        .transpose()?;
    let temporal = client().await?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(4))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<InstallWorkflow>()?
        .register_activities(InstallActivities {
            pool: pool.clone(),
            storage,
            pairings,
            apns,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(10))
        .build();
    let worker = Worker::new(&runtime, temporal.clone(), options)?;
    let dispatch = tokio::spawn(dispatch(pool, temporal));
    tracing::info!(task_queue = TASK_QUEUE, "Install worker listening");
    let result = worker
        .run_until(async {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await;
    dispatch.abort();
    let _ = dispatch.await;
    result?;
    Ok(())
}
