use std::{io, net::Ipv4Addr};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_MESSAGE_SIZE: usize = 4096;

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Status,
    RequireConnectedPeer {
        public_key: [u8; 32],
    },
    Start,
    Stop,
    RemovePeer {
        public_key: [u8; 32],
    },
    EnsurePeer {
        public_key: [u8; 32],
        address: Ipv4Addr,
        expires_at: i64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Response {
    Status { status: VpnStatus },
    Error { message: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VpnStatus {
    pub state: VpnState,
    pub interface: Option<String>,
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub listen_port: u16,
    pub public_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VpnState {
    Running,
    Stopped,
}

pub(super) async fn receive<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> io::Result<T> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid VPN agent message size",
        ));
    }
    let mut message = vec![0; length];
    stream.read_exact(&mut message).await?;
    serde_json::from_slice(&message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) async fn send<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &T,
) -> io::Result<()> {
    let message = serde_json::to_vec(message).map_err(io::Error::other)?;
    if message.is_empty() || message.len() > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid VPN agent message size",
        ));
    }
    stream.write_u32(message.len() as u32).await?;
    stream.write_all(&message).await?;
    stream.flush().await
}
