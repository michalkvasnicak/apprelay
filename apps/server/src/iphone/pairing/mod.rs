//! Establish and verify iPhone Remote Pairing credentials.

use std::{future::Future, io, net::Ipv4Addr, time::Duration};

use idevice::{
    IdeviceError,
    remote_pairing::{
        PairableHost, PairableHostInfo, PeerDevice, RemotePairingClient, RpPairingFile,
    },
};
use tokio::{
    net::TcpStream,
    time::{Interval, interval, timeout},
};

use crate::{agent::VpnStatus, error::OperationError};

mod discovery;
mod identity;

use self::{
    discovery::Discovery,
    identity::{IdentitySocket, Transcript},
};

/// Credentials stay inside the activity and are never a Temporal payload.
pub(crate) struct PairingRecord {
    pub pairing_file: RpPairingFile,
    pub host_info: PairableHostInfo,
    pub peer: PeerDevice,
    pub peer_public_key: [u8; 32],
}

pub(crate) struct SetupSession {
    discovery: Discovery,
    host_info: PairableHostInfo,
    pairing_file: RpPairingFile,
}

impl SetupSession {
    pub(crate) async fn new(status: &VpnStatus, peer: Ipv4Addr) -> io::Result<Self> {
        let interface = status
            .interface
            .as_deref()
            .ok_or_else(|| io::Error::other("pairing requires a running VPN"))?;
        let host_info = PairableHostInfo::generate("AppRelay", "Mac17,7");
        let mut pairing_file = RpPairingFile::generate(&host_info.name);
        pairing_file.identifier = uuid::Uuid::new_v4().to_string();
        let discovery =
            Discovery::new(interface, status.address, peer, &host_info, &pairing_file).await?;
        Ok(Self {
            discovery,
            host_info,
            pairing_file,
        })
    }

    pub(crate) async fn stop(&self) {
        self.discovery.stop().await;
    }

    pub(crate) async fn run<F, Fut>(
        &mut self,
        mut on_code: F,
    ) -> Result<PairingRecord, OperationError>
    where
        F: FnMut(Option<String>) -> Fut + Send,
        Fut: Future<Output = Result<(), OperationError>> + Send,
    {
        let mut announcements = interval(Duration::from_secs(5));
        loop {
            let stream = self.discovery.accept(&mut announcements).await?;
            let transcript = Transcript::default();
            let mut host = PairableHost::new(
                IdentitySocket::new(stream, transcript.clone(), true),
                self.host_info.clone(),
            );
            let result = self
                .exchange(&mut host, &mut announcements, &mut on_code)
                .await?;
            // Neither failed exchanges nor completed pairing retain a usable code.
            on_code(None).await?;
            match result {
                Ok(peer) => {
                    let peer_public_key = transcript
                        .setup_public_key(host.encryption_key())
                        .map_err(OperationError::at("pair-setup-peer-identity"))?;
                    tracing::info!("Pair-setup response sent; iOS completion is not yet confirmed");
                    return Ok(PairingRecord {
                        pairing_file: self.pairing_file.clone(),
                        host_info: self.host_info.clone(),
                        peer,
                        peer_public_key,
                    });
                }
                Err(_) => tracing::warn!("Pairing exchange failed"),
            }
        }
    }

    async fn exchange<F, Fut>(
        &mut self,
        host: &mut PairableHost<IdentitySocket>,
        announcements: &mut Interval,
        on_code: &mut F,
    ) -> Result<Result<PeerDevice, IdeviceError>, OperationError>
    where
        F: FnMut(Option<String>) -> Fut + Send,
        Fut: Future<Output = Result<(), OperationError>> + Send,
    {
        let (pin_sender, mut pin_receiver) = tokio::sync::oneshot::channel();
        let (resume_sender, resume_receiver) = tokio::sync::oneshot::channel();
        let mut resume_sender = Some(resume_sender);
        let mut pin_received = false;
        // Pause the handshake until the PIN is durably published.
        let pairing = host.accept(&mut self.pairing_file, |pin| async move {
            let _ = pin_sender.send(pin);
            let _ = resume_receiver.await;
        });
        tokio::pin!(pairing);
        loop {
            tokio::select! {
                result = &mut pairing => return Ok(result),
                pin = &mut pin_receiver, if !pin_received => {
                    pin_received = true;
                    if let Ok(pin) = pin {
                        let publication = on_code(Some(pin));
                        tokio::pin!(publication);
                        loop {
                            tokio::select! {
                                result = &mut publication => { result?; break; },
                                _ = announcements.tick() => self.discovery.announce().await?,
                            }
                        }
                        if let Some(sender) = resume_sender.take() {
                            let _ = sender.send(());
                        }
                    }
                }
                _ = announcements.tick() => self.discovery.announce().await?,
            }
        }
    }
}

/// A control connection whose peer proved possession of the key learned at setup.
pub(super) struct VerifiedPairing(RemotePairingClient<IdentitySocket>);

impl VerifiedPairing {
    pub(super) async fn create_tcp_listener(&mut self) -> Result<u16, IdeviceError> {
        self.0.create_tcp_listener().await
    }

    pub(super) fn encryption_key(&self) -> &[u8] {
        self.0.encryption_key()
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

pub(super) async fn dial_device(address: Ipv4Addr, port: u16) -> Result<TcpStream, OperationError> {
    if port == 0 {
        return Err(OperationError::without_source("invalid-tunnel-port"));
    }
    timeout(Duration::from_secs(10), TcpStream::connect((address, port)))
        .await
        .map_err(OperationError::at("device-connect-timeout"))?
        .map_err(OperationError::at("device-connect"))
}

pub(super) async fn connect_verified_pairing(
    address: Ipv4Addr,
    port: u16,
    record: &mut RpPairingFile,
    peer_key: &[u8; 32],
) -> Result<VerifiedPairing, OperationError> {
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
    Ok(VerifiedPairing(control))
}
