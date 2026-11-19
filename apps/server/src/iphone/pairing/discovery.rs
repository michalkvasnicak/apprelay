//! VPN-bound unicast DNS-SD; never registers a system Bonjour service.

use std::{
    ffi::CString,
    io,
    net::{Ipv4Addr, SocketAddr},
    os::fd::AsRawFd,
};

use hickory_proto::{
    op::{Message, MessageType},
    rr::{
        Name, RData, Record,
        rdata::{A, PTR, SRV, TXT},
    },
};
use idevice::remote_pairing::{PAIRABLE_HOST_SERVICE_TYPE, PairableHostInfo, RpPairingFile};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    time::Interval,
};

use crate::error::OperationError;

pub(super) struct Discovery {
    listener: TcpListener,
    socket: UdpSocket,
    destination: SocketAddr,
    announcement: Vec<u8>,
    goodbye: Vec<u8>,
}

impl Discovery {
    pub(super) async fn new(
        interface: &str,
        address: Ipv4Addr,
        peer: Ipv4Addr,
        host: &PairableHostInfo,
        pairing: &RpPairingFile,
    ) -> io::Result<Self> {
        let listener = bind_pairing_listener(address).await?;
        bind_interface(&listener, interface)?;
        // Use a dedicated announcement port, separate from system mDNS (5353).
        // Bind only the VPN IP and interface; never publish via system Bonjour.
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        bind_interface(&socket, interface)?;
        socket.bind(&SocketAddr::from((address, 5354)).into())?;
        socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(socket.into())?;
        socket.set_ttl(255)?;

        let port = listener.local_addr()?.port();
        let goodbye = announcement(host, pairing, address, port, 0)?;
        let announcement = announcement(host, pairing, address, port, 10)?;
        tracing::debug!(interface, port, "Pairing listener ready");
        Ok(Self {
            listener,
            socket,
            destination: SocketAddr::from((peer, 5354)),
            announcement,
            goodbye,
        })
    }

    pub(super) async fn announce(&self) -> Result<(), OperationError> {
        self.socket
            .send_to(&self.announcement, self.destination)
            .await
            .map_err(OperationError::at("send_pairing_announcement"))?;
        Ok(())
    }

    pub(super) async fn stop(&self) {
        if let Err(error) = self.socket.send_to(&self.goodbye, self.destination).await {
            tracing::warn!(error_kind = ?error.kind(), "Could not withdraw pairing advertisement");
        }
    }

    pub(super) async fn accept(
        &self,
        announcements: &mut Interval,
    ) -> Result<TcpStream, OperationError> {
        let mut receiving = [0u8; 1500];
        loop {
            tokio::select! {
                _ = announcements.tick() => {
                    self.announce().await?;
                    tracing::debug!("Pairing mDNS announcement sent");
                }
                received = self.socket.recv_from(&mut receiving) => {
                    let (length, source) = received.map_err(OperationError::at("receive_pairing_query"))?;
                    if source != self.destination {
                        continue;
                    }
                    if let Ok(message) = Message::from_vec(&receiving[..length])
                        && message.message_type() == MessageType::Query
                    {
                        tracing::debug!("Pairing DNS query received");
                        self.announce().await?;
                    }
                }
                accepted = self.listener.accept() => {
                    let (stream, source) = accepted.map_err(OperationError::at("accept_pairing_connection"))?;
                    if source.ip() != self.destination.ip() {
                        tracing::debug!("Rejected pairing connection from unexpected peer");
                        continue;
                    }
                    tracing::debug!("Selected iPhone connected for pairing");
                    return Ok(stream);
                }
            }
        }
    }
}

// Keep this dedicated pool in sync with the container's VPN firewall rules.
// It is outside the container's outbound ephemeral range (32768–60999).
async fn bind_pairing_listener(address: Ipv4Addr) -> io::Result<TcpListener> {
    for port in 20000..=20049 {
        match TcpListener::bind((address, port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "all 50 pairing listener ports are in use",
    ))
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

#[cfg(target_os = "linux")]
fn bind_interface(socket: &impl AsRawFd, interface: &str) -> io::Result<()> {
    let interface = CString::new(interface).map_err(io::Error::other)?;
    // Fail closed if binding fails: announcements must never escape over LAN.
    // Setting SO_BINDTODEVICE requires CAP_NET_RAW on Linux.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            interface.as_ptr().cast(),
            interface.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!(
                "could not bind pairing socket to VPN interface (CAP_NET_RAW required): {error}"
            ),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
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
