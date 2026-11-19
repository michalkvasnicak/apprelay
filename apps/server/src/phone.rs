//! Authenticated connections to paired iPhones and access to their services.

use std::{net::Ipv4Addr, time::Duration};

use idevice::{
    IdeviceError, RsdService,
    remote_pairing::{RemotePairingClient, RpPairingFile, connect_tls_psk_tunnel_native},
    rsd::RsdHandshake,
    tcp::{adapter::Adapter, handle::AdapterHandle},
};
use tokio::{net::TcpStream, time::timeout};

use crate::{
    error::OperationError,
    remote_pairing_identity::{IdentitySocket, Transcript},
};

pub(crate) struct VerifiedPhoneSession {
    rsd: RsdHandshake,
    handle: AdapterHandle,
    // Keep the control connection alive while using the tunnel it opened.
    _control: RemotePairingClient<IdentitySocket>,
}

impl VerifiedPhoneSession {
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

/// Confirm saved pairing credentials without opening a service tunnel.
pub(crate) async fn verify_saved_pairing(
    address: Ipv4Addr,
    port: u16,
    record: &mut RpPairingFile,
    peer_key: &[u8; 32],
) -> Result<(), OperationError> {
    connect_verified_pairing(address, port, record, peer_key).await?;
    Ok(())
}

async fn dial_device(address: Ipv4Addr, port: u16) -> Result<TcpStream, OperationError> {
    if port == 0 {
        return Err(OperationError::without_source("invalid-tunnel-port"));
    }
    timeout(Duration::from_secs(10), TcpStream::connect((address, port)))
        .await
        .map_err(OperationError::at("device-connect-timeout"))?
        .map_err(OperationError::at("device-connect"))
}

async fn connect_verified_pairing(
    address: Ipv4Addr,
    port: u16,
    record: &mut RpPairingFile,
    peer_key: &[u8; 32],
) -> Result<RemotePairingClient<IdentitySocket>, OperationError> {
    let transcript = Transcript::default();
    let stream = dial_device(address, port).await?;
    let mut control = RemotePairingClient::new(
        IdentitySocket::new(stream, transcript.clone(), false),
        "AppRelay",
    );
    control
        .attempt_pair_verify()
        .await
        .map_err(OperationError::at("pairing-handshake"))?;
    control
        .validate_pairing(record)
        .await
        .map_err(OperationError::at("pairing-verification"))?;
    transcript
        .verify(control.encryption_key(), peer_key)
        .map_err(OperationError::at("peer-identity-verification"))?;
    Ok(control)
}
