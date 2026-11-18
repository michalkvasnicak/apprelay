use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

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
