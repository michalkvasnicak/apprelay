use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::num::NonZeroU32;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct Device {
    pub id: Uuid,
    pub wireguard_public_key: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct DeviceRepository {
    pool: PgPool,
    lifetime_seconds: NonZeroU32,
}

impl DeviceRepository {
    pub fn new(pool: PgPool, lifetime_seconds: NonZeroU32) -> Self {
        Self {
            pool,
            lifetime_seconds,
        }
    }

    /// Looks up an unexpired device by its decoded, 32-byte WireGuard public key.
    pub async fn find_by_public_key(
        &self,
        public_key: &[u8; 32],
    ) -> Result<Option<Device>, sqlx::Error> {
        sqlx::query_as::<_, Device>(
            "SELECT id, wireguard_public_key, created_at, expires_at
             FROM devices
             WHERE wireguard_public_key = $1 AND expires_at > statement_timestamp()",
        )
        .bind(public_key.as_slice())
        .fetch_optional(&self.pool)
        .await
    }

    /// Registers a device, or returns its existing unexpired registration.
    ///
    /// Expired registrations receive a new lifetime. Active registrations are
    /// not extended by retries. The atomic upsert handles concurrent requests;
    /// existing IDs and creation times are preserved.
    /// This records identity only; it does not provision or remove VPN peers.
    pub async fn register(&self, public_key: &[u8; 32]) -> Result<Device, sqlx::Error> {
        sqlx::query_as::<_, Device>(
            "INSERT INTO devices (id, wireguard_public_key, expires_at)
             VALUES ($1, $2, statement_timestamp() + $3::bigint * INTERVAL '1 second')
             ON CONFLICT (wireguard_public_key) DO UPDATE
             SET expires_at = CASE
                 WHEN devices.expires_at <= statement_timestamp() THEN EXCLUDED.expires_at
                 ELSE devices.expires_at
             END
             RETURNING id, wireguard_public_key, created_at, expires_at",
        )
        .bind(Uuid::new_v4())
        .bind(public_key.as_slice())
        .bind(i64::from(self.lifetime_seconds.get()))
        .fetch_one(&self.pool)
        .await
    }
}
