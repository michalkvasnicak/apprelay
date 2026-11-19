use std::{env, future::Future, io, process::ExitCode, time::Duration};

use crate::error::OperationError;

/// Emit newline-delimited application events. Dependency logs are excluded
/// because their fields may contain SQL, payloads, or credentials.
pub fn init_logging() -> io::Result<()> {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::{filter::Targets, layer::SubscriberExt};

    let level = setting("APPRELAY_LOG_LEVEL")?
        .as_deref()
        .unwrap_or("info")
        .parse::<LevelFilter>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "APPRELAY_LOG_LEVEL must be off, error, warn, info, debug, or trace",
            )
        })?;
    let logger = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(io::stdout);
    let subscriber = tracing_subscriber::registry()
        .with(Targets::new().with_target("apprelay_server", level))
        .with(logger);
    tracing::subscriber::set_global_default(subscriber).map_err(io::Error::other)
}

/// Initialize before creating Tokio; retain the guard until after runtime shutdown.
pub fn init(service: &'static str) -> io::Result<Option<sentry::ClientInitGuard>> {
    // Sentry uses rustls-no-provider; select ring before any TLS clients are built.
    // An error only means a process-wide provider was already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    init_logging()?;

    let Some(dsn) = setting("SENTRY_DSN")? else {
        return Ok(None);
    };
    dsn.parse::<sentry::types::Dsn>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid SENTRY_DSN"))?;

    let mut options = sentry::ClientOptions::new()
        .dsn(&dsn)
        .send_default_pii(false)
        .max_request_body_size(sentry::MaxRequestBodySize::None)
        .max_breadcrumbs(0)
        .attach_stacktrace(true)
        .in_app_include(["apprelay_server"])
        .shutdown_timeout(Duration::from_secs(5))
        .before_send(|mut event| {
            // Never send credentials, protocol payloads, device identifiers, or
            // arbitrary panic messages (assertions may interpolate secrets).
            event.request = None;
            event.user = None;
            event.server_name = None;
            event.breadcrumbs = Default::default();
            event.extra.clear();
            for exception in &mut event.exception {
                exception.value = Some("Backend panic or unexpected error".to_owned());
            }
            Some(event)
        });
    if let Some(environment) = setting("SENTRY_ENVIRONMENT")? {
        options = options.environment(environment);
    }
    if let Some(release) = setting("SENTRY_RELEASE")? {
        options = options.release(release);
    }
    let guard = sentry::init(options);
    sentry::configure_scope(|scope| scope.set_tag("service", service));
    Ok(Some(guard))
}

/// Report a fixed diagnostic, never an error's Display/Debug or source chain.
pub fn report(message: &'static str) {
    sentry::capture_message(message, sentry::Level::Error);
}

/// Report only application-defined codes, never formatted dependency errors.
pub(crate) fn report_operation(
    message: &'static str,
    operation: &'static str,
    cause: &'static str,
) {
    sentry::with_scope(
        |scope| {
            scope.set_tag("operation", operation);
            scope.set_tag("cause", cause);
        },
        || report(message),
    );
}

/// Run an entry point while telemetry is active, including Tokio runtime creation.
pub fn run_service(
    service: &'static str,
    message: &'static str,
    mut runtime: tokio::runtime::Builder,
    run: impl Future<Output = Result<(), OperationError>>,
) -> ExitCode {
    let _sentry = match init(service) {
        Ok(guard) => guard,
        Err(_) => {
            // Telemetry may not have initialized; don't print its raw error.
            eprintln!("{service}: telemetry initialization failed");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime
        .enable_all()
        .build()
        .map_err(OperationError::at("build_tokio_runtime"))
        .and_then(|runtime| runtime.block_on(run));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let operation = error.operation;
            let cause = error.cause_code();
            report_operation(message, operation, cause);
            eprintln!("{service}: {message} (operation={operation}, cause={cause})");
            ExitCode::FAILURE
        }
    }
}

fn setting(name: &str) -> io::Result<Option<String>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must contain valid Unicode"),
        )),
    }
}
