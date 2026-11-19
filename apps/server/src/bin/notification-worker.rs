#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> std::process::ExitCode {
    apprelay_server::telemetry::run_service(
        "notification-worker",
        "notification-worker terminated with an error",
        tokio::runtime::Builder::new_multi_thread(),
        apprelay_server::temporal_notifications::run_worker(),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the notification worker requires macOS or Linux",
    ))
}
