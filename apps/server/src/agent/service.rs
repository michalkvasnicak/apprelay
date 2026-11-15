use std::{env, future::pending, io, time::Duration};

use tokio::{
    net::{UnixListener, UnixStream},
    signal::unix::{Signal, SignalKind, signal},
    time::timeout,
};

use super::{
    protocol::{self, Request, Response, VpnState, VpnStatus},
    socket::Socket,
    socket_path,
};
use crate::vpn::{Vpn, VpnConfig};

pub async fn run() -> io::Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "vpn-agent must run as root",
        ));
    }

    let client_uid = client_uid()?;
    let config = VpnConfig::from_env()?;
    let path = socket_path()?;
    let socket = Socket::bind(&path, client_uid)?;
    let interrupt = signal(SignalKind::interrupt())?;
    let terminate = signal(SignalKind::terminate())?;
    let mut vpn = Some(Vpn::start(&config).await?);
    eprintln!(
        "VPN agent ready at {} for client UID {client_uid}",
        path.display()
    );

    let outcome = serve(
        &socket.listener,
        client_uid,
        &config,
        &mut vpn,
        interrupt,
        terminate,
    )
    .await;
    let cleanup = match vpn {
        Some(vpn) => vpn.shutdown().await,
        None => Ok(()),
    };
    match (outcome, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(io::Error::other(format!(
            "{error}; VPN cleanup also failed: {cleanup}"
        ))),
    }
}

fn client_uid() -> io::Result<u32> {
    env::var_os("APPRELAY_VPN_AGENT_UID")
        .or_else(|| env::var_os("SUDO_UID"))
        .and_then(|value| value.to_str().and_then(|value| value.parse::<u32>().ok()))
        .filter(|uid| *uid != 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "APPRELAY_VPN_AGENT_UID (or SUDO_UID) must identify the non-root server user",
            )
        })
}

async fn serve(
    listener: &UnixListener,
    client_uid: u32,
    config: &VpnConfig,
    vpn: &mut Option<Vpn>,
    mut interrupt: Signal,
    mut terminate: Signal,
) -> io::Result<()> {
    loop {
        let (mut stream, _) = tokio::select! {
            _ = interrupt.recv() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
            result = wait_for_vpn(vpn) => return result,
            connection = listener.accept() => connection?,
        };
        match stream.peer_cred() {
            Ok(credentials) if credentials.uid() == client_uid => {}
            _ => continue,
        }

        if let Err(error) = handle(&mut stream, config, vpn).await {
            eprintln!("VPN agent request failed: {error}");
        }
    }
}

async fn wait_for_vpn(vpn: &mut Option<Vpn>) -> io::Result<()> {
    match vpn {
        Some(vpn) => vpn.wait().await,
        None => pending().await,
    }
}

async fn handle(
    stream: &mut UnixStream,
    config: &VpnConfig,
    vpn: &mut Option<Vpn>,
) -> io::Result<()> {
    let request = timeout(Duration::from_secs(5), protocol::receive(stream))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VPN agent read timed out"))??;
    let result = match request {
        Request::Status => Ok(()),
        Request::RequireConnectedPeer { public_key } => match vpn.as_ref() {
            Some(vpn) => vpn.require_connected_peer(&public_key).await,
            None => Err(io::Error::other("VPN is stopped")),
        },
        Request::Start => {
            if vpn.is_none() {
                match Vpn::start(config).await {
                    Ok(started) => {
                        *vpn = Some(started);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            } else {
                Ok(())
            }
        }
        Request::EnsurePeer {
            public_key,
            address,
            expires_at,
        } => match vpn.as_ref() {
            Some(vpn) => {
                vpn.ensure_peer(config, &public_key, address, expires_at)
                    .await
            }
            None => Err(io::Error::other("VPN is stopped")),
        },
        Request::RemovePeer { public_key } => match vpn.as_ref() {
            Some(vpn) => vpn.remove_peer(&public_key).await,
            None => Ok(()),
        },
        Request::Stop => match vpn.take() {
            Some(vpn) => vpn.shutdown().await,
            None => Ok(()),
        },
    };
    let response = match result {
        Ok(()) => Response::Status {
            status: status(config, vpn.as_ref()),
        },
        Err(error) => Response::Error {
            message: error.to_string(),
        },
    };
    timeout(Duration::from_secs(5), protocol::send(stream, &response))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VPN agent write timed out"))?
}

fn status(config: &VpnConfig, vpn: Option<&Vpn>) -> VpnStatus {
    VpnStatus {
        state: if vpn.is_some() {
            VpnState::Running
        } else {
            VpnState::Stopped
        },
        interface: vpn.and_then(|vpn| vpn.interface_name().map(str::to_owned)),
        address: config.address,
        prefix_len: config.prefix_len,
        listen_port: config.listen_port,
        public_key: vpn.map(|vpn| vpn.public_key().to_owned()),
    }
}
