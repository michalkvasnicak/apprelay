//! Device-specific pairing sessions using unicast DNS-SD announcements. Never registers a system Bonjour service.

use std::{
    ffi::CString,
    io,
    net::{Ipv4Addr, SocketAddr},
    os::fd::AsRawFd,
    time::Duration,
};

use hickory_proto::{
    op::{Message, MessageType},
    rr::{
        Name, RData, Record,
        rdata::{A, PTR, SRV, TXT},
    },
};
use idevice::remote_pairing::{
    PAIRABLE_HOST_SERVICE_TYPE, PairableHost, PairableHostInfo, RpPairingFile, RpPairingSocket,
};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    net::{TcpListener, UdpSocket},
    time::interval,
};

use crate::agent::VpnStatus;

pub(crate) struct Session {
    listener: TcpListener,
    discovery: UdpSocket,
    peer: Ipv4Addr,
    announcement: Vec<u8>,
    goodbye: Vec<u8>,
    host_info: PairableHostInfo,
    pairing_file: RpPairingFile,
}

impl Session {
    pub(crate) async fn new(status: &VpnStatus, peer: Ipv4Addr) -> io::Result<Self> {
        let interface = status
            .interface
            .as_deref()
            .ok_or_else(|| io::Error::other("pairing requires a running VPN"))?;
        let listener = TcpListener::bind((status.address, 0)).await?;
        bind_interface(&listener, interface)?;
        // Use a dedicated announcement port, separate from system mDNS (5353).
        // Bind only the VPN IP and interface; never publish via system Bonjour.
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        bind_interface(&socket, interface)?;
        socket.bind(&SocketAddr::from((status.address, 5354)).into())?;
        socket.set_nonblocking(true)?;
        let discovery = UdpSocket::from_std(socket.into())?;
        discovery.set_ttl(255)?;

        let host_info = PairableHostInfo::generate("AppRelay", "Mac17,7");
        let mut pairing_file = RpPairingFile::generate(&host_info.name);
        pairing_file.identifier = uuid::Uuid::new_v4().to_string();
        let port = listener.local_addr()?.port();
        let goodbye = announcement(&host_info, &pairing_file, status.address, port, 0)?;
        let announcement = announcement(&host_info, &pairing_file, status.address, port, 10)?;
        eprintln!(
            "Pairing: VPN interface {interface}, TCP {}, unicast mDNS only to {peer}:5354. Identity is ephemeral.",
            listener.local_addr()?
        );
        Ok(Self {
            listener,
            discovery,
            peer,
            announcement,
            goodbye,
            host_info,
            pairing_file,
        })
    }

    pub(crate) async fn stop(&self) {
        if let Err(error) = self
            .discovery
            .send_to(&self.goodbye, (self.peer, 5354))
            .await
        {
            eprintln!(
                "Could not withdraw pairing advertisement for {}: {error}",
                self.peer
            );
        }
    }

    pub(crate) async fn run(&mut self) -> io::Result<()> {
        let mut announcements = interval(Duration::from_secs(5));
        let destination = SocketAddr::from((self.peer, 5354));
        let mut receiving = [0u8; 1500];
        loop {
            tokio::select! {
                _ = announcements.tick() => {
                    self.discovery.send_to(&self.announcement, destination).await?;
                    eprintln!("Pairing: unicast mDNS announcement sent to {destination}");
                }
                received = self.discovery.recv_from(&mut receiving) => {
                    let (length, source) = received?;
                    if source != destination {
                        continue;
                    }
                    if let Ok(message) = Message::from_vec(&receiving[..length])
                        && message.message_type() == MessageType::Query
                    {
                        eprintln!("Pairing: DNS query received from {source}");
                        self.discovery.send_to(&self.announcement, destination).await?;
                    }
                }
                accepted = self.listener.accept() => {
                    let (stream, source) = accepted?;
                    if source.ip() != self.peer {
                        eprintln!("Pairing: rejecting TCP connection from {source}");
                        continue;
                    }
                    eprintln!("Pairing: selected iPhone connected from {source}");
                    let mut host = PairableHost::new(
                        RpPairingSocket::new_device(stream), self.host_info.clone());
                    // Keep announcements running while the user enters the PIN.
                    let pairing = host.accept(
                        &mut self.pairing_file,
                        |pin| async move { eprintln!("Pairing: enter {pin} on the iPhone"); },
                    );
                    tokio::pin!(pairing);
                    let result = loop {
                        tokio::select! {
                            result = &mut pairing => break result,
                            _ = announcements.tick() => {
                                self.discovery.send_to(&self.announcement, destination).await?;
                            }
                        }
                    };
                    match result {
                        Ok(peer) => {
                            eprintln!("Pair-setup response sent: {} ({}, UDID {}). iOS completion is not confirmed; pairing keys are not yet persisted.",
                                peer.name, peer.model, peer.remotepairing_udid);
                            return Ok(());
                        },
                        Err(error) => eprintln!("Pairing failed: {error}"),
                    }
                }
            }
        }
    }
}

fn announcement(
    host: &PairableHostInfo,
    pairing: &RpPairingFile,
    address: Ipv4Addr,
    port: u16,
    ttl: u32,
) -> io::Result<Vec<u8>> {
    let service = Name::from_ascii(PAIRABLE_HOST_SERVICE_TYPE).map_err(io::Error::other)?;
    let instance = Name::from_ascii(format!(
        "{}.{}",
        pairing.identifier, PAIRABLE_HOST_SERVICE_TYPE
    ))
    .map_err(io::Error::other)?;
    let hostname = Name::from_ascii(format!("apprelay-{}.local.", pairing.identifier))
        .map_err(io::Error::other)?;
    let txt = host
        .mdns_txt_records(&pairing.identifier)
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    let mut message = Message::new();
    message
        .set_message_type(MessageType::Response)
        .set_authoritative(true);
    // Short TTL: stale entries expire quickly when the server/VPN stops. These
    // are targeted unsolicited responses, not multicast or wide-area DNS-SD.
    message.add_answer(Record::from_rdata(
        service,
        ttl,
        RData::PTR(PTR(instance.clone())),
    ));
    for mut record in [
        Record::from_rdata(
            instance.clone(),
            ttl,
            RData::SRV(SRV::new(0, 0, port, hostname.clone())),
        ),
        Record::from_rdata(instance, ttl, RData::TXT(TXT::new(txt))),
        Record::from_rdata(hostname, ttl, RData::A(A(address))),
    ] {
        record.set_mdns_cache_flush(true);
        message.add_additional(record);
    }
    message.to_vec().map_err(io::Error::other)
}

fn bind_interface(socket: &impl AsRawFd, interface: &str) -> io::Result<()> {
    let interface = CString::new(interface).map_err(io::Error::other)?;
    // IP_BOUND_IF prevents route changes from sending announcements over LAN.
    let index = unsafe { libc::if_nametoindex(interface.as_ptr()) };
    if index == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            (&index as *const libc::c_uint).cast(),
            std::mem::size_of_val(&index) as libc::socklen_t,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
