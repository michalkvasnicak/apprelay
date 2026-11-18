#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _sentry = apprelay_server::telemetry::init("install-worker")?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(apprelay_server::temporal_install::run_worker())
        .inspect_err(|_| {
            apprelay_server::telemetry::report("install-worker terminated with an error")
        })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the install worker requires macOS or Linux",
    ))
}
