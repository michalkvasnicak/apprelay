#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut arguments = std::env::args().skip(1);
    let device_id = arguments.next().ok_or("usage: pair-device <device-id>")?;
    if arguments.next().is_some() {
        return Err("usage: pair-device <device-id>".into());
    }
    apprelay_server::temporal_pairing::start(uuid::Uuid::parse_str(&device_id)?).await
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "pairing workflows currently require the macOS VPN agent",
    ))
}
