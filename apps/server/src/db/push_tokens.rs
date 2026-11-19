use std::io;

use sqlx::PgPool;
use uuid::Uuid;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::pairing_codes::Notification;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) async fn remove_if_current(
    pool: &PgPool,
    notification: &Notification,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM device_push_tokens WHERE device_id = $1 AND token = $2
         AND environment = $3 AND updated_at = $4",
    )
    .bind(notification.device_id)
    .bind(&notification.token)
    .bind(&notification.environment)
    .bind(notification.token_updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Clone, Copy)]
enum ApnsEnvironment {
    Sandbox,
    Production,
}

impl ApnsEnvironment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

#[derive(Clone)]
pub struct PushTokenRepository {
    pool: PgPool,
    environment: ApnsEnvironment,
}

impl PushTokenRepository {
    pub fn from_env(pool: PgPool) -> io::Result<Self> {
        let environment = match std::env::var("APPRELAY_APNS_ENVIRONMENT").as_deref() {
            Ok("sandbox") => ApnsEnvironment::Sandbox,
            Ok("production") => ApnsEnvironment::Production,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "APPRELAY_APNS_ENVIRONMENT must be sandbox or production",
                ));
            }
        };
        Ok(Self { pool, environment })
    }

    /// None removes the token. False means the caller does not own an active
    /// registration. Authorization and mutation share a row lock with cleanup.
    pub async fn set_token(
        &self,
        device_id: Uuid,
        credential_hash: &[u8; 32],
        token: Option<&[u8]>,
    ) -> Result<bool, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let device = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices
             WHERE id = $1 AND bearer_token_hash = $2
               AND expires_at > clock_timestamp()
             FOR UPDATE",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .fetch_optional(&mut *transaction)
        .await?;
        if device.is_none() {
            return Ok(false);
        }

        if let Some(token) = token {
            sqlx::query(
                "INSERT INTO device_push_tokens (device_id, token, environment)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (device_id) DO UPDATE SET
                     token = EXCLUDED.token,
                     environment = EXCLUDED.environment,
                     updated_at = clock_timestamp()",
            )
            .bind(device_id)
            .bind(token)
            .bind(self.environment.as_str())
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query("DELETE FROM device_push_tokens WHERE device_id = $1")
                .bind(device_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }
}
