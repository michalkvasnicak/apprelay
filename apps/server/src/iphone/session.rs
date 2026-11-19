//! Authenticated service connections to paired iPhones.

use std::net::Ipv4Addr;

use idevice::{
    IdeviceError, RsdService,
    remote_pairing::{RpPairingFile, connect_tls_psk_tunnel_native},
    rsd::RsdHandshake,
    tcp::{adapter::Adapter, handle::AdapterHandle},
};

use super::pairing::{VerifiedPairing, connect_verified_pairing, dial_device};
use crate::error::OperationError;

pub(crate) struct IphoneSession {
    rsd: RsdHandshake,
    handle: AdapterHandle,
    // Keep the control connection alive while using the tunnel it opened.
    _control: VerifiedPairing,
}

impl IphoneSession {
    pub(crate) async fn connect(
        address: Ipv4Addr,
        port: u16,
        record: &mut RpPairingFile,
        peer_key: &[u8; 32],
    ) -> Result<Self, OperationError> {
        let mut control = connect_verified_pairing(address, port, record, peer_key).await?;
        let port = control
            .create_tcp_listener()
            .await
            .map_err(OperationError::at("tunnel-listener"))?;
        let stream = dial_device(address, port).await?;
        let tunnel = connect_tls_psk_tunnel_native(stream, control.encryption_key())
            .await
            .map_err(OperationError::at("tunnel-handshake"))?;
        let client_ip = tunnel
            .info
            .client_address
            .parse()
            .map_err(OperationError::at("tunnel-client-address"))?;
        let server_ip = tunnel
            .info
            .server_address
            .parse()
            .map_err(OperationError::at("tunnel-server-address"))?;
        let port = tunnel.info.server_rsd_port;
        let mtu = usize::from(tunnel.info.mtu);
        if port == 0 || !(1280..=16_000).contains(&mtu) {
            return Err(OperationError::without_source("tunnel-parameters"));
        }
        let mut adapter = Adapter::new(Box::new(tunnel.into_inner()), client_ip, server_ip);
        adapter.set_mss(mtu - 60);
        let mut handle = adapter.to_async_handle();
        let rsd_stream = handle
            .connect(port)
            .await
            .map_err(OperationError::at("rsd-connect"))?;
        let rsd = RsdHandshake::new(rsd_stream)
            .await
            .map_err(OperationError::at("rsd-handshake"))?;
        Ok(Self {
            rsd,
            handle,
            _control: control,
        })
    }

    pub(crate) async fn connect_service<T: RsdService>(&mut self) -> Result<T, IdeviceError> {
        self.rsd.connect(&mut self.handle).await
    }
}
