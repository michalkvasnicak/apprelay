use std::{env, io, time::Duration};

/// Initialize before creating Tokio; retain the guard until after runtime shutdown.
pub fn init(service: &'static str) -> io::Result<Option<sentry::ClientInitGuard>> {
    // Sentry uses rustls-no-provider; select ring before any TLS clients are built.
    // An error only means a process-wide provider was already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

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
