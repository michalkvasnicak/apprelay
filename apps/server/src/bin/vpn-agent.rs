#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    apprelay_server::agent::run().await
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "VPN agent requires macOS or Linux",
    ))
}
