mod command;
mod config;
mod macos;

use std::io;

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

    pub async fn wait(&mut self) -> io::Result<()> {
        self.interface.wait().await
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.interface.shutdown().await
    }
}
