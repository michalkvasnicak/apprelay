use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    pairing_codes::{Notification, NotificationContent},
    push_tokens::PushTokenRepository,
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
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
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
    }

    pub async fn find(&self, hash: &[u8; 32], id: Uuid) -> Result<Option<InstallJob>, sqlx::Error> {
        sqlx::query_as(concat!(
            "SELECT j.size_bytes, j.sha256, j.queued_at FROM install_jobs j WHERE ",
            authorized_job!()
        ))
        .bind(hash.as_slice())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn queue(&self, hash: &[u8; 32], id: Uuid) -> Result<bool, sqlx::Error> {
        // The row is the durable queue entry. Keep the original timestamp on
        // retries, and reauthorize after the R2 HEAD request.
        let result = sqlx::query(concat!(
            "UPDATE install_jobs j SET queued_at = COALESCE(j.queued_at, clock_timestamp()) WHERE ",
            authorized_job!()
        ))
        .bind(hash.as_slice())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn expired(&self) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM install_jobs WHERE expires_at <= clock_timestamp()
             ORDER BY expires_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn remove(&self, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM install_jobs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl InstallJobRepository {
    pub(crate) async fn load_installable(
        &self,
        id: Uuid,
    ) -> Result<Option<InstallArtifact>, sqlx::Error> {
        sqlx::query_as(
            "SELECT size_bytes, sha256 FROM install_jobs WHERE id = $1 AND queued_at IS NOT NULL
             AND finished_at IS NULL AND expires_at > clock_timestamp()",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub(crate) async fn record_outcome(
        &self,
        id: Uuid,
        installed: bool,
    ) -> Result<(), sqlx::Error> {
        let outcome = if installed { "installed" } else { "failed" };
        sqlx::query(
            "UPDATE install_jobs SET outcome = COALESCE(outcome, $2),
                 finished_at = COALESCE(finished_at, clock_timestamp())
             WHERE id = $1 AND queued_at IS NOT NULL",
        )
        .bind(id)
        .bind(outcome)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn notification(
        &self,
        id: Uuid,
        outcome: Option<bool>,
    ) -> Result<Option<Notification>, sqlx::Error> {
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
        .await?;
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
    ) -> Result<(), sqlx::Error> {
        PushTokenRepository::remove_if_current(&self.pool, notification).await
    }

    pub(crate) async fn pending_dispatch(&self) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM install_jobs WHERE queued_at IS NOT NULL AND dispatched_at IS NULL
             AND finished_at IS NULL AND expires_at > clock_timestamp()
             ORDER BY queued_at LIMIT 20",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub(crate) async fn mark_dispatched(&self, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE install_jobs SET dispatched_at = clock_timestamp() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
