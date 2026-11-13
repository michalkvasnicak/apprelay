#[cfg(target_os = "macos")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    apprelay_server::agent::run().await
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "VPN agent is currently implemented only for macOS",
    ))
}
