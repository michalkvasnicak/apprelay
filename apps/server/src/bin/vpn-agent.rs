#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> std::process::ExitCode {
    apprelay_server::telemetry::run_service(
        "vpn-agent",
        "VPN agent terminated with an error",
        tokio::runtime::Builder::new_current_thread(),
        async {
            apprelay_server::agent::run()
                .await
                .map_err(apprelay_server::error::OperationError::at("run_service"))
        },
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "VPN agent requires macOS or Linux",
    ))
}
