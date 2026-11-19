#[cfg(unix)]
use apprelay_server::error::OperationError;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    apprelay_server::telemetry::run_service(
        "server",
        "HTTP server terminated with an error",
        tokio::runtime::Builder::new_current_thread(),
        run(),
    )
}

#[cfg(unix)]
async fn run() -> Result<(), OperationError> {
    use apprelay_server::{
        cleanup, db,
        db::{
            devices::DeviceRepository, pairing_codes::PairingCodeRepository,
            push_tokens::PushTokenRepository,
        },
        http,
        vpn::{agent::Client, reconciliation},
    };
    use tokio::{
        net::TcpListener,
        signal::unix::{SignalKind, signal},
    };

    let app_key = http::AppKey::from_env().map_err(OperationError::at("configure_app_key"))?;
    let https_proxy =
        http::HttpsProxy::from_env().map_err(OperationError::at("configure_https_proxy"))?;
    let registration_limiter = http::RegistrationRateLimiter::from_env()
        .await
        .map_err(OperationError::at("configure_registration_rate_limit"))?;
    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(OperationError::at("register_sigint"))?;
    let mut terminate =
        signal(SignalKind::terminate()).map_err(OperationError::at("register_sigterm"))?;
    let lifetime_seconds = std::env::var("APPRELAY_DEVICE_LIFETIME_SECONDS")
        .or_else(|error| match error {
            std::env::VarError::NotPresent => Ok("604800".to_owned()),
            error => Err(error),
        })
        .map_err(OperationError::at("read_device_lifetime"))?
        .parse::<std::num::NonZeroU32>()
        .map_err(OperationError::at("parse_device_lifetime"))?;
    let database_url =
        std::env::var("DATABASE_URL").map_err(OperationError::at("read_database_url"))?;
    let pool = db::connect(&database_url)
        .await
        .map_err(OperationError::at("connect_database"))?;
    let push_tokens = PushTokenRepository::from_env(pool.clone())
        .map_err(OperationError::at("configure_push_token_repository"))?;
    let codes = PairingCodeRepository::from_env(pool.clone())?;
    db::MIGRATOR
        .run(&pool)
        .await
        .map_err(OperationError::at("migrate_database"))?;
    let devices = DeviceRepository::new(pool.clone(), lifetime_seconds);
    let artifacts = http::ArtifactApi::from_env(pool.clone())
        .map_err(OperationError::at("configure_artifact_api"))?;

    let agent = Client::from_env().map_err(OperationError::at("configure_vpn_agent"))?;
    match agent.status().await {
        Ok(_) => tracing::info!("Connected to VPN agent"),
        Err(error) => tracing::warn!(
            error_kind = ?error.kind(),
            "VPN agent unavailable; HTTP liveness is independent"
        ),
    }

    let address = std::env::var("APPRELAY_HTTP_ADDRESS")
        .or_else(|error| match error {
            std::env::VarError::NotPresent => Ok("127.0.0.1:8080".to_owned()),
            error => Err(error),
        })
        .map_err(OperationError::at("read_http_address"))?;
    let listener = TcpListener::bind(&address)
        .await
        .map_err(OperationError::at("bind_http_listener"))?;
    let listen_address = listener
        .local_addr()
        .map_err(OperationError::at("read_bound_http_address"))?;
    tracing::info!(%listen_address, "HTTP server listening");

    let cleanup = tokio::spawn(cleanup::run(pool.clone(), agent.clone()));
    let install_cleanup = artifacts
        .clone()
        .map(|api| tokio::spawn(api.clean_install_uploads()));
    let attempts = db::pairing_attempts::PairingAttemptRepository::new(pool);
    let vpn_reconciliation = tokio::spawn(reconciliation::run(devices.clone(), agent.clone()));
    let result = axum::serve(
        listener,
        http::router(
            devices,
            agent,
            push_tokens,
            codes,
            attempts,
            http::RouterConfig {
                app_key,
                registration_limiter,
                https_proxy,
                artifacts,
            },
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
    if let Some(install_cleanup) = install_cleanup {
        install_cleanup.abort();
        let _ = install_cleanup.await;
    }
    cleanup.abort();
    vpn_reconciliation.abort();
    let _ = cleanup.await;
    let _ = vpn_reconciliation.await;
    result.map_err(OperationError::at("serve_http"))
}

#[cfg(not(unix))]
fn main() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the server requires Unix-domain socket support",
    ))
}
