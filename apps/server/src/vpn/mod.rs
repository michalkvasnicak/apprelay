mod command;
mod config;
mod macos;

use std::{io, net::Ipv4Addr};

use chrono::Utc;
use ipnet::Ipv4Net;

use base64::{Engine, engine::general_purpose::STANDARD};

pub use config::VpnConfig;

pub struct Vpn {
    interface: macos::Interface,
    public_key: String,
}

impl Vpn {
    pub async fn start(config: &VpnConfig) -> io::Result<Self> {
        let interface = macos::Interface::start(config).await?;
        let result = async {
            let name = interface
                .name()
                .ok_or_else(|| io::Error::other("WireGuard interface name is unavailable"))?;
            let output = command::output("wg", &["show", name, "public-key"]).await?;
            let public_key = String::from_utf8(output).map_err(io::Error::other)?;
            let public_key = public_key.trim().to_owned();
            let decoded = STANDARD.decode(&public_key).map_err(io::Error::other)?;
            if decoded.len() != 32 || decoded.iter().all(|byte| *byte == 0) {
                return Err(io::Error::other("invalid WireGuard server public key"));
            }
            Ok(public_key)
        }
        .await;
        match result {
            Ok(public_key) => Ok(Self {
                interface,
                public_key,
            }),
            Err(error) => {
                if let Err(cleanup) = interface.shutdown().await {
                    eprintln!("VPN startup cleanup failed: {cleanup}");
                }
                Err(error)
            }
        }
    }

    pub fn interface_name(&self) -> Option<&str> {
        self.interface.name()
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub async fn ensure_peer(
        &self,
        config: &VpnConfig,
        public_key: &[u8; 32],
        address: Ipv4Addr,
        expires_at: i64,
    ) -> io::Result<()> {
        let subnet = Ipv4Net::new(config.address, config.prefix_len).map_err(io::Error::other)?;
        if subnet.prefix_len() > 30
            || !subnet.contains(&address)
            || address == config.address
            || address == subnet.network()
            || address == subnet.broadcast()
            || *public_key == [0; 32]
            || STANDARD.encode(public_key) == self.public_key
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid VPN peer",
            ));
        }
        // Reject stale requests queued before cleanup, so an expired peer cannot
        // be re-added after its address has been released.
        if expires_at <= Utc::now().timestamp() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VPN lease expired",
            ));
        }
        let interface = self
            .interface_name()
            .ok_or_else(|| io::Error::other("WireGuard interface name is unavailable"))?;
        let public_key = STANDARD.encode(public_key);
        let allowed_ip = format!("{address}/32");
        command::run(
            "wg",
            &[
                "set",
                interface,
                "peer",
                &public_key,
                "allowed-ips",
                &allowed_ip,
            ],
            None,
        )
        .await
    }

    pub async fn remove_peer(&self, public_key: &[u8; 32]) -> io::Result<()> {
        let interface = self
            .interface_name()
            .ok_or_else(|| io::Error::other("WireGuard interface name is unavailable"))?;
        let public_key = STANDARD.encode(public_key);
        command::run(
            "wg",
            &["set", interface, "peer", &public_key, "remove"],
            None,
        )
        .await
    }

    pub async fn wait(&mut self) -> io::Result<()> {
        self.interface.wait().await
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.interface.shutdown().await
    }
}
