use std::{
    env,
    fs::File,
    io::{self, Read},
    net::Ipv4Addr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

use zeroize::Zeroizing;

pub struct VpnConfig {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub listen_port: u16,
    pub private_key_file: PathBuf,
}

impl VpnConfig {
    pub(super) fn read_private_key(&self) -> io::Result<Zeroizing<Vec<u8>>> {
        let file = File::open(&self.private_key_file)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "VPN private key must be a regular file accessible only by its owner",
            ));
        }

        let mut key = Zeroizing::new(Vec::new());
        file.take(128).read_to_end(&mut key)?;
        if key.is_empty() || key.len() >= 128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VPN private key file is empty or too large",
            ));
        }
        Ok(key)
    }

    pub fn from_env() -> io::Result<Self> {
        let address = setting("APPRELAY_VPN_ADDRESS", "10.77.0.1/24")?;
        let (address, prefix) = address.split_once('/').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "APPRELAY_VPN_ADDRESS must be an IPv4 address with a prefix length",
            )
        })?;
        let address = address
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid VPN IPv4 address"))?;
        let prefix_len = prefix.parse::<u8>().ok().filter(|prefix| *prefix <= 32);
        let prefix_len = prefix_len.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid VPN prefix length")
        })?;
        let listen_port = setting("APPRELAY_VPN_PORT", "51820")?
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid VPN listen port")
            })?;
        let private_key_file = env::var_os("APPRELAY_VPN_PRIVATE_KEY_FILE")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "APPRELAY_VPN_PRIVATE_KEY_FILE is required",
                )
            })?;

        Ok(Self {
            address,
            prefix_len,
            listen_port,
            private_key_file,
        })
    }
}

fn setting(name: &str, default: &str) -> io::Result<String> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be valid Unicode"),
        )),
    }
}
