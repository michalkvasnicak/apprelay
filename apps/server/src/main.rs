#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    use apprelay_server::{
        agent::Client,
        cleanup, db,
        db::{
            devices::DeviceRepository, pairing_codes::PairingCodeRepository,
            push_tokens::PushTokenRepository,
        },
        http, reconciliation,
    };
    use tokio::{
        net::TcpListener,
        signal::unix::{SignalKind, signal},
    };

    let app_key = http::AppKey::from_env()?;
    let https_proxy = http::HttpsProxy::new()?;
    let registration_limiter = http::RegistrationRateLimiter::from_env().await?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let lifetime_seconds = std::env::var("APPRELAY_DEVICE_LIFETIME_SECONDS")
        .or_else(|error| match error {
            std::env::VarError::NotPresent => Ok("604800".to_owned()),
            error => Err(error),
        })
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?
        .parse::<std::num::NonZeroU32>()
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "APPRELAY_DEVICE_LIFETIME_SECONDS must be a positive 32-bit integer: {error}"
                ),
            )
        })?;
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let pool = db::connect(&database_url)
        .await
        .map_err(std::io::Error::other)?;
    let push_tokens = PushTokenRepository::from_env(pool.clone())?;
    let codes = PairingCodeRepository::from_env(pool.clone())?;
    db::MIGRATOR
        .run(&pool)
        .await
        .map_err(std::io::Error::other)?;
    let devices = DeviceRepository::new(pool.clone(), lifetime_seconds);

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

    let cleanup = tokio::spawn(cleanup::run(pool.clone(), agent.clone()));
    let attempts = db::pairing_attempts::PairingAttemptRepository::new(pool.clone());
    let reconciliation = tokio::spawn(reconciliation::run(pool, agent.clone()));
    let result = axum::serve(
        listener,
        http::router(
            devices,
            agent,
            push_tokens,
            codes,
            app_key,
            registration_limiter,
            attempts,
            https_proxy,
        )
        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::select! {
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
        }
    })
    .await;
    cleanup.abort();
    reconciliation.abort();
    let _ = cleanup.await;
    let _ = reconciliation.await;
    result
}

#[cfg(not(unix))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the server requires Unix-domain socket support",
    ))
}
