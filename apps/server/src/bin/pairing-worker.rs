#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> std::process::ExitCode {
    apprelay_server::telemetry::run_service(
        "pairing-worker",
        "pairing-worker terminated with an error",
        tokio::runtime::Builder::new_multi_thread(),
        async {
            apprelay_server::temporal_pairing::run_worker()
                .await
                .map_err(apprelay_server::error::OperationError::at("run_service"))
        },
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the pairing worker requires macOS or Linux",
    ))
}
