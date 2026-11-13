use std::{io, net::Ipv4Addr};

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::secrets::SecretKey;

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
struct CodeRow {
    id: Uuid,
    device_id: Uuid,
    code_id: Uuid,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    expires_at: DateTime<Utc>,
}

#[cfg(target_os = "macos")]
pub(crate) struct Notification {
    pub code: ActiveCode,
    pub device_id: Uuid,
    pub token: Vec<u8>,
    pub environment: String,
    pub token_updated_at: DateTime<Utc>,
}

fn storage_error(_: sqlx::Error) -> io::Error {
    // Database constraint errors can include secret values.
    io::Error::other("pairing code storage operation failed")
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

    fn decrypt(&self, row: CodeRow) -> io::Result<ActiveCode> {
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
        source: Ipv4Addr,
    ) -> io::Result<CodeLookup> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let authorized = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices WHERE id = $1 AND address = $2::text::inet
             AND expires_at > clock_timestamp() FOR SHARE",
        )
        .bind(device_id)
        .bind(source.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
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
        .map_err(storage_error)?;
        let result = match row {
            Some(row) => CodeLookup::Active(self.decrypt(row)?),
            None => CodeLookup::Missing,
        };
        tx.commit().await.map_err(storage_error)?;
        Ok(result)
    }
}

#[cfg(target_os = "macos")]
impl PairingCodeRepository {
    pub(crate) async fn begin(
        &self,
        device_id: Uuid,
        device_expiry: DateTime<Utc>,
        started_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> io::Result<Uuid> {
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let valid = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices WHERE id = $1 AND expires_at = $2
             AND expires_at > clock_timestamp() FOR UPDATE",
        )
        .bind(device_id)
        .bind(device_expiry)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        if valid.is_none() {
            return Err(io::Error::other("device registration changed or expired"));
        }
        let saved = sqlx::query(
            "INSERT INTO pairing_attempts (id, device_id, device_expires_at, started_at, expires_at)
             VALUES ($1, $2, $3, $4, LEAST($3, $5))
             ON CONFLICT (device_id) DO UPDATE SET
               id = EXCLUDED.id, device_expires_at = EXCLUDED.device_expires_at,
               started_at = EXCLUDED.started_at, expires_at = EXCLUDED.expires_at,
               ended_at = NULL, code_id = NULL, nonce = NULL, ciphertext = NULL,
               notification_queued_at = NULL, notification_sent_at = NULL
             WHERE pairing_attempts.started_at <= EXCLUDED.started_at",
        ).bind(id).bind(device_id).bind(device_expiry).bind(started_at).bind(expires_at)
            .execute(&mut *tx).await.map_err(storage_error)?;
        if saved.rows_affected() != 1 {
            return Err(io::Error::other(
                "a newer pairing attempt is already active",
            ));
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(id)
    }

    pub(crate) async fn publish(
        &self,
        attempt_id: Uuid,
        device_id: Uuid,
        pin: String,
    ) -> io::Result<()> {
        if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err(io::Error::other("invalid pairing code"));
        }
        let code_id = Uuid::new_v4();
        let (nonce, ciphertext) = self.key.seal(
            &associated_data(attempt_id, code_id, device_id),
            pin.into_bytes(),
        )?;
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
        .map_err(storage_error)?;
        if saved.rows_affected() != 1 {
            return Err(io::Error::other("pairing attempt is no longer active"));
        }
        Ok(())
    }

    pub(crate) async fn clear(&self, attempt_id: Uuid) -> io::Result<()> {
        sqlx::query(
            "UPDATE pairing_attempts SET code_id = NULL, nonce = NULL, ciphertext = NULL,
             notification_queued_at = NULL, notification_sent_at = NULL WHERE id = $1",
        )
        .bind(attempt_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    pub(crate) async fn end(&self, attempt_id: Uuid) -> io::Result<()> {
        sqlx::query(
            "UPDATE pairing_attempts SET ended_at = clock_timestamp(),
             code_id = NULL, nonce = NULL, ciphertext = NULL WHERE id = $1",
        )
        .bind(attempt_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    pub(crate) async fn pending_notifications(&self) -> io::Result<Vec<Uuid>> {
        sqlx::query_scalar(
            "SELECT a.code_id FROM pairing_attempts a
             JOIN devices d ON d.id = a.device_id
             JOIN device_push_tokens t ON t.device_id = d.id
             WHERE a.code_id IS NOT NULL AND a.ended_at IS NULL
               AND a.notification_queued_at IS NULL AND a.notification_sent_at IS NULL
               AND a.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND d.expires_at = a.device_expires_at
             ORDER BY a.expires_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)
    }

    pub(crate) async fn mark_queued(&self, code_id: Uuid) -> io::Result<()> {
        sqlx::query("UPDATE pairing_attempts SET notification_queued_at = clock_timestamp() WHERE code_id = $1")
            .bind(code_id).execute(&self.pool).await.map_err(storage_error)?;
        Ok(())
    }

    pub(crate) async fn notification(&self, code_id: Uuid) -> io::Result<Option<Notification>> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let row = sqlx::query_as::<_, CodeRow>(
            "SELECT a.id, a.device_id, a.code_id, a.nonce, a.ciphertext, a.expires_at
             FROM pairing_attempts a JOIN devices d ON d.id = a.device_id
             WHERE a.code_id = $1 AND a.ended_at IS NULL AND a.notification_sent_at IS NULL
               AND a.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND d.expires_at = a.device_expires_at",
        )
        .bind(code_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let token = sqlx::query_as::<_, (Vec<u8>, String, DateTime<Utc>)>(
            "SELECT token, environment, updated_at FROM device_push_tokens WHERE device_id = $1",
        )
        .bind(row.device_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some((token, environment, token_updated_at)) = token else {
            return Ok(None);
        };
        let notification = Notification {
            device_id: row.device_id,
            code: self.decrypt(row)?,
            token,
            environment,
            token_updated_at,
        };
        tx.commit().await.map_err(storage_error)?;
        Ok(Some(notification))
    }

    pub(crate) async fn mark_sent(&self, code_id: Uuid) -> io::Result<()> {
        sqlx::query("UPDATE pairing_attempts SET notification_sent_at = clock_timestamp() WHERE code_id = $1")
            .bind(code_id).execute(&self.pool).await.map_err(storage_error)?;
        Ok(())
    }

    pub(crate) async fn remove_invalid_token(&self, notification: &Notification) -> io::Result<()> {
        sqlx::query(
            "DELETE FROM device_push_tokens WHERE device_id = $1 AND token = $2
             AND environment = $3 AND updated_at = $4",
        )
        .bind(notification.device_id)
        .bind(&notification.token)
        .bind(&notification.environment)
        .bind(notification.token_updated_at)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }
}
