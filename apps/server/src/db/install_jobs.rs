use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::OperationError;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    pairing_codes::{Notification, NotificationContent},
    push_tokens::{PushTokenRepository, RemovePushTokenError},
};

#[derive(Clone)]
pub(crate) struct InstallJobRepository {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
pub(crate) struct InstallJob {
    pub size_bytes: i64,
    pub sha256: String,
    pub queued_at: Option<DateTime<Utc>>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(sqlx::FromRow)]
pub(crate) struct InstallArtifact {
    pub size_bytes: i64,
    pub sha256: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(sqlx::FromRow)]
struct PushTarget {
    device_id: Uuid,
    token: Vec<u8>,
    environment: String,
    token_updated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

pub(crate) fn object_key(id: Uuid) -> String {
    format!("signed-installs/{id}.ipa")
}

// The browser capability authorizes only the exact registration and pairing
// generation it produced. Neither the target ID nor the generation comes from
// the browser. In particular an original-IPA share token cannot satisfy this.
macro_rules! live_pairing {
    () => {
        "FROM public_pairing_attempts a
         JOIN devices d ON d.id = a.device_id AND d.expires_at = a.device_expires_at
         JOIN device_pairings p ON p.device_id = d.id AND p.attempt_started_at = a.pairing_started_at
         WHERE a.token_hash = $1 AND a.status = 'success'
           AND a.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()"
    };
}

macro_rules! authorized_job {
    () => {
        concat!(
            "j.id = $2 AND j.expires_at > clock_timestamp()
             AND EXISTS (SELECT 1 ", live_pairing!(),
            " AND a.id = j.public_attempt_id AND d.id = j.device_id
              AND d.expires_at = j.device_expires_at AND p.attempt_started_at = j.pairing_started_at)"
        )
    };
}

#[derive(Debug, thiserror::Error)]
#[error("could not create install job")]
pub(crate) struct CreateInstallJobError(#[source] sqlx::Error);

impl From<CreateInstallJobError> for OperationError {
    fn from(error: CreateInstallJobError) -> Self {
        Self::at("create_install_job")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not find install job")]
pub(crate) struct FindInstallJobError(#[source] sqlx::Error);

impl From<FindInstallJobError> for OperationError {
    fn from(error: FindInstallJobError) -> Self {
        Self::at("find_install_job")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not queue install job")]
pub(crate) struct QueueInstallJobError(#[source] sqlx::Error);

impl From<QueueInstallJobError> for OperationError {
    fn from(error: QueueInstallJobError) -> Self {
        Self::at("queue_install_job")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not list expired install jobs")]
pub(crate) struct ListExpiredInstallJobsError(#[source] sqlx::Error);

impl From<ListExpiredInstallJobsError> for OperationError {
    fn from(error: ListExpiredInstallJobsError) -> Self {
        Self::at("list_expired_install_jobs")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not remove install job")]
pub(crate) struct RemoveInstallJobError(#[source] sqlx::Error);

impl From<RemoveInstallJobError> for OperationError {
    fn from(error: RemoveInstallJobError) -> Self {
        Self::at("remove_install_job")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not load install job")]
pub(crate) struct LoadInstallJobError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<LoadInstallJobError> for OperationError {
    fn from(error: LoadInstallJobError) -> Self {
        Self::at("load_install_job")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not record install outcome")]
pub(crate) struct RecordInstallOutcomeError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<RecordInstallOutcomeError> for OperationError {
    fn from(error: RecordInstallOutcomeError) -> Self {
        Self::at("record_install_outcome")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not load install notification")]
pub(crate) struct LoadInstallNotificationError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<LoadInstallNotificationError> for OperationError {
    fn from(error: LoadInstallNotificationError) -> Self {
        Self::at("load_install_notification")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not load pending install jobs")]
pub(crate) struct LoadPendingInstallJobsError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<LoadPendingInstallJobsError> for OperationError {
    fn from(error: LoadPendingInstallJobsError) -> Self {
        Self::at("load_pending_install_jobs")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not mark install job dispatched")]
pub(crate) struct MarkInstallJobDispatchedError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<MarkInstallJobDispatchedError> for OperationError {
    fn from(error: MarkInstallJobDispatchedError) -> Self {
        Self::at("mark_install_job_dispatched")(error)
    }
}

impl InstallJobRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        hash: &[u8; 32],
        id: Uuid,
        size_bytes: i64,
        sha256: &str,
    ) -> Result<Option<DateTime<Utc>>, CreateInstallJobError> {
        sqlx::query_scalar(concat!(
            "INSERT INTO install_jobs
                 (id, public_attempt_id, device_id, device_expires_at, pairing_started_at, size_bytes, sha256)
             SELECT $2, a.id, d.id, d.expires_at, p.attempt_started_at, $3, $4 ",
            live_pairing!(),
            " RETURNING LEAST(expires_at, (SELECT a.expires_at
                   FROM public_pairing_attempts a WHERE a.token_hash = $1))"
        ))
        .bind(hash.as_slice())
        .bind(id)
        .bind(size_bytes)
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await
        .map_err(CreateInstallJobError)
    }

    pub async fn find(
        &self,
        hash: &[u8; 32],
        id: Uuid,
    ) -> Result<Option<InstallJob>, FindInstallJobError> {
        sqlx::query_as(concat!(
            "SELECT j.size_bytes, j.sha256, j.queued_at FROM install_jobs j WHERE ",
            authorized_job!()
        ))
        .bind(hash.as_slice())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(FindInstallJobError)
    }

    pub async fn queue(&self, hash: &[u8; 32], id: Uuid) -> Result<bool, QueueInstallJobError> {
        // The row is the durable queue entry. Keep the original timestamp on
        // retries, and reauthorize after the R2 HEAD request.
        let result = sqlx::query(concat!(
            "UPDATE install_jobs j SET queued_at = COALESCE(j.queued_at, clock_timestamp()) WHERE ",
            authorized_job!()
        ))
        .bind(hash.as_slice())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(QueueInstallJobError)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn expired(&self) -> Result<Vec<Uuid>, ListExpiredInstallJobsError> {
        sqlx::query_scalar(
            "SELECT id FROM install_jobs WHERE expires_at <= clock_timestamp()
             ORDER BY expires_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ListExpiredInstallJobsError)
    }

    pub async fn remove(&self, id: Uuid) -> Result<(), RemoveInstallJobError> {
        sqlx::query("DELETE FROM install_jobs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(RemoveInstallJobError)?;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl InstallJobRepository {
    pub(crate) async fn load_installable(
        &self,
        id: Uuid,
    ) -> Result<Option<InstallArtifact>, LoadInstallJobError> {
        sqlx::query_as(
            "SELECT size_bytes, sha256 FROM install_jobs WHERE id = $1 AND queued_at IS NOT NULL
             AND finished_at IS NULL AND expires_at > clock_timestamp()",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(LoadInstallJobError)
    }

    pub(crate) async fn record_outcome(
        &self,
        id: Uuid,
        installed: bool,
    ) -> Result<(), RecordInstallOutcomeError> {
        let outcome = if installed { "installed" } else { "failed" };
        sqlx::query(
            "UPDATE install_jobs SET outcome = COALESCE(outcome, $2),
                 finished_at = COALESCE(finished_at, clock_timestamp())
             WHERE id = $1 AND queued_at IS NOT NULL",
        )
        .bind(id)
        .bind(outcome)
        .execute(&self.pool)
        .await
        .map_err(RecordInstallOutcomeError)?;
        Ok(())
    }

    pub(crate) async fn notification(
        &self,
        id: Uuid,
        outcome: Option<bool>,
    ) -> Result<Option<Notification>, LoadInstallNotificationError> {
        let state = match outcome {
            Some(true) => "installed",
            Some(false) => "failed",
            None => "starting",
        };
        let target = sqlx::query_as::<_, PushTarget>(
            "SELECT d.id AS device_id, t.token, t.environment, t.updated_at AS token_updated_at,
                 LEAST(j.expires_at, d.expires_at, clock_timestamp() + INTERVAL '5 minutes') AS expires_at
             FROM install_jobs j
             JOIN devices d ON d.id = j.device_id AND d.expires_at = j.device_expires_at
             JOIN device_pairings p ON p.device_id = d.id AND p.attempt_started_at = j.pairing_started_at
             JOIN device_push_tokens t ON t.device_id = d.id
             WHERE j.id = $1 AND j.queued_at IS NOT NULL
               AND j.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND (($2 = 'starting' AND j.finished_at IS NULL) OR j.outcome = $2)",
        )
        .bind(id)
        .bind(state)
        .fetch_optional(&self.pool)
        .await
        .map_err(LoadInstallNotificationError)?;
        Ok(target.map(|target| Notification {
            content: NotificationContent::Install {
                job_id: id,
                outcome,
            },
            expires_at: target.expires_at,
            device_id: target.device_id,
            token: target.token,
            environment: target.environment,
            token_updated_at: target.token_updated_at,
        }))
    }

    pub(crate) async fn remove_invalid_token(
        &self,
        notification: &Notification,
    ) -> Result<(), RemovePushTokenError> {
        PushTokenRepository::remove_if_current(&self.pool, notification).await
    }

    pub(crate) async fn pending_dispatch(&self) -> Result<Vec<Uuid>, LoadPendingInstallJobsError> {
        sqlx::query_scalar(
            "SELECT id FROM install_jobs WHERE queued_at IS NOT NULL AND dispatched_at IS NULL
             AND finished_at IS NULL AND expires_at > clock_timestamp()
             ORDER BY queued_at LIMIT 20",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(LoadPendingInstallJobsError)
    }

    pub(crate) async fn mark_dispatched(
        &self,
        id: Uuid,
    ) -> Result<(), MarkInstallJobDispatchedError> {
        sqlx::query("UPDATE install_jobs SET dispatched_at = clock_timestamp() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(MarkInstallJobDispatchedError)?;
        Ok(())
    }
}
