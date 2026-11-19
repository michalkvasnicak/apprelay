mod auth;
mod middleware;
mod responses;
mod routes;

pub use middleware::{app_key::AppKey, rate_limit::RegistrationRateLimiter};
pub use routes::{artifacts::ArtifactApi, https_proxy::HttpsProxy, openapi::document};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post, put},
};

use crate::{
    db::{
        devices::DeviceRepository, pairing_codes::PairingCodeRepository,
        push_tokens::PushTokenRepository,
    },
    vpn::agent::Client,
};

use self::{
    middleware::{app_key, rate_limit, request_logging},
    routes::{
        artifacts, devices, health, https_proxy, install_jobs, openapi, pairing_attempts,
        pairing_codes, push_tokens, remote_pairing, web,
    },
};

#[derive(Clone)]
struct AppState {
    devices: DeviceRepository,
    agent: Client,
    push_tokens: PushTokenRepository,
    codes: PairingCodeRepository,
    attempts: crate::db::pairing_attempts::PairingAttemptRepository,
}

pub struct RouterConfig {
    pub app_key: AppKey,
    pub registration_limiter: RegistrationRateLimiter,
    pub https_proxy: HttpsProxy,
    pub artifacts: Option<ArtifactApi>,
}

pub fn router(
    devices: DeviceRepository,
    agent: Client,
    push_tokens: PushTokenRepository,
    codes: PairingCodeRepository,
    attempts: crate::db::pairing_attempts::PairingAttemptRepository,
    config: RouterConfig,
) -> Router {
    let RouterConfig {
        app_key,
        registration_limiter,
        https_proxy,
        artifacts,
    } = config;
    Router::new()
        .route(
            "/v1/devices",
            post(devices::register_device).layer(from_fn_with_state(
                registration_limiter.clone(),
                rate_limit::limit_registration,
            )),
        )
        .route(
            "/v1/devices/{device_id}/push-token",
            put(push_tokens::put).delete(push_tokens::delete),
        )
        .route(
            "/v1/devices/{device_id}/pairing-code",
            get(pairing_codes::get),
        )
        .route(
            "/v1/devices/{device_id}/pairing",
            post(pairing_attempts::start),
        )
        .route(
            "/v1/devices/{device_id}/remote-pairing",
            put(remote_pairing::put),
        )
        .route_layer(from_fn_with_state(app_key, app_key::require_app_key))
        .merge(
            Router::new()
                .route("/v1/pairing-attempts", post(pairing_attempts::create))
                .route("/v1/pairing-attempts/{token}", get(pairing_attempts::get))
                .route_layer(from_fn_with_state(
                    registration_limiter.clone(),
                    rate_limit::limit_pairing_attempts,
                )),
        )
        .merge(
            Router::new()
                .route("/v1/install-jobs", post(install_jobs::create))
                .route("/v1/install-jobs/{job_id}", get(install_jobs::get))
                .route(
                    "/v1/install-jobs/{job_id}/complete",
                    post(install_jobs::complete),
                )
                .route_layer(from_fn_with_state(
                    registration_limiter.clone(),
                    rate_limit::limit_install_jobs,
                ))
                .with_state(artifacts.clone()),
        )
        .merge(artifacts::router(artifacts, registration_limiter))
        .merge(https_proxy::router(https_proxy))
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi::get_document))
        .merge(web::router())
        .layer(DefaultBodyLimit::max(1024))
        .layer(from_fn(request_logging::log_request))
        .layer(sentry::integrations::tower::NewSentryLayer::<Request>::new_from_top())
        .with_state(AppState {
            devices,
            agent,
            push_tokens,
            codes,
            attempts,
        })
}
