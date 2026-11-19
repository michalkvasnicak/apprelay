use std::{error::Error, net::Ipv4Addr, time::Duration};

use tokio::time::{MissedTickBehavior, interval};

use super::agent::Client;
use crate::db::devices::DeviceRepository;

/// Restore active peers at startup and periodically, including when the agent
/// restarts independently of the API. Reapplying a peer is idempotent.
pub async fn run(devices: DeviceRepository, agent: Client) {
    let mut ticks = interval(Duration::from_secs(60));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticks.tick().await;
        match reconcile(&devices, &agent).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "Reconciled active VPN peers"),
            Err(_) => {
                crate::telemetry::report("VPN peer reconciliation failed");
                tracing::warn!("VPN peer reconciliation failed; will retry");
            }
        }
    }
}

async fn reconcile(
    devices: &DeviceRepository,
    agent: &Client,
) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let mut cursor = None;
    let mut restored = 0;
    while let Some(locked) = devices.lock_next_active_for_provisioning(cursor).await? {
        let device = &locked.device;

        // Keep the row locked until provisioning completes. Cleanup cannot
        // remove the peer and release its address while we are restoring it.
        // The agent also rejects leases that expire while a request is queued.
        let public_key = device.wireguard_public_key.as_slice().try_into()?;
        let address = device.address.parse::<Ipv4Addr>()?;
        agent
            .ensure_peer(public_key, address, device.expires_at.timestamp())
            .await?;
        cursor = Some(locked.commit().await?.id);
        restored += 1;
    }
    Ok(restored)
}
