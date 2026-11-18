#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut arguments = std::env::args().skip(1);
    let device_id = arguments.next().ok_or("usage: pair-device <device-id>")?;
    if arguments.next().is_some() {
        return Err("usage: pair-device <device-id>".into());
    }
    apprelay_server::telemetry::init_logging()?;
    apprelay_server::temporal_pairing::start(uuid::Uuid::parse_str(&device_id)?).await
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "pairing workflows require macOS or Linux",
    ))
}
