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
    pub address: String,
}

pub enum Registration {
    Registered { device: Device, created: bool },
    Unauthorized,
    Exhausted,
}

#[derive(Clone)]
pub struct DeviceRepository {
    pool: PgPool,
    lifetime_seconds: NonZeroU32,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) struct PairingDeviceRepository {
    pool: PgPool,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(sqlx::FromRow)]
pub(crate) struct DevicePeer {
    pub wireguard_public_key: Vec<u8>,
    pub expires_at: DateTime<Utc>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(sqlx::FromRow)]
pub(crate) struct PairingTarget {
    pub address: String,
    pub expires_at: DateTime<Utc>,
    pub attempt_started_at: DateTime<Utc>,
    pub wireguard_public_key: Vec<u8>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl PairingDeviceRepository {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn active_peer(
        &self,
        device: Uuid,
    ) -> Result<Option<DevicePeer>, sqlx::Error> {
        sqlx::query_as(
            "SELECT wireguard_public_key, expires_at FROM devices
             WHERE id = $1 AND expires_at > statement_timestamp()",
        )
        .bind(device)
        .fetch_optional(&self.pool)
        .await
    }

    pub(crate) async fn pairing_target(
        &self,
        device: Uuid,
    ) -> Result<Option<PairingTarget>, sqlx::Error> {
        sqlx::query_as(
            "SELECT host(address) AS address, expires_at,
                    statement_timestamp() AS attempt_started_at, wireguard_public_key
             FROM devices WHERE id = $1 AND expires_at > statement_timestamp()",
        )
        .bind(device)
        .fetch_optional(&self.pool)
        .await
    }
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
    /// Active authenticated retries preserve expiry; expired credentials may renew.
    pub async fn register(
        &self,
        public_key: &[u8; 32],
        subnet: Ipv4Net,
        credential_hash: Option<&[u8; 32]>,
        new_credential_hash: &[u8; 32],
    ) -> Result<Registration, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        // Serialize address reservations across server instances. Cleanup uses
        // row locks and only frees an address after the agent confirms removal.
        sqlx::query("SELECT pg_advisory_xact_lock(71827901)")
            .execute(&mut *transaction)
            .await?;
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM devices WHERE wireguard_public_key = $1)",
        )
        .bind(public_key.as_slice())
        .fetch_one(&mut *transaction)
        .await?;
        // Authenticate before changing expiry. Legacy rows with no hash cannot renew.
        let existing = sqlx::query_as::<_, Device>(
            "UPDATE devices
             SET expires_at = CASE
                 WHEN expires_at <= statement_timestamp()
                 THEN statement_timestamp() + $2::bigint * INTERVAL '1 second'
                 ELSE expires_at
             END
             WHERE wireguard_public_key = $1 AND bearer_token_hash = $3
             RETURNING id, wireguard_public_key, created_at, expires_at, host(address) AS address",
        )
        .bind(public_key.as_slice())
        .bind(i64::from(self.lifetime_seconds.get()))
        .bind(credential_hash.map(|hash| hash.as_slice()))
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(device) = existing {
            transaction.commit().await?;
            return Ok(Registration::Registered {
                device,
                created: false,
            });
        }
        if exists {
            return Ok(Registration::Unauthorized);
        }

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
            return Ok(Registration::Exhausted);
        };
        let device = sqlx::query_as::<_, Device>(
            "INSERT INTO devices (id, wireguard_public_key, expires_at, address, bearer_token_hash)
             VALUES ($1, $2, statement_timestamp() + $3::bigint * INTERVAL '1 second', $4::text::inet, $5)
             RETURNING id, wireguard_public_key, created_at, expires_at, host(address) AS address",
        )
        .bind(Uuid::new_v4())
        .bind(public_key.as_slice())
        .bind(i64::from(self.lifetime_seconds.get()))
        .bind(address)
        .bind(new_credential_hash.as_slice())
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Registration::Registered {
            device,
            created: true,
        })
    }

    pub async fn set_remote_pairing_port(
        &self,
        device_id: Uuid,
        credential_hash: &[u8; 32],
        port: u16,
    ) -> Result<bool, sqlx::Error> {
        let updated = sqlx::query(
            "UPDATE devices
             SET remote_pairing_port = $3,
                 remote_pairing_port_expires_at = LEAST(
                     expires_at, clock_timestamp() + INTERVAL '5 minutes'
                 )
             WHERE id = $1 AND bearer_token_hash = $2
               AND expires_at > clock_timestamp()",
        )
        .bind(device_id)
        .bind(credential_hash.as_slice())
        .bind(i32::from(port))
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() != 0)
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
