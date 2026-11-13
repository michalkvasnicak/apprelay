mod command;
mod config;
mod macos;

use std::io;

use base64::{Engine, engine::general_purpose::STANDARD};

pub use config::VpnConfig;

pub struct Vpn {
    interface: macos::Interface,
}

impl Vpn {
    pub async fn start(config: &VpnConfig) -> io::Result<Self> {
        Ok(Self {
            interface: macos::Interface::start(config).await?,
        })
    }

    pub fn interface_name(&self) -> Option<&str> {
        self.interface.name()
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
