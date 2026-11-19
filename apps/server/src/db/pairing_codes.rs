use std::io;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::OperationError, secrets::SecretKey};

#[derive(Clone)]
pub struct PairingCodeRepository {
    pool: PgPool,
    key: SecretKey,
}

pub(crate) struct ActiveCode {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

pub(crate) enum CodeLookup {
    Forbidden,
    Missing,
    Active(ActiveCode),
}

#[derive(sqlx::FromRow)]
pub(crate) struct CodeRow {
    pub id: Uuid,
    pub device_id: Uuid,
    pub code_id: Uuid,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub expires_at: DateTime<Utc>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) enum NotificationContent {
    Started,
    Code(ActiveCode),
    Success,
    Timeout,
    Install { job_id: Uuid, outcome: Option<bool> },
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) struct Notification {
    pub content: NotificationContent,
    pub expires_at: DateTime<Utc>,
    pub device_id: Uuid,
    pub token: Vec<u8>,
    pub environment: String,
    pub token_updated_at: DateTime<Utc>,
}

fn associated_data(attempt: Uuid, code: Uuid, device: Uuid) -> Vec<u8> {
    let mut data = b"apprelay/pairing-code/v1/".to_vec();
    for id in [attempt, code, device] {
        data.extend_from_slice(id.as_bytes());
    }
    data
}

impl PairingCodeRepository {
    pub fn from_env(pool: PgPool) -> io::Result<Self> {
        Ok(Self {
            pool,
            key: SecretKey::from_env()?,
        })
    }

    pub(crate) fn decrypt(&self, row: CodeRow) -> io::Result<ActiveCode> {
        let plaintext = self.key.open(
            &associated_data(row.id, row.code_id, row.device_id),
            &row.nonce,
            row.ciphertext,
        )?;
        if plaintext.len() != 6 || !plaintext.iter().all(u8::is_ascii_digit) {
            return Err(io::Error::other("invalid stored pairing code"));
        }
        Ok(ActiveCode {
            code: String::from_utf8(plaintext.to_vec())
                .map_err(|_| io::Error::other("invalid stored pairing code"))?,
            expires_at: row.expires_at,
        })
    }

    pub(crate) async fn for_device(
        &self,
        device_id: Uuid,
        credential_hash: &[u8; 32],
    ) -> Result<CodeLookup, OperationError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(OperationError::at("begin_pairing_code_lookup"))?;
        let authorized = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices WHERE id = $1 AND bearer_token_hash = $2
             AND expires_at > clock_timestamp() FOR SHARE",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(OperationError::at("authorize_pairing_code_lookup"))?;
        if authorized.is_none() {
            return Ok(CodeLookup::Forbidden);
        }
        let row = sqlx::query_as::<_, CodeRow>(
            "SELECT a.id, a.device_id, a.code_id, a.nonce, a.ciphertext, a.expires_at
             FROM pairing_attempts a JOIN devices d ON d.id = a.device_id
             WHERE a.device_id = $1 AND a.ended_at IS NULL AND a.code_id IS NOT NULL
               AND a.expires_at > clock_timestamp() AND d.expires_at = a.device_expires_at
               AND d.expires_at > clock_timestamp()",
        )
        .bind(device_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(OperationError::at("get_pairing_code"))?;
        let result = match row {
            Some(row) => CodeLookup::Active(
                self.decrypt(row)
                    .map_err(OperationError::at("decrypt_device_pairing_code"))?,
            ),
            None => CodeLookup::Missing,
        };
        tx.commit()
            .await
            .map_err(OperationError::at("commit_pairing_code_lookup"))?;
        Ok(result)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl PairingCodeRepository {
    pub(crate) async fn begin(
        &self,
        device_id: Uuid,
        device_expiry: DateTime<Utc>,
        started_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        public_attempt_id: Option<Uuid>,
    ) -> Result<Uuid, OperationError> {
        let id = Uuid::new_v4();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(OperationError::at("begin_pairing_attempt_transaction"))?;
        let valid = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices WHERE id = $1 AND expires_at = $2
             AND expires_at > clock_timestamp() FOR UPDATE",
        )
        .bind(device_id)
        .bind(device_expiry)
        .fetch_optional(&mut *tx)
        .await
        .map_err(OperationError::at("lock_pairing_code_device"))?;
        if valid.is_none() {
            return Err(OperationError::without_source("pairing_device_unavailable"));
        }
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM public_pairing_attempts
             WHERE device_id = $1 AND status = 'pairing' AND expires_at > clock_timestamp()
               AND ($2::uuid IS NULL OR id = $2))",
        )
        .bind(device_id)
        .bind(public_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(OperationError::at("check_pairing_attempt_owner"))?;
        if owned != public_attempt_id.is_some() {
            return Err(OperationError::without_source("pairing_ownership_changed"));
        }
        let saved = sqlx::query(
            "INSERT INTO pairing_attempts
                 (id, device_id, device_expires_at, started_at, expires_at, timeout_notification_id, public_attempt_id)
             VALUES ($1, $2, $3, $4, LEAST($3, $5), $6, $7)
             ON CONFLICT (device_id) DO UPDATE SET
               id = EXCLUDED.id, device_expires_at = EXCLUDED.device_expires_at,
               started_at = EXCLUDED.started_at, expires_at = EXCLUDED.expires_at,
               public_attempt_id = EXCLUDED.public_attempt_id,
               ended_at = NULL, code_id = NULL, nonce = NULL, ciphertext = NULL,
               notification_queued_at = NULL, notification_sent_at = NULL,
               started_notification_queued_at = NULL, started_notification_sent_at = NULL,
               success_notification_id = NULL, success_notification_expires_at = NULL,
               success_notification_queued_at = NULL, success_notification_sent_at = NULL,
               timeout_notification_id = EXCLUDED.timeout_notification_id,
               timeout_notification_queued_at = NULL, timeout_notification_sent_at = NULL
             WHERE pairing_attempts.started_at <= EXCLUDED.started_at
               AND (pairing_attempts.ended_at IS NOT NULL OR pairing_attempts.expires_at <= clock_timestamp())",
        )
        .bind(id)
        .bind(device_id)
        .bind(device_expiry)
        .bind(started_at)
        .bind(expires_at)
        .bind(Uuid::new_v4())
        .bind(public_attempt_id)
        .execute(&mut *tx)
        .await
        .map_err(OperationError::at("insert_pairing_attempt"))?;
        if saved.rows_affected() != 1 {
            return Err(OperationError::without_source(
                "pairing_attempt_already_active",
            ));
        }
        tx.commit()
            .await
            .map_err(OperationError::at("commit_pairing_attempt"))?;
        Ok(id)
    }

    pub(crate) async fn publish(
        &self,
        attempt_id: Uuid,
        device_id: Uuid,
        pin: String,
    ) -> Result<(), OperationError> {
        if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err(OperationError::without_source("invalid_pairing_code"));
        }
        let code_id = Uuid::new_v4();
        let (nonce, ciphertext) = self
            .key
            .seal(
                &associated_data(attempt_id, code_id, device_id),
                pin.into_bytes(),
            )
            .map_err(OperationError::at("encrypt_pairing_code"))?;
        // This row is also the outbox: the notification worker discovers it
        // independently, so a crash after publication cannot lose the enqueue.
        let saved = sqlx::query(
            "UPDATE pairing_attempts a SET code_id = $3, nonce = $4, ciphertext = $5,
                 notification_queued_at = NULL, notification_sent_at = NULL
             FROM devices d WHERE a.id = $1 AND a.device_id = $2 AND d.id = a.device_id
               AND a.ended_at IS NULL AND a.expires_at > clock_timestamp()
               AND d.expires_at = a.device_expires_at AND d.expires_at > clock_timestamp()",
        )
        .bind(attempt_id)
        .bind(device_id)
        .bind(code_id)
        .bind(nonce.as_slice())
        .bind(ciphertext)
        .execute(&self.pool)
        .await
        .map_err(OperationError::at("publish_pairing_code"))?;
        if saved.rows_affected() != 1 {
            return Err(OperationError::without_source(
                "pairing_attempt_unavailable",
            ));
        }
        Ok(())
    }

    pub(crate) async fn clear(&self, attempt_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pairing_attempts SET code_id = NULL, nonce = NULL, ciphertext = NULL,
             notification_queued_at = NULL, notification_sent_at = NULL WHERE id = $1",
        )
        .bind(attempt_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn end(&self, attempt_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pairing_attempts SET ended_at = clock_timestamp(),
             code_id = NULL, nonce = NULL, ciphertext = NULL WHERE id = $1",
        )
        .bind(attempt_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn pending_notifications(&self) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT notification.id FROM pairing_attempts a
             JOIN devices d ON d.id = a.device_id
             JOIN device_push_tokens t ON t.device_id = d.id
             CROSS JOIN LATERAL (VALUES
                 (a.id, a.started_notification_queued_at, a.started_notification_sent_at,
                     a.expires_at, a.ended_at IS NULL),
                 (a.code_id, a.notification_queued_at, a.notification_sent_at,
                     a.expires_at, a.ended_at IS NULL),
                 (a.success_notification_id, a.success_notification_queued_at,
                     a.success_notification_sent_at, a.success_notification_expires_at, TRUE),
                 (a.timeout_notification_id, a.timeout_notification_queued_at,
                     a.timeout_notification_sent_at,
                     LEAST(a.expires_at + INTERVAL '60 seconds', d.expires_at),
                     a.expires_at <= clock_timestamp() AND a.success_notification_id IS NULL)
             ) AS notification(id, queued_at, sent_at, expires_at, active)
             WHERE notification.id IS NOT NULL AND notification.active
               AND notification.queued_at IS NULL AND notification.sent_at IS NULL
               AND notification.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND d.expires_at = a.device_expires_at
             ORDER BY a.expires_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub(crate) async fn mark_queued(&self, notification_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pairing_attempts SET
                 started_notification_queued_at = CASE WHEN id = $1
                     THEN clock_timestamp() ELSE started_notification_queued_at END,
                 notification_queued_at = CASE WHEN code_id = $1
                     THEN clock_timestamp() ELSE notification_queued_at END,
                 success_notification_queued_at = CASE WHEN success_notification_id = $1
                     THEN clock_timestamp() ELSE success_notification_queued_at END,
                 timeout_notification_queued_at = CASE WHEN timeout_notification_id = $1
                     THEN clock_timestamp() ELSE timeout_notification_queued_at END
             WHERE id = $1 OR code_id = $1 OR success_notification_id = $1
                OR timeout_notification_id = $1",
        )
        .bind(notification_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn notification(
        &self,
        notification_id: Uuid,
    ) -> Result<Option<Notification>, OperationError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(OperationError::at("begin_pairing_notification_lookup"))?;
        let attempt = sqlx::query_as::<_, (Uuid, Uuid, DateTime<Utc>, bool, bool)>(
            "SELECT a.id, a.device_id, notification.expires_at,
                    COALESCE(a.success_notification_id = $1, FALSE),
                    COALESCE(a.timeout_notification_id = $1, FALSE)
             FROM pairing_attempts a JOIN devices d ON d.id = a.device_id
             CROSS JOIN LATERAL (VALUES
                 (a.id, a.started_notification_sent_at, a.expires_at, a.ended_at IS NULL),
                 (a.code_id, a.notification_sent_at, a.expires_at, a.ended_at IS NULL),
                 (a.success_notification_id, a.success_notification_sent_at,
                     a.success_notification_expires_at, TRUE),
                 (a.timeout_notification_id, a.timeout_notification_sent_at,
                     LEAST(a.expires_at + INTERVAL '60 seconds', d.expires_at),
                     a.expires_at <= clock_timestamp() AND a.success_notification_id IS NULL)
             ) AS notification(id, sent_at, expires_at, active)
             WHERE notification.id = $1 AND notification.sent_at IS NULL AND notification.active
               AND notification.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND d.expires_at = a.device_expires_at
             FOR SHARE OF a, d",
        )
        .bind(notification_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(OperationError::at("load_pairing_notification"))?;
        let Some((attempt_id, device_id, expires_at, success, timed_out)) = attempt else {
            return Ok(None);
        };
        let content = if success {
            NotificationContent::Success
        } else if timed_out {
            NotificationContent::Timeout
        } else if notification_id == attempt_id {
            NotificationContent::Started
        } else {
            let row = sqlx::query_as::<_, CodeRow>(
                "SELECT id, device_id, code_id, nonce, ciphertext, expires_at
                 FROM pairing_attempts WHERE id = $1 AND code_id = $2",
            )
            .bind(attempt_id)
            .bind(notification_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(OperationError::at("load_notification_pairing_code"))?;
            NotificationContent::Code(
                self.decrypt(row)
                    .map_err(OperationError::at("decrypt_notification_pairing_code"))?,
            )
        };
        let token = sqlx::query_as::<_, (Vec<u8>, String, DateTime<Utc>)>(
            "SELECT token, environment, updated_at FROM device_push_tokens WHERE device_id = $1",
        )
        .bind(device_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(OperationError::at("load_pairing_push_token"))?;
        let Some((token, environment, token_updated_at)) = token else {
            return Ok(None);
        };
        let notification = Notification {
            device_id,
            content,
            expires_at,
            token,
            environment,
            token_updated_at,
        };
        tx.commit()
            .await
            .map_err(OperationError::at("commit_pairing_notification_lookup"))?;
        Ok(Some(notification))
    }

    pub(crate) async fn mark_sent(&self, notification_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pairing_attempts SET
                 started_notification_sent_at = CASE WHEN id = $1
                     THEN clock_timestamp() ELSE started_notification_sent_at END,
                 notification_sent_at = CASE WHEN code_id = $1
                     THEN clock_timestamp() ELSE notification_sent_at END,
                 success_notification_sent_at = CASE WHEN success_notification_id = $1
                     THEN clock_timestamp() ELSE success_notification_sent_at END,
                 timeout_notification_sent_at = CASE WHEN timeout_notification_id = $1
                     THEN clock_timestamp() ELSE timeout_notification_sent_at END
             WHERE id = $1 OR code_id = $1 OR success_notification_id = $1
                OR timeout_notification_id = $1",
        )
        .bind(notification_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn remove_invalid_token(
        &self,
        notification: &Notification,
    ) -> Result<(), OperationError> {
        super::push_tokens::PushTokenRepository::remove_if_current(&self.pool, notification).await
    }
}
