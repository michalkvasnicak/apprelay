use std::io;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::OperationError;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::pairing_codes::Notification;

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

#[derive(Debug, thiserror::Error)]
pub enum PushTokenUpdateError {
    #[error("no active registration matches the device credential")]
    Unauthorized,
    #[error("could not start push token update")]
    Begin(#[source] sqlx::Error),
    #[error("could not authorize push token update")]
    Authorize(#[source] sqlx::Error),
    #[error("could not save push token")]
    Upsert(#[source] sqlx::Error),
    #[error("could not delete push token")]
    Delete(#[source] sqlx::Error),
    #[error("could not commit push token update")]
    Commit(#[source] sqlx::Error),
}

impl From<PushTokenUpdateError> for OperationError {
    fn from(error: PushTokenUpdateError) -> Self {
        let operation = match &error {
            PushTokenUpdateError::Unauthorized => "push_token_update_unauthorized",
            PushTokenUpdateError::Begin(_) => "begin_push_token_update",
            PushTokenUpdateError::Authorize(_) => "authorize_push_token_update",
            PushTokenUpdateError::Upsert(_) => "upsert_push_token",
            PushTokenUpdateError::Delete(_) => "delete_push_token",
            PushTokenUpdateError::Commit(_) => "commit_push_token_update",
        };
        Self::at(operation)(error)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
#[error("could not remove invalid push token")]
pub(crate) struct RemovePushTokenError(#[source] sqlx::Error);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl From<RemovePushTokenError> for OperationError {
    fn from(error: RemovePushTokenError) -> Self {
        Self::at("remove_invalid_push_token")(error)
    }
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) async fn remove_if_current(
        pool: &PgPool,
        notification: &Notification,
    ) -> Result<(), RemovePushTokenError> {
        sqlx::query(
            "DELETE FROM device_push_tokens WHERE device_id = $1 AND token = $2
             AND environment = $3 AND updated_at = $4",
        )
        .bind(notification.device_id)
        .bind(&notification.token)
        .bind(&notification.environment)
        .bind(notification.token_updated_at)
        .execute(pool)
        .await
        .map_err(RemovePushTokenError)?;
        Ok(())
    }

    /// None removes the token. Authorization and mutation share a row lock with cleanup.
    pub async fn set_token(
        &self,
        device_id: Uuid,
        credential_hash: &[u8; 32],
        token: Option<&[u8]>,
    ) -> Result<(), PushTokenUpdateError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PushTokenUpdateError::Begin)?;
        let device = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices
             WHERE id = $1 AND bearer_token_hash = $2
               AND expires_at > clock_timestamp()
             FOR UPDATE",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(PushTokenUpdateError::Authorize)?;
        if device.is_none() {
            return Err(PushTokenUpdateError::Unauthorized);
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
            .await
            .map_err(PushTokenUpdateError::Upsert)?;
        } else {
            sqlx::query("DELETE FROM device_push_tokens WHERE device_id = $1")
                .bind(device_id)
                .execute(&mut *transaction)
                .await
                .map_err(PushTokenUpdateError::Delete)?;
        }
        transaction
            .commit()
            .await
            .map_err(PushTokenUpdateError::Commit)?;
        Ok(())
    }
}
