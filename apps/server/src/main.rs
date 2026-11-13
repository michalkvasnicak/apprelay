#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    use apprelay_server::{agent::Client, http};
    use tokio::{
        net::TcpListener,
        signal::unix::{SignalKind, signal},
    };

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let agent = Client::from_env()?;
    match agent.status().await {
        Ok(status) => eprintln!("Connected to VPN agent: {status:?}"),
        Err(error) => eprintln!("VPN agent unavailable (HTTP liveness is independent): {error}"),
    }

    let address = std::env::var("APPRELAY_HTTP_ADDRESS")
        .or_else(|error| match error {
            std::env::VarError::NotPresent => Ok("127.0.0.1:8080".to_owned()),
            error => Err(error),
        })
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let listener = TcpListener::bind(&address).await?;
    eprintln!("HTTP server listening on {}", listener.local_addr()?);

    axum::serve(listener, http::router())
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await
}

#[cfg(not(unix))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the server requires Unix-domain socket support",
    ))
}
