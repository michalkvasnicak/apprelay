use std::{io, sync::Arc, time::Duration};

use temporalio_client::{WorkflowIdConflictPolicy, WorkflowIdReusePolicy, WorkflowStartOptions};
use temporalio_common::{RetryPolicy, error::ApplicationFailure};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, Runtime, Worker, WorkerOptions, WorkflowContext, WorkflowResult,
    activities::{ActivityContext, ActivityError},
    runtime::worker_tuner::{FixedSizeSlotSupplier, TunerHolder},
};
use uuid::Uuid;

use crate::{
    apns::{ApnsClient, SendOutcome},
    db::{self, pairing_codes::PairingCodeRepository},
    error::OperationError,
    temporal_pairing::{activity_failure, client, report_failure},
};

const TASK_QUEUE: &str = "apprelay-apns";

#[workflow]
#[derive(Default)]
pub struct PairingNotificationWorkflow;

#[workflow_methods]
impl PairingNotificationWorkflow {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        notification_id: String,
    ) -> WorkflowResult<()> {
        ctx.execute_activity(
            NotificationActivities::send,
            notification_id,
            ActivityOptions::with_schedule_to_close_timeout(Duration::from_secs(60))
                .retry_policy(
                    RetryPolicy::builder()
                        .initial_interval(Duration::from_secs(1))
                        .maximum_interval(Duration::from_secs(4))
                        .maximum_attempts(5)
                        .build(),
                )
                .build(),
        )
        .await?;
        Ok(())
    }
}

pub struct NotificationActivities {
    codes: PairingCodeRepository,
    apns: ApnsClient,
}

#[activities]
impl NotificationActivities {
    #[activity]
    pub async fn send(
        self: Arc<Self>,
        ctx: ActivityContext,
        notification_id: String,
    ) -> Result<(), ActivityError> {
        let notification_id = Uuid::parse_str(&notification_id)
            .map_err(OperationError::at("parse_pairing_notification_id"))
            .map_err(activity_failure)?;
        let operation = async {
            // All credentials and the PIN stay in this activity, never in history.
            let Some(notification) = self
                .codes
                .notification(notification_id)
                .await
                .map_err(activity_failure)?
            else {
                return Ok(());
            };
            match self
                .apns
                .send(notification_id, &notification)
                .await
                .map_err(OperationError::at("send_pairing_notification"))
                .map_err(activity_failure)?
            {
                SendOutcome::Accepted => self
                    .codes
                    .mark_sent(notification_id)
                    .await
                    .map_err(activity_failure)?,
                SendOutcome::Expired => {}
                SendOutcome::InvalidToken(invalidated_at) => {
                    if invalidated_at
                        .is_none_or(|time| notification.token_updated_at.timestamp_millis() <= time)
                    {
                        self.codes
                            .remove_invalid_token(&notification)
                            .await
                            .map_err(activity_failure)?;
                    }
                    return Err(ActivityError::application(
                        ApplicationFailure::non_retryable(io::Error::other(
                            "APNs device token is invalid",
                        )),
                    ));
                }
                SendOutcome::Rejected => {
                    return Err(ActivityError::application(
                        ApplicationFailure::non_retryable(io::Error::other(
                            "APNs rejected pairing notification",
                        )),
                    ));
                }
            }
            Ok(())
        };
        tokio::select! {
            _ = ctx.cancelled() => Err(ActivityError::cancelled()),
            result = tokio::time::timeout(Duration::from_secs(8), operation) => {
                result.map_err(OperationError::at("send_pairing_notification_timeout"))
                    .map_err(activity_failure)?
            }
        }
    }
}

async fn dispatch(codes: PairingCodeRepository, temporal: temporalio_client::Client) {
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut was_unavailable = false;
    loop {
        ticks.tick().await;
        let mut dispatch_error = None;
        let pending = match codes.pending_notifications().await {
            Ok(pending) => pending,
            Err(error) => {
                if !was_unavailable {
                    report_failure(
                        "Pairing notification dispatch unavailable; will retry",
                        &error,
                    );
                }
                was_unavailable = true;
                continue;
            }
        };
        for notification_id in pending {
            let start = temporal.start_workflow(
                PairingNotificationWorkflow::run,
                notification_id.to_string(),
                WorkflowStartOptions::new(
                    TASK_QUEUE,
                    format!("pairing-notification-{notification_id}"),
                )
                .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
                .id_reuse_policy(WorkflowIdReusePolicy::AllowDuplicate)
                .build(),
            );
            let result = match tokio::time::timeout(Duration::from_secs(2), start).await {
                Ok(Ok(_)) => codes.mark_queued(notification_id).await,
                Ok(Err(error)) => Err(OperationError::at("enqueue_pairing_notification")(error)),
                Err(error) => Err(OperationError::at("enqueue_pairing_notification_timeout")(
                    error,
                )),
            };
            if let Err(error) = result {
                dispatch_error.get_or_insert(error);
            }
        }
        if let Some(error) = &dispatch_error
            && !was_unavailable
        {
            report_failure(
                "Pairing notification dispatch unavailable; will retry",
                error,
            );
        }
        was_unavailable = dispatch_error.is_some();
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
    let codes = PairingCodeRepository::from_env(pool.clone())
        .map_err(OperationError::at("configure_pairing_code_repository"))?;
    let apns = ApnsClient::from_env().map_err(OperationError::at("configure_apns"))?;
    db::migrate(&pool)
        .await
        .map_err(OperationError::at("migrate_database"))?;
    let temporal = client()
        .await
        .map_err(OperationError::at("connect_temporal"))?;
    let runtime = Runtime::from_current_tokio(Default::default())
        .map_err(OperationError::at("build_temporal_runtime"))?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<PairingNotificationWorkflow>()
        .map_err(OperationError::at("register_pairing_notification_workflow"))?
        .register_activities(NotificationActivities {
            codes: codes.clone(),
            apns,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(2))
        .build();
    let worker = Worker::new(&runtime, temporal.clone(), options)
        .map_err(OperationError::at("build_notification_worker"))?;
    let dispatch = tokio::spawn(dispatch(codes, temporal));
    tracing::info!(task_queue = TASK_QUEUE, "Notification worker listening");
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
    result.map_err(OperationError::at("run_notification_worker"))
}
