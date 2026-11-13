#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    apprelay_server::temporal_notifications::run_worker().await
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the notification worker currently requires the macOS Temporal worker runtime",
    ))
}
