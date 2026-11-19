use std::{io, sync::Arc, time::Duration};

use chrono::Utc;
use serde::{Deserialize, Serialize};
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
        install_jobs::{InstallJobRepository, object_key},
        pairings::PairingStore,
    },
    error::OperationError,
    http::ArtifactApi,
    iphone::{
        install::{UnpackedIpa, deliver},
        pairing::verify_saved_pairing,
        session::IphoneSession,
    },
    r2::R2Storage,
    telemetry,
    temporal_pairing::{activity_failure, client, report_failure},
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
        let installed: WorkflowResult<()> = async {
            // Keep the old activity sequence when replaying existing histories.
            if !ctx.patched("install-verify-on-delivery-connection") {
                ctx.execute_activity(
                    InstallActivities::verify_pairing,
                    id,
                    ActivityOptions::with_schedule_to_close_timeout(Duration::from_secs(30))
                        .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                        .build(),
                )
                .await?;
            }
            let _ = ctx
                .execute_activity(
                    InstallActivities::notify,
                    InstallNotice { id, outcome: None },
                    notice_options(),
                )
                .await;
            ctx.execute_activity(
                InstallActivities::install,
                id,
                ActivityOptions::with_schedule_to_close_timeout(Duration::from_secs(660))
                    .heartbeat_timeout(Duration::from_secs(20))
                    // Once InstallationProxy has acted, a lost response is ambiguous.
                    .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                    .build(),
            )
            .await?;
            Ok(())
        }
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
    jobs: InstallJobRepository,
    storage: R2Storage,
    pairings: PairingStore,
    apns: Option<ApnsClient>,
}

#[activities]
impl InstallActivities {
    // Retained for install workflows started before the verification patch.
    #[activity]
    pub async fn verify_pairing(
        self: Arc<Self>,
        _ctx: ActivityContext,
        id: Uuid,
    ) -> Result<(), ActivityError> {
        let operation = async {
            let mut target = self.pairings.install_target(id).await?;
            let remaining = (target.expires_at - Utc::now())
                .to_std()
                .map_err(OperationError::at("install_authorization_expired"))?;
            // Verify the saved credentials and peer identity without creating a tunnel.
            tokio::time::timeout(
                remaining.min(Duration::from_secs(20)),
                verify_saved_pairing(
                    target.address,
                    target.port,
                    &mut target.record,
                    &target.peer_key,
                ),
            )
            .await
            .map_err(OperationError::at("pairing-preflight-timeout"))??;
            Ok(())
        };
        operation.await.map_err(activity_failure)
    }

    #[activity]
    pub async fn install(
        self: Arc<Self>,
        ctx: ActivityContext,
        id: Uuid,
    ) -> Result<(), ActivityError> {
        tracing::info!(job_id = %id, "Install activity started");
        let operation = async {
            let artifact = self
                .jobs
                .load_installable(id)
                .await?
                .ok_or_else(|| OperationError::without_source("install_job_expired"))?;
            // Check that the authorized pairing is still usable before downloading.
            self.pairings.install_target(id).await?;
            let bytes = tokio::time::timeout(
                Duration::from_secs(60),
                self.storage
                    .read_signed(&object_key(id), artifact.size_bytes, &artifact.sha256),
            )
            .await
            .map_err(OperationError::at("download_signed_ipa_timeout"))??;
            let app = tokio::task::spawn_blocking(move || UnpackedIpa::unpack(bytes))
                .await
                .map_err(OperationError::at("stage_ipa_task"))??;
            // Refresh the lease after staging, then verify the connection delivery will use.
            let mut target = self.pairings.install_target(id).await?;
            let remaining = (target.expires_at - Utc::now())
                .to_std()
                .map_err(OperationError::at("install_authorization_expired"))?;
            tokio::time::timeout(remaining.min(Duration::from_secs(540)), async {
                let mut phone = IphoneSession::connect(
                    target.address,
                    target.port,
                    &mut target.record,
                    &target.peer_key,
                )
                .await?;
                deliver(id, &mut phone, app, target.expires_at).await
            })
            .await
            .map_err(OperationError::at("phone_install_timeout"))?
        };
        tokio::pin!(operation);
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = ctx.cancelled() => {
                    tracing::warn!(job_id = %id, "Install activity cancelled");
                    return Err(ActivityError::cancelled());
                },
                _ = ticks.tick() => ctx.record_heartbeat(()).await.inspect_err(|_| {
                    tracing::error!(job_id = %id, "Install activity heartbeat failed");
                    telemetry::report("Install activity heartbeat failed");
                })?,
                result = &mut operation => return result.map_err(|error| {
                    let operation = error.operation;
                    let cause = error.cause_code();
                    tracing::error!(job_id = %id, operation, cause, "Phone installation failed");
                    telemetry::report_operation("Phone installation failed", operation, cause);
                    // Temporal persists failure details; don't serialize the source chain.
                    io::Error::other(operation).into()
                }),
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
        let outcome = if installed { "installed" } else { "failed" };
        self.jobs
            .record_outcome(notice.id, installed)
            .await
            .map_err(|error| activity_failure(error.into()))?;
        if installed {
            tracing::info!(job_id = %notice.id, outcome, "Install finished");
        } else {
            tracing::warn!(job_id = %notice.id, outcome, "Install finished");
        }
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
        let Some(notification) = self
            .jobs
            .notification(notice.id, notice.outcome)
            .await
            .map_err(|error| activity_failure(error.into()))?
        else {
            return Ok(());
        };
        // Deterministic for APNs retries, distinct for start and terminal events.
        let push_id =
            Uuid::from_u128(notice.id.as_u128() ^ if notice.outcome.is_some() { 1 } else { 0 });
        match apns.send(push_id, &notification).await.inspect_err(|_| {
            tracing::error!(job_id = %notice.id, state, "Install notification delivery failed");
            telemetry::report("Install notification delivery failed");
        })? {
            SendOutcome::Accepted | SendOutcome::Expired => {}
            SendOutcome::Rejected => {
                tracing::warn!(job_id = %notice.id, state, "APNs rejected install notification");
            }
            SendOutcome::InvalidToken(invalidated_at) => {
                if invalidated_at
                    .is_none_or(|time| notification.token_updated_at.timestamp_millis() <= time)
                {
                    self.jobs
                        .remove_invalid_token(&notification)
                        .await
                        .map_err(|error| activity_failure(error.into()))?;
                }
            }
        }
        Ok(())
    }
}

async fn dispatch(jobs: InstallJobRepository, temporal: temporalio_client::Client) {
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut unavailable = false;
    loop {
        ticks.tick().await;
        let result = async {
            let pending = jobs.pending_dispatch().await?;
            let mut enqueue_error = None;
            for id in pending {
                let start = temporal.start_workflow(
                    InstallWorkflow::run,
                    id,
                    WorkflowStartOptions::new(TASK_QUEUE, format!("install-job-{id}"))
                        .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
                        .build(),
                );
                let error = match tokio::time::timeout(Duration::from_secs(2), start).await {
                    Ok(Ok(_)) => {
                        jobs.mark_dispatched(id).await?;
                        continue;
                    }
                    Ok(Err(error)) => OperationError::at("enqueue_install_job")(error),
                    Err(error) => OperationError::at("enqueue_install_job_timeout")(error),
                };
                enqueue_error.get_or_insert(error);
            }
            enqueue_error.map_or(Ok(()), Err)
        }
        .await;
        if let Err(error) = &result
            && !unavailable
        {
            report_failure(
                "Install workflow dispatch unavailable; queued jobs will be retried",
                error,
            );
        }
        unavailable = result.is_err();
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
    db::migrate(&pool)
        .await
        .map_err(OperationError::at("migrate_database"))?;
    let storage = ArtifactApi::from_env(pool.clone())
        .map_err(OperationError::at("configure_artifact_api"))?
        .ok_or_else(|| OperationError::without_source("missing_r2_configuration"))?
        .storage;
    let pairings = PairingStore::from_env(pool.clone())?;
    let jobs = InstallJobRepository::new(pool);
    let apns = std::env::var_os("APPRELAY_APNS_SIGNING_KEY_FILE")
        .map(|_| ApnsClient::from_env())
        .transpose()
        .map_err(OperationError::at("configure_apns"))?;
    let temporal = client()
        .await
        .map_err(OperationError::at("connect_temporal"))?;
    let runtime = Runtime::from_current_tokio(Default::default())
        .map_err(OperationError::at("build_temporal_runtime"))?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(4))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(5))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<InstallWorkflow>()
        .map_err(OperationError::at("register_install_workflow"))?
        .register_activities(InstallActivities {
            jobs: jobs.clone(),
            storage,
            pairings,
            apns,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(10))
        .build();
    let worker = Worker::new(&runtime, temporal.clone(), options)
        .map_err(OperationError::at("build_install_worker"))?;
    let dispatch = tokio::spawn(dispatch(jobs, temporal));
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
    result.map_err(OperationError::at("run_install_worker"))
}
