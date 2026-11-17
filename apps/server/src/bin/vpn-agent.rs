#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> std::io::Result<()> {
    let _sentry = apprelay_server::telemetry::init("vpn-agent")?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(apprelay_server::agent::run())
        .inspect_err(|_| apprelay_server::telemetry::report("VPN agent terminated with an error"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "VPN agent requires macOS or Linux",
    ))
}
