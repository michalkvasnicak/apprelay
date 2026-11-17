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
    temporal_pairing::client,
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
        let notification_id = Uuid::parse_str(&notification_id)?;
        let operation = async {
            // All credentials and the PIN stay in this activity, never in history.
            let Some(notification) = self
                .codes
                .notification(notification_id)
                .await
                .inspect_err(|_| crate::telemetry::report("Could not read pairing notification"))?
            else {
                return Ok(());
            };
            match self
                .apns
                .send(notification_id, &notification)
                .await
                .inspect_err(|_| crate::telemetry::report("APNs notification delivery failed"))?
            {
                SendOutcome::Accepted => {
                    self.codes
                        .mark_sent(notification_id)
                        .await
                        .inspect_err(|_| {
                            crate::telemetry::report("Could not mark pairing notification sent");
                        })?
                }
                SendOutcome::Expired => {}
                SendOutcome::InvalidToken(invalidated_at) => {
                    if invalidated_at
                        .is_none_or(|time| notification.token_updated_at.timestamp_millis() <= time)
                    {
                        self.codes
                            .remove_invalid_token(&notification)
                            .await
                            .inspect_err(|_| {
                                crate::telemetry::report("Could not remove invalid APNs token");
                            })?;
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
                result.map_err(|_| {
                    crate::telemetry::report("APNs notification attempt timed out");
                    io::Error::other("APNs notification attempt timed out")
                })?
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
        let mut unavailable = false;
        let pending = match codes.pending_notifications().await {
            Ok(pending) => pending,
            Err(_) => {
                eprintln!("Could not read pairing notification outbox; will retry");
                if !was_unavailable {
                    crate::telemetry::report("Pairing notification dispatch unavailable");
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
            match tokio::time::timeout(Duration::from_secs(2), start).await {
                Ok(Ok(_)) => {
                    if codes.mark_queued(notification_id).await.is_err() {
                        unavailable = true;
                        eprintln!("Could not mark pairing notification queued; will retry");
                    }
                }
                _ => {
                    unavailable = true;
                    eprintln!("Could not queue pairing notification; will retry while active");
                }
            }
        }
        if unavailable && !was_unavailable {
            crate::telemetry::report("Pairing notification dispatch unavailable");
        }
        was_unavailable = unavailable;
    }
}

pub async fn run_worker() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
    let codes = PairingCodeRepository::from_env(pool.clone())?;
    let apns = ApnsClient::from_env()?;
    db::MIGRATOR.run(&pool).await?;
    let temporal = client().await?;
    let runtime = Runtime::from_current_tokio(Default::default())?;
    let tuner = TunerHolder::builder()
        .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .activity_task_slot_supplier(FixedSizeSlotSupplier::new(10))
        .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(1))
        .build();
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<PairingNotificationWorkflow>()?
        .register_activities(NotificationActivities {
            codes: codes.clone(),
            apns,
        })
        .tuner(tuner)
        .graceful_shutdown_period(Duration::from_secs(2))
        .build();
    let worker = Worker::new(&runtime, temporal.clone(), options)?;
    let dispatch = tokio::spawn(dispatch(codes, temporal));
    eprintln!("Notification worker listening on {TASK_QUEUE}");
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
