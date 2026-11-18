use std::{error::Error, net::Ipv4Addr, time::Duration};

use sqlx::PgPool;
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use crate::{agent::Client, db::devices::Device};

/// Restore active peers at startup and periodically, including when the agent
/// restarts independently of the API. Reapplying a peer is idempotent.
pub async fn run(pool: PgPool, agent: Client) {
    let mut ticks = interval(Duration::from_secs(60));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticks.tick().await;
        match reconcile(&pool, &agent).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "Reconciled active VPN peers"),
            Err(_) => {
                crate::telemetry::report("VPN peer reconciliation failed");
                tracing::warn!("VPN peer reconciliation failed; will retry");
            }
        }
    }
}

async fn reconcile(pool: &PgPool, agent: &Client) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let mut cursor: Option<Uuid> = None;
    let mut restored = 0;
    loop {
        let mut transaction = pool.begin().await?;
        // Walk by ID instead of loading all registrations into memory. Skip
        // busy rows so registration and cleanup are not blocked by this scan;
        // skipped rows are revisited on the next sweep.
        let device = sqlx::query_as::<_, Device>(
            "SELECT id, wireguard_public_key, created_at, expires_at, host(address) AS address
             FROM devices
             WHERE expires_at > statement_timestamp()
               AND ($1::uuid IS NULL OR id > $1)
             ORDER BY id
             LIMIT 1
             FOR UPDATE SKIP LOCKED",
        )
        .bind(cursor)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(device) = device else {
            transaction.commit().await?;
            return Ok(restored);
        };

        // Keep the row locked until provisioning completes. Cleanup cannot
        // remove the peer and release its address while we are restoring it.
        // The agent also rejects leases that expire while a request is queued.
        let public_key = device.wireguard_public_key.as_slice().try_into()?;
        let address = device.address.parse::<Ipv4Addr>()?;
        agent
            .ensure_peer(public_key, address, device.expires_at.timestamp())
            .await?;
        transaction.commit().await?;
        cursor = Some(device.id);
        restored += 1;
    }
}
