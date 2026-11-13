#[cfg(target_os = "macos")]
mod vpn;

#[cfg(target_os = "macos")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    use vpn::{Vpn, VpnConfig};

    let config = VpnConfig::from_env()?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut vpn = Vpn::start(&config).await?;

    eprintln!(
        "WireGuard ready at {}/{} on UDP port {}",
        config.address, config.prefix_len, config.listen_port
    );

    let outcome = tokio::select! {
        _ = interrupt.recv() => Ok(()),
        _ = terminate.recv() => Ok(()),
        result = vpn.wait() => result,
    };
    match (outcome, vpn.shutdown().await) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(std::io::Error::other(format!(
            "{error}; VPN cleanup also failed: {cleanup}"
        ))),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "VPN management is currently implemented only for macOS",
    ))
}
