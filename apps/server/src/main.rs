#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    use apprelay_server::agent::Client;
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let agent = Client::from_env()?;
    let status = agent.status().await?;
    eprintln!("Connected to VPN agent: {status:?}");

    tokio::select! {
        _ = interrupt.recv() => {},
        _ = terminate.recv() => {},
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the server requires Unix-domain socket support",
    ))
}
