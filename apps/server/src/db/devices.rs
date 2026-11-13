use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use sqlx::{PgPool, Postgres, Transaction};
use std::num::NonZeroU32;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct Device {
    pub id: Uuid,
    pub wireguard_public_key: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub address: Option<String>,
}

#[derive(Clone)]
pub struct DeviceRepository {
    pool: PgPool,
    lifetime_seconds: NonZeroU32,
}

/// Keeps cleanup and other registration requests out during peer provisioning.
pub struct LockedDevice {
    pub device: Device,
    transaction: Transaction<'static, Postgres>,
}

impl LockedDevice {
    pub async fn commit(self) -> Result<Device, sqlx::Error> {
        self.transaction.commit().await?;
        Ok(self.device)
    }
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
            "SELECT id, wireguard_public_key, created_at, expires_at, host(address) AS address
             FROM devices
             WHERE wireguard_public_key = $1 AND expires_at > statement_timestamp()",
        )
        .bind(public_key.as_slice())
        .fetch_optional(&self.pool)
        .await
    }

    /// Durably reserves an address before any agent call. A failed/uncertain
    /// provisioning result must not release an address potentially in use.
    /// Returns None if the pool is exhausted. Active retries preserve expiry.
    pub async fn register(
        &self,
        public_key: &[u8; 32],
        subnet: Ipv4Net,
    ) -> Result<Option<Device>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        // Serialize address reservations across server instances. Cleanup uses
        // row locks and only frees an address after the agent confirms removal.
        sqlx::query("SELECT pg_advisory_xact_lock(71827901)")
            .execute(&mut *transaction)
            .await?;
        let mut device = sqlx::query_as::<_, Device>(
            "INSERT INTO devices (id, wireguard_public_key, expires_at)
             VALUES ($1, $2, statement_timestamp() + $3::bigint * INTERVAL '1 second')
             ON CONFLICT (wireguard_public_key) DO UPDATE
             SET expires_at = CASE
                 WHEN devices.expires_at <= statement_timestamp() THEN EXCLUDED.expires_at
                 ELSE devices.expires_at
             END
             RETURNING id, wireguard_public_key, created_at, expires_at, host(address) AS address",
        )
        .bind(Uuid::new_v4())
        .bind(public_key.as_slice())
        .bind(i64::from(self.lifetime_seconds.get()))
        .fetch_one(&mut *transaction)
        .await?;

        if device.address.is_none() {
            // Find the first gap without enumerating every address in a large
            // subnet. The network, broadcast and server addresses are excluded.
            let address = sqlx::query_scalar::<_, String>(
                "WITH candidates AS (
                    SELECT $1::text::inet + 1 AS address
                    UNION SELECT $3::text::inet + 1
                    UNION SELECT address + 1 FROM devices
                        WHERE address >= $1::text::inet AND address < $2::text::inet
                 )
                 SELECT host(candidate.address) FROM candidates candidate
                 WHERE candidate.address > $1::text::inet AND candidate.address < $2::text::inet
                   AND candidate.address <> $3::text::inet
                   AND NOT EXISTS (SELECT 1 FROM devices WHERE address = candidate.address)
                 ORDER BY candidate.address LIMIT 1",
            )
            .bind(subnet.network().to_string())
            .bind(subnet.broadcast().to_string())
            .bind(subnet.addr().to_string())
            .fetch_optional(&mut *transaction)
            .await?;
            let Some(address) = address else {
                return Ok(None);
            };
            sqlx::query("UPDATE devices SET address = $2::text::inet WHERE id = $1")
                .bind(device.id)
                .bind(&address)
                .execute(&mut *transaction)
                .await?;
            device.address = Some(address);
        }
        transaction.commit().await?;
        Ok(Some(device))
    }

    pub async fn lock_for_provisioning(
        &self,
        id: Uuid,
    ) -> Result<Option<LockedDevice>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let device = sqlx::query_as::<_, Device>(
            "SELECT id, wireguard_public_key, created_at, expires_at, host(address) AS address
             FROM devices
             WHERE id = $1 AND expires_at > statement_timestamp()
             FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        Ok(device.map(|device| LockedDevice {
            device,
            transaction,
        }))
    }
}
