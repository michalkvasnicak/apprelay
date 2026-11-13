use std::{io, time::Duration};

use tokio::time::{Interval, MissedTickBehavior, interval};

use super::{command, config::VpnConfig};

// A dedicated name avoids taking over an administrator-managed wg0.
const INTERFACE: &str = "wg-apprelay";

pub struct Interface {
    checks: Interval,
}

impl Interface {
    pub async fn start(config: &VpnConfig) -> io::Result<Self> {
        let private_key = config.read_private_key()?;
        // Never adopt or delete an existing interface, even if its name matches.
        command::run(
            "ip",
            &["link", "add", "dev", INTERFACE, "type", "wireguard"],
            None,
        )
        .await?;
        let mut checks = interval(Duration::from_secs(5));
        checks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let interface = Self { checks };
        let result = async {
            command::run(
                "wg",
                &[
                    "set",
                    INTERFACE,
                    "listen-port",
                    &config.listen_port.to_string(),
                    "private-key",
                    "/dev/stdin",
                ],
                Some(&private_key),
            )
            .await?;
            // Linux installs the connected subnet route when the address is added.
            command::run(
                "ip",
                &[
                    "address",
                    "add",
                    &format!("{}/{}", config.address, config.prefix_len),
                    "dev",
                    INTERFACE,
                ],
                None,
            )
            .await?;
            command::run("ip", &["link", "set", "dev", INTERFACE, "up"], None).await
        }
        .await;

        match result {
            Ok(()) => Ok(interface),
            Err(error) => match interface.shutdown().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::other(format!(
                    "{error}; VPN cleanup also failed: {cleanup}"
                ))),
            },
        }
    }

    pub fn name(&self) -> Option<&str> {
        Some(INTERFACE)
    }

    pub async fn wait(&mut self) -> io::Result<()> {
        // Kernel WireGuard has no child process. Check the control interface so
        // external deletion or loss of access fails the agent instead of leaving
        // it reporting a healthy VPN indefinitely.
        loop {
            self.checks.tick().await;
            command::output("wg", &["show", INTERFACE, "public-key"]).await?;
        }
    }

    pub async fn shutdown(self) -> io::Result<()> {
        command::run("ip", &["link", "delete", "dev", INTERFACE], None).await
    }
}
