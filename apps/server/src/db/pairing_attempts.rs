use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone)]
pub struct PairingAttemptRepository {
    pool: PgPool,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum AttemptStatus {
    Pending,
    Pairing,
    Success { device: PairedDevice },
    Failed { reason: FailureReason },
    Expired,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct PairedDevice {
    /// Provisionally the remote-pairing identifier, not a verified hardware UDID.
    pub udid: String,
    pub name: String,
    pub model: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct StatusRow {
    status: String,
    reason: Option<String>,
    expired: bool,
    live_pairing: bool,
    paired_udid: Option<String>,
    paired_name: Option<String>,
    paired_model: Option<String>,
    expires_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureReason {
    PairingFailed,
    Timeout,
}

pub(crate) enum Claim {
    Accepted,
    Unauthorized,
    Missing,
    Conflict,
    Expired,
}

impl PairingAttemptRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn create(&self, hash: &[u8; 32]) -> Result<DateTime<Utc>, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO public_pairing_attempts (id, token_hash) VALUES ($1, $2) RETURNING expires_at",
        )
        .bind(Uuid::new_v4())
        .bind(hash.as_slice())
        .fetch_one(&self.pool)
        .await
    }

    pub(crate) async fn status(
        &self,
        hash: &[u8; 32],
    ) -> Result<Option<AttemptStatus>, sqlx::Error> {
        let row = sqlx::query_as::<_, StatusRow>(
            "SELECT a.status, a.reason, a.expires_at <= clock_timestamp() AS expired,
                 a.paired_udid, a.paired_name, a.paired_model, a.expires_at,
                 EXISTS (SELECT 1 FROM devices d
                     JOIN device_pairings p ON p.device_id = d.id
                     WHERE d.id = a.device_id AND d.expires_at = a.device_expires_at
                       AND d.expires_at > clock_timestamp()
                       AND p.attempt_started_at = a.pairing_started_at) AS live_pairing
             FROM public_pairing_attempts a WHERE a.token_hash = $1",
        )
        .bind(hash.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            if row.expired {
                return AttemptStatus::Expired;
            }
            match row.status.as_str() {
                "pending" => AttemptStatus::Pending,
                "pairing" => AttemptStatus::Pairing,
                "success" if row.live_pairing => {
                    match (row.paired_udid, row.paired_name, row.paired_model) {
                        (Some(udid), Some(name), Some(model)) if !udid.trim().is_empty() => {
                            AttemptStatus::Success {
                                device: PairedDevice {
                                    udid,
                                    name,
                                    model,
                                    expires_at: row.expires_at,
                                },
                            }
                        }
                        _ => AttemptStatus::Expired,
                    }
                }
                "failed" => AttemptStatus::Failed {
                    reason: if row.reason.as_deref() == Some("timeout") {
                        FailureReason::Timeout
                    } else {
                        FailureReason::PairingFailed
                    },
                },
                _ => AttemptStatus::Expired,
            }
        }))
    }

    pub(crate) async fn claim(
        &self,
        hash: &[u8; 32],
        device_id: Uuid,
        credential_hash: &[u8; 32],
    ) -> Result<Claim, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // Device lock serializes claims for different tokens targeting one device.
        let device_expiry = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT expires_at FROM devices WHERE id = $1 AND bearer_token_hash = $2
             AND expires_at > clock_timestamp() FOR UPDATE",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(device_expiry) = device_expiry else {
            return Ok(Claim::Unauthorized);
        };
        sqlx::query(
            "UPDATE public_pairing_attempts SET status = 'expired'
             WHERE device_id = $1 AND status = 'pairing' AND expires_at <= clock_timestamp()",
        )
        .bind(device_id)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as::<_, (Option<Uuid>, String, bool, Option<DateTime<Utc>>)>(
            "SELECT device_id, status, expires_at <= clock_timestamp(), device_expires_at
             FROM public_pairing_attempts WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let Some((owner, status, expired, claimed_expiry)) = row else {
            return Ok(Claim::Missing);
        };
        if expired {
            return Ok(Claim::Expired);
        }
        if let Some(owner) = owner {
            return Ok(
                if owner == device_id
                    && claimed_expiry == Some(device_expiry)
                    && matches!(status.as_str(), "pairing" | "success")
                {
                    Claim::Accepted
                } else {
                    Claim::Conflict
                },
            );
        }
        let busy = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM public_pairing_attempts WHERE device_id = $1 AND status = 'pairing')
             OR EXISTS (SELECT 1 FROM pairing_attempts WHERE device_id = $1
                 AND ended_at IS NULL AND expires_at > clock_timestamp())",
        )
        .bind(device_id)
        .fetch_one(&mut *tx)
        .await?;
        if busy {
            return Ok(Claim::Conflict);
        }
        // This row is a durable enqueue outbox. Only IDs enter Temporal history.
        sqlx::query(
            "UPDATE public_pairing_attempts SET device_id = $2, device_expires_at = $3,
             status = 'pairing', expires_at = LEAST(expires_at, $3, clock_timestamp() + INTERVAL '3 minutes')
             WHERE token_hash = $1",
        )
        .bind(hash.as_slice())
        .bind(device_id)
        .bind(device_expiry)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Claim::Accepted)
    }
}
