use std::{
    fs::{self, DirBuilder, File},
    io::{self, Read},
    net::Ipv4Addr,
    os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt},
    path::PathBuf,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{
    process::{Child, Command},
    time::{sleep, timeout},
};

use super::{command, config::VpnConfig};

pub struct Interface {
    child: Child,
    runtime: RuntimeDirectory,
    name: Option<String>,
}

impl Interface {
    pub async fn start(config: &VpnConfig) -> io::Result<Self> {
        let private_key = read_private_key(config)?;
        let runtime = RuntimeDirectory::create()?;
        let child = Command::new("wireguard-go")
            .args(["-f", "utun"])
            .env("WG_TUN_NAME_FILE", runtime.name_file())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not start wireguard-go: {error}"),
                )
            })?;
        let mut interface = Self {
            child,
            runtime,
            name: None,
        };

        if let Err(error) = interface.configure(config, &private_key).await {
            return match interface.shutdown().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::other(format!(
                    "{error}; VPN cleanup also failed: {cleanup}"
                ))),
            };
        }

        Ok(interface)
    }

    async fn configure(&mut self, config: &VpnConfig, private_key: &[u8]) -> io::Result<()> {
        timeout(Duration::from_secs(10), self.wait_until_ready())
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "wireguard-go startup timed out")
            })??;

        let name = self
            .name
            .as_deref()
            .ok_or_else(|| io::Error::other("WireGuard interface name is unavailable"))?;
        command::run(
            "wg",
            &[
                "set",
                name,
                "listen-port",
                &config.listen_port.to_string(),
                "private-key",
                "/dev/stdin",
            ],
            Some(private_key),
        )
        .await?;

        let mask = u32::MAX.checked_shl(32 - u32::from(config.prefix_len));
        let mask = mask.unwrap_or(0);
        let address = config.address.to_string();
        command::run(
            "/sbin/ifconfig",
            &[
                name,
                "inet",
                &address,
                &address,
                "netmask",
                &Ipv4Addr::from(mask).to_string(),
                "up",
            ],
            None,
        )
        .await?;

        let network = Ipv4Addr::from(u32::from(config.address) & mask);
        command::run(
            "/sbin/route",
            &[
                "-q",
                "-n",
                "add",
                "-inet",
                &format!("{network}/{}", config.prefix_len),
                "-interface",
                name,
            ],
            None,
        )
        .await
    }

    async fn wait_until_ready(&mut self) -> io::Result<()> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "wireguard-go exited during startup with {status}"
                )));
            }

            if self.name.is_none() {
                self.name = self.runtime.read_name()?;
            }

            if let Some(name) = &self.name {
                match fs::symlink_metadata(socket_path(name)) {
                    Ok(metadata) if metadata.file_type().is_socket() => return Ok(()),
                    Ok(_) => {
                        return Err(io::Error::other(
                            "WireGuard control socket path is not a socket",
                        ));
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }

            sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn wait(&mut self) -> io::Result<()> {
        let status = self.child.wait().await?;
        Err(io::Error::other(format!(
            "wireguard-go exited unexpectedly with {status}"
        )))
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }

        let name = match self.name.take() {
            Some(name) => Some(name),
            None => self.runtime.read_name()?,
        };
        if let Some(name) = name {
            match fs::remove_file(socket_path(&name)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }
}

fn read_private_key(config: &VpnConfig) -> io::Result<Vec<u8>> {
    let file = File::open(&config.private_key_file)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "VPN private key must be a regular file accessible only by its owner",
        ));
    }

    let mut key = Vec::new();
    file.take(128).read_to_end(&mut key)?;
    if key.is_empty() || key.len() >= 128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VPN private key file is empty or too large",
        ));
    }
    Ok(key)
}

fn socket_path(name: &str) -> PathBuf {
    PathBuf::from("/var/run/wireguard").join(format!("{name}.sock"))
}

struct RuntimeDirectory {
    path: PathBuf,
}

impl RuntimeDirectory {
    fn create() -> io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = PathBuf::from(format!(
            "/var/run/apprelay-wireguard-{}-{timestamp}",
            std::process::id()
        ));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self { path })
    }

    fn name_file(&self) -> PathBuf {
        self.path.join("interface")
    }

    fn read_name(&self) -> io::Result<Option<String>> {
        let name = match fs::read_to_string(self.name_file()) {
            Ok(name) => name.trim().to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let valid = name.strip_prefix("utun").is_some_and(|index| {
            !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
        });
        Ok(valid.then_some(name))
    }
}

impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.name_file());
        let _ = fs::remove_dir(&self.path);
    }
}
