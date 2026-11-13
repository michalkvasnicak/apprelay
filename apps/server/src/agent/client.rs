use std::{io, net::Ipv4Addr, path::PathBuf, time::Duration};

use tokio::{net::UnixStream, time::timeout};

use super::{
    protocol::{self, Request, Response, VpnStatus},
    socket_path,
};

#[derive(Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn from_env() -> io::Result<Self> {
        Ok(Self {
            socket: socket_path()?,
        })
    }

    pub async fn status(&self) -> io::Result<VpnStatus> {
        self.request(Request::Status).await
    }

    pub async fn start(&self) -> io::Result<VpnStatus> {
        self.request(Request::Start).await
    }

    pub async fn stop(&self) -> io::Result<VpnStatus> {
        self.request(Request::Stop).await
    }

    pub async fn ensure_peer(
        &self,
        public_key: [u8; 32],
        address: Ipv4Addr,
        expires_at: i64,
    ) -> io::Result<VpnStatus> {
        self.request(Request::EnsurePeer {
            public_key,
            address,
            expires_at,
        })
        .await
    }

    /// Removing an already absent peer succeeds, making retries safe.
    pub async fn remove_peer(&self, public_key: [u8; 32]) -> io::Result<()> {
        self.request(Request::RemovePeer { public_key })
            .await
            .map(|_| ())
    }

    async fn request(&self, request: Request) -> io::Result<VpnStatus> {
        timeout(Duration::from_secs(30), async {
            let mut stream = UnixStream::connect(&self.socket).await.map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "could not connect to VPN agent at {}: {error}",
                        self.socket.display()
                    ),
                )
            })?;
            if stream.peer_cred()?.uid() != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "VPN agent must be owned by root",
                ));
            }

            protocol::send(&mut stream, &request).await?;
            match protocol::receive(&mut stream).await? {
                Response::Status { status } => Ok(status),
                Response::Error { message } => Err(io::Error::other(message)),
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "VPN agent request timed out"))?
    }
}
