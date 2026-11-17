mod client;
mod protocol;

#[cfg(feature = "test-support")]
pub mod testing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod service;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod socket;

use std::{env, io, path::PathBuf};

pub use client::Client;
pub use protocol::{VpnState, VpnStatus};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use service::run;

fn socket_path() -> io::Result<PathBuf> {
    let path = env::var_os("APPRELAY_VPN_AGENT_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/var/db/apprelay/vpn-agent.sock"
            } else {
                "/var/run/apprelay/vpn-agent.sock"
            })
        });
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "APPRELAY_VPN_AGENT_SOCKET must be an absolute socket file path",
        ));
    }
    Ok(path)
}
