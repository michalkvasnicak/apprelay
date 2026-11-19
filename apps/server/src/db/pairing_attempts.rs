use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use super::pairing_codes::{CodeRow, DecryptPairingCodeError, PairingCodeRepository};
use crate::error::OperationError;

#[derive(Clone)]
pub struct PairingAttemptRepository {
    pool: PgPool,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum AttemptStatus {
    Waiting,
    Connecting,
    Pairing {
        code: String,
        expires_at: DateTime<Utc>,
    },
    Done {
        device: PairedDevice,
    },
    Failed {
        reason: FailureReason,
    },
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
    worker_started: bool,
    live_device_id: Option<Uuid>,
    live_pairing: bool,
    session_id: Option<Uuid>,
    code_id: Option<Uuid>,
    nonce: Option<Vec<u8>>,
    ciphertext: Option<Vec<u8>>,
    code_expires_at: Option<DateTime<Utc>>,
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

#[derive(Debug, thiserror::Error)]
#[error("could not create pairing attempt")]
pub(crate) struct CreatePairingAttemptError(#[source] sqlx::Error);

impl From<CreatePairingAttemptError> for OperationError {
    fn from(error: CreatePairingAttemptError) -> Self {
        Self::at("create_pairing_attempt")(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PairingAttemptStatusError {
    #[error("could not fetch pairing attempt")]
    Fetch(#[source] sqlx::Error),
    #[error("could not decrypt pairing attempt code")]
    Decrypt(#[source] DecryptPairingCodeError),
}

impl From<PairingAttemptStatusError> for OperationError {
    fn from(error: PairingAttemptStatusError) -> Self {
        let operation = match &error {
            PairingAttemptStatusError::Fetch(_) => "get_pairing_attempt",
            PairingAttemptStatusError::Decrypt(_) => "decrypt_pairing_attempt_code",
        };
        Self::at(operation)(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PairingAttemptClaimError {
    #[error("could not start pairing attempt claim")]
    Begin(#[source] sqlx::Error),
    #[error("could not authorize pairing attempt claim")]
    Authorize(#[source] sqlx::Error),
    #[error("could not expire overdue device pairing attempts")]
    ExpireOverdue(#[source] sqlx::Error),
    #[error("could not lock pairing attempt claim")]
    Lock(#[source] sqlx::Error),
    #[error("could not check device pairing conflict")]
    CheckConflict(#[source] sqlx::Error),
    #[error("could not claim pairing attempt")]
    Claim(#[source] sqlx::Error),
    #[error("could not commit pairing attempt claim")]
    Commit(#[source] sqlx::Error),
}

impl From<PairingAttemptClaimError> for OperationError {
    fn from(error: PairingAttemptClaimError) -> Self {
        let operation = match &error {
            PairingAttemptClaimError::Begin(_) => "begin_pairing_attempt_claim",
            PairingAttemptClaimError::Authorize(_) => "authorize_pairing_attempt_claim",
            PairingAttemptClaimError::ExpireOverdue(_) => "expire_overdue_device_pairing_attempts",
            PairingAttemptClaimError::Lock(_) => "lock_pairing_attempt_claim",
            PairingAttemptClaimError::CheckConflict(_) => "check_device_pairing_conflict",
            PairingAttemptClaimError::Claim(_) => "claim_pairing_attempt",
            PairingAttemptClaimError::Commit(_) => "commit_pairing_attempt_claim",
        };
        Self::at(operation)(error)
    }
}

impl PairingAttemptRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn create(
        &self,
        hash: &[u8; 32],
    ) -> Result<DateTime<Utc>, CreatePairingAttemptError> {
        sqlx::query_scalar(
            "INSERT INTO public_pairing_attempts (id, token_hash) VALUES ($1, $2) RETURNING expires_at",
        )
        .bind(Uuid::new_v4())
        .bind(hash.as_slice())
        .fetch_one(&self.pool)
        .await
        .map_err(CreatePairingAttemptError)
    }

    pub(crate) async fn status(
        &self,
        hash: &[u8; 32],
        codes: &PairingCodeRepository,
    ) -> Result<Option<AttemptStatus>, PairingAttemptStatusError> {
        let row = sqlx::query_as::<_, StatusRow>(
            "SELECT a.status, a.reason, a.expires_at <= statement_timestamp() AS expired,
                 a.worker_started_at IS NOT NULL AS worker_started,
                 d.id AS live_device_id,
                 a.paired_udid, a.paired_name, a.paired_model, a.expires_at,
                 EXISTS (SELECT 1 FROM device_pairings p WHERE p.device_id = d.id
                     AND p.attempt_started_at = a.pairing_started_at) AS live_pairing,
                 s.id AS session_id, s.code_id, s.nonce, s.ciphertext,
                 CASE WHEN s.id IS NOT NULL THEN LEAST(s.expires_at, a.expires_at)
                 END AS code_expires_at
             FROM public_pairing_attempts a
             LEFT JOIN devices d ON d.id = a.device_id AND d.expires_at = a.device_expires_at
                 AND d.expires_at > statement_timestamp()
             LEFT JOIN pairing_attempts s ON s.public_attempt_id = a.id AND s.device_id = d.id
                 AND s.device_expires_at = d.expires_at AND s.ended_at IS NULL
                 AND s.code_id IS NOT NULL AND s.expires_at > statement_timestamp()
             WHERE a.token_hash = $1",
        )
        .bind(hash.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(PairingAttemptStatusError::Fetch)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let failed = |reason| AttemptStatus::Failed { reason };
        let status = if row.expired || row.status == "expired" {
            failed(FailureReason::Timeout)
        } else {
            match row.status.as_str() {
                "pending" => AttemptStatus::Waiting,
                "pairing" => match (
                    row.live_device_id,
                    row.session_id,
                    row.code_id,
                    row.nonce,
                    row.ciphertext,
                    row.code_expires_at,
                ) {
                    (None, ..) => failed(FailureReason::PairingFailed),
                    (
                        Some(device_id),
                        Some(id),
                        Some(code_id),
                        Some(nonce),
                        Some(ciphertext),
                        Some(expires_at),
                    ) => {
                        let active = codes
                            .decrypt(CodeRow {
                                id,
                                device_id,
                                code_id,
                                nonce,
                                ciphertext,
                                expires_at,
                            })
                            .map_err(PairingAttemptStatusError::Decrypt)?;
                        AttemptStatus::Pairing {
                            code: active.code,
                            expires_at: active.expires_at,
                        }
                    }
                    _ if row.worker_started => AttemptStatus::Connecting,
                    _ => AttemptStatus::Waiting,
                },
                "success" if row.live_pairing => {
                    match (row.paired_udid, row.paired_name, row.paired_model) {
                        (Some(udid), Some(name), Some(model)) if !udid.trim().is_empty() => {
                            AttemptStatus::Done {
                                device: PairedDevice {
                                    udid,
                                    name,
                                    model,
                                    expires_at: row.expires_at,
                                },
                            }
                        }
                        _ => failed(FailureReason::PairingFailed),
                    }
                }
                "failed" if row.reason.as_deref() == Some("timeout") => {
                    failed(FailureReason::Timeout)
                }
                _ => failed(FailureReason::PairingFailed),
            }
        };
        Ok(Some(status))
    }

    pub(crate) async fn claim(
        &self,
        hash: &[u8; 32],
        device_id: Uuid,
        credential_hash: &[u8; 32],
    ) -> Result<Claim, PairingAttemptClaimError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(PairingAttemptClaimError::Begin)?;
        // Device lock serializes claims for different tokens targeting one device.
        let device_expiry = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT expires_at FROM devices WHERE id = $1 AND bearer_token_hash = $2
             AND expires_at > clock_timestamp() FOR UPDATE",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(PairingAttemptClaimError::Authorize)?;
        let Some(device_expiry) = device_expiry else {
            return Ok(Claim::Unauthorized);
        };
        sqlx::query(
            "UPDATE public_pairing_attempts SET status = 'expired'
             WHERE device_id = $1 AND status = 'pairing' AND expires_at <= clock_timestamp()",
        )
        .bind(device_id)
        .execute(&mut *tx)
        .await
        .map_err(PairingAttemptClaimError::ExpireOverdue)?;
        let row = sqlx::query_as::<_, (Option<Uuid>, String, bool, Option<DateTime<Utc>>)>(
            "SELECT device_id, status, expires_at <= clock_timestamp(), device_expires_at
             FROM public_pairing_attempts WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(PairingAttemptClaimError::Lock)?;
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
        .await
        .map_err(PairingAttemptClaimError::CheckConflict)?;
        if busy {
            return Ok(Claim::Conflict);
        }
        // This row is a durable enqueue outbox. Only IDs enter Temporal history.
        // Leave time for the VPN report after the connection and PIN windows.
        sqlx::query(
            "UPDATE public_pairing_attempts SET device_id = $2, device_expires_at = $3,
             status = 'pairing', expires_at = LEAST(expires_at, $3, clock_timestamp() + INTERVAL '5 minutes')
             WHERE token_hash = $1",
        )
        .bind(hash.as_slice())
        .bind(device_id)
        .bind(device_expiry)
        .execute(&mut *tx)
        .await
        .map_err(PairingAttemptClaimError::Claim)?;
        tx.commit()
            .await
            .map_err(PairingAttemptClaimError::Commit)?;
        Ok(Claim::Accepted)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(sqlx::FromRow)]
pub(crate) struct PendingPairingAttempt {
    pub id: Uuid,
    pub device_id: Uuid,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not mark pairing attempt failed")]
pub(crate) struct MarkPairingAttemptFailedError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<MarkPairingAttemptFailedError> for OperationError {
    fn from(error: MarkPairingAttemptFailedError) -> Self {
        Self::at("mark_pairing_attempt_failed")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not mark pairing attempt started")]
pub(crate) struct MarkPairingAttemptStartedError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<MarkPairingAttemptStartedError> for OperationError {
    fn from(error: MarkPairingAttemptStartedError) -> Self {
        Self::at("mark_pairing_attempt_started")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
pub(crate) enum RequirePairingAttemptError {
    #[error("could not validate pairing attempt")]
    Validate(#[source] sqlx::Error),
    #[error("pairing attempt is unavailable")]
    Unavailable,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<RequirePairingAttemptError> for OperationError {
    fn from(error: RequirePairingAttemptError) -> Self {
        match error {
            RequirePairingAttemptError::Validate(_) => Self::at("validate_pairing_attempt")(error),
            RequirePairingAttemptError::Unavailable => {
                Self::without_source("pairing_attempt_unavailable")
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not expire pairing attempts")]
pub(crate) struct ExpirePairingAttemptsError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<ExpirePairingAttemptsError> for OperationError {
    fn from(error: ExpirePairingAttemptsError) -> Self {
        Self::at("expire_pairing_attempts")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not load pending pairing attempts")]
pub(crate) struct LoadPendingPairingAttemptsError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<LoadPendingPairingAttemptsError> for OperationError {
    fn from(error: LoadPendingPairingAttemptsError) -> Self {
        Self::at("load_pending_pairing_attempts")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not mark pairing attempt queued")]
pub(crate) struct MarkPairingAttemptQueuedError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<MarkPairingAttemptQueuedError> for OperationError {
    fn from(error: MarkPairingAttemptQueuedError) -> Self {
        Self::at("mark_pairing_attempt_queued")(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl PairingAttemptRepository {
    pub(crate) async fn mark_failed(
        &self,
        attempt: Option<Uuid>,
    ) -> Result<(), MarkPairingAttemptFailedError> {
        sqlx::query(
            "UPDATE public_pairing_attempts SET status = 'failed',
             reason = CASE WHEN expires_at <= clock_timestamp() THEN 'timeout' ELSE 'pairing_failed' END
             WHERE id = $1 AND status = 'pairing'",
        )
        .bind(attempt)
        .execute(&self.pool)
        .await
        .map_err(MarkPairingAttemptFailedError)?;
        Ok(())
    }

    pub(crate) async fn mark_worker_started(
        &self,
        attempt_id: Uuid,
        device_id: Uuid,
    ) -> Result<(), MarkPairingAttemptStartedError> {
        sqlx::query(
            "UPDATE public_pairing_attempts
             SET worker_started_at = COALESCE(worker_started_at, clock_timestamp())
             WHERE id = $1 AND device_id = $2 AND status = 'pairing'
               AND expires_at > clock_timestamp()",
        )
        .bind(attempt_id)
        .bind(device_id)
        .execute(&self.pool)
        .await
        .map_err(MarkPairingAttemptStartedError)?;
        Ok(())
    }

    pub(crate) async fn require_attempt(
        &self,
        device: Uuid,
        attempt: Option<Uuid>,
    ) -> Result<Option<DateTime<Utc>>, RequirePairingAttemptError> {
        let valid = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT a.expires_at FROM public_pairing_attempts a JOIN devices d ON d.id = a.device_id
                WHERE a.device_id = $1 AND ($2::uuid IS NULL OR a.id = $2)
                  AND a.status = 'pairing' AND a.expires_at > clock_timestamp()
                  AND d.expires_at = a.device_expires_at AND d.expires_at > clock_timestamp()",
        )
        .bind(device)
        .bind(attempt)
        .fetch_optional(&self.pool)
        .await
        .map_err(RequirePairingAttemptError::Validate)?;
        // CLI sessions cannot race a web-owned session.
        if valid.is_some() != attempt.is_some() {
            return Err(RequirePairingAttemptError::Unavailable);
        }
        Ok(valid)
    }

    pub(crate) async fn expire_overdue(&self) -> Result<(), ExpirePairingAttemptsError> {
        sqlx::query(
            "UPDATE public_pairing_attempts SET status = 'expired'
             WHERE status IN ('pending', 'pairing') AND expires_at <= clock_timestamp()",
        )
        .execute(&self.pool)
        .await
        .map_err(ExpirePairingAttemptsError)?;
        Ok(())
    }

    pub(crate) async fn pending_dispatch(
        &self,
    ) -> Result<Vec<PendingPairingAttempt>, LoadPendingPairingAttemptsError> {
        sqlx::query_as(
            "SELECT id, device_id FROM public_pairing_attempts
             WHERE status = 'pairing' AND queued_at IS NULL AND expires_at > clock_timestamp()
             ORDER BY expires_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(LoadPendingPairingAttemptsError)
    }

    pub(crate) async fn mark_queued(
        &self,
        attempt: Uuid,
    ) -> Result<(), MarkPairingAttemptQueuedError> {
        sqlx::query(
            "UPDATE public_pairing_attempts SET queued_at = clock_timestamp() WHERE id = $1",
        )
        .bind(attempt)
        .execute(&self.pool)
        .await
        .map_err(MarkPairingAttemptQueuedError)?;
        Ok(())
    }
}
