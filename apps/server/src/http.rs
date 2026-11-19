mod app_key;
mod artifacts;
mod auth;
mod https_proxy;
mod install_jobs;
mod openapi;
mod pairing_attempts;
mod pairing_codes;
mod push_tokens;
mod rate_limit;
mod remote_pairing;
mod web;

pub use app_key::AppKey;
pub use artifacts::ArtifactApi;
pub use https_proxy::HttpsProxy;
pub use openapi::document;
pub use rate_limit::RegistrationRateLimiter;

use std::{net::Ipv4Addr, sync::Arc, time::Instant};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, MatchedPath, Request, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    agent::{Client, VpnState},
    db::{
        devices::{DeviceRepository, Registration},
        pairing_codes::PairingCodeRepository,
        push_tokens::PushTokenRepository,
    },
    error::OperationError,
};

#[derive(Clone)]
struct AppState {
    devices: DeviceRepository,
    agent: Client,
    push_tokens: PushTokenRepository,
    codes: PairingCodeRepository,
    attempts: crate::db::pairing_attempts::PairingAttemptRepository,
}

#[derive(Serialize, ToSchema)]
struct Health {
    status: &'static str,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct RegistrationRequest {
    wireguard_public_key: String,
}

#[derive(Serialize, ToSchema)]
struct RegistrationResponse {
    device_id: Uuid,
    token: String,
    address: String,
    expires_at: DateTime<Utc>,
    lifetime_seconds: i64,
    wireguard: WireguardConfiguration,
}

#[derive(Serialize, ToSchema)]
struct WireguardConfiguration {
    server_public_key: String,
    allowed_ips: [String; 1],
    persistent_keepalive_seconds: u16,
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
            post(register_device).layer(middleware::from_fn_with_state(
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
        .route_layer(middleware::from_fn_with_state(
            app_key,
            app_key::require_app_key,
        ))
        .merge(
            Router::new()
                .route("/v1/pairing-attempts", post(pairing_attempts::create))
                .route("/v1/pairing-attempts/{token}", get(pairing_attempts::get))
                .route_layer(middleware::from_fn_with_state(
                    registration_limiter.clone(),
                    rate_limit::limit_pairing_attempts,
                )),
        )
        .merge(
            Router::new()
                .route("/v1/install-jobs", post(install_jobs::create))
                .route(
                    "/v1/install-jobs/{job_id}/complete",
                    post(install_jobs::complete),
                )
                .route_layer(middleware::from_fn_with_state(
                    registration_limiter.clone(),
                    rate_limit::limit_install_jobs,
                ))
                .with_state(artifacts.clone()),
        )
        .merge(artifacts::router(artifacts, registration_limiter))
        .merge(https_proxy::router(https_proxy))
        .route("/health", get(health))
        .route("/openapi.json", get(|| async { Json(document()) }))
        .merge(web::router())
        .layer(DefaultBodyLimit::max(1024))
        .layer(middleware::from_fn(log_request))
        .layer(sentry::integrations::tower::NewSentryLayer::<Request>::new_from_top())
        .with_state(AppState {
            devices,
            agent,
            push_tokens,
            codes,
            attempts,
        })
}

async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    // Matched route patterns contain placeholders, never capability tokens/IDs.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    sentry::configure_scope(|scope| {
        scope.set_tag("http.method", method.as_str());
        scope.set_tag("http.route", &route);
    });
    let started = Instant::now();
    let mut response = next.run(request).await;
    if response.status().is_server_error() {
        let error = response.extensions_mut().remove::<Arc<OperationError>>();
        let status = response.status();
        sentry::with_scope(
            |scope| {
                scope.set_tag("http.status_code", status.as_str());
                let mut fingerprint =
                    vec!["http", method.as_str(), route.as_str(), status.as_str()];
                if let Some(error) = &error {
                    fingerprint.push(error.operation);
                }
                scope.set_fingerprint(Some(&fingerprint));
            },
            || {
                if let Some(error) = &error {
                    crate::telemetry::report_operation(
                        "HTTP request failed",
                        error.operation,
                        error.cause_code(),
                    );
                } else {
                    crate::telemetry::report("HTTP request failed");
                }
            },
        );
        if status == StatusCode::INTERNAL_SERVER_ERROR && error.is_none() {
            // Normalize framework/service 500s that bypass ApiError as well.
            response = internal_error_response();
        }
    }
    response
        .headers_mut()
        .entry(CACHE_CONTROL)
        .or_insert(axum::http::HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    let status = response.status();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if status.is_server_error() {
        tracing::error!(
            method = method.as_str(),
            route = route.as_str(),
            status = status.as_u16(),
            elapsed_ms,
            "HTTP request completed"
        );
    } else {
        tracing::info!(
            method = method.as_str(),
            route = route.as_str(),
            status = status.as_u16(),
            elapsed_ms,
            "HTTP request completed"
        );
    }
    response
}

#[utoipa::path(get, path = "/health", responses((status = 200, body = Health)))]
async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

#[utoipa::path(
    post,
    path = "/v1/devices",
    request_body = RegistrationRequest,
    responses(
        (status = 201, description = "Device registered", body = RegistrationResponse),
        (status = 400, description = "Invalid registration", body = ErrorResponse),
        (status = 401, description = "Invalid app key or device credential", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Expected application/json", body = ErrorResponse),
        (status = 422, description = "Invalid JSON fields", body = ErrorResponse),
        (status = 429, description = "Rate limited; see Retry-After", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    security(("app_key" = []))
)]
async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<RegistrationRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = auth::Credential::optional(&headers)?;
    let Json(request) = request.map_err(|error| ApiError::Public {
        status: error.status(),
        body: ErrorResponse::InvalidRequest {
            error: "invalid registration JSON; expected wireguard_public_key",
        },
    })?;
    let invalid_key = || ApiError::Public {
        status: StatusCode::BAD_REQUEST,
        body: ErrorResponse::InvalidRequest {
            error: "wireguard_public_key must be a nonzero, base64-encoded 32-byte key",
        },
    };
    if request.wireguard_public_key.len() != 44 {
        return Err(invalid_key());
    }
    let decoded = STANDARD
        .decode(&request.wireguard_public_key)
        .map_err(|_| invalid_key())?;
    let public_key: [u8; 32] = decoded.try_into().map_err(|_| invalid_key())?;
    if public_key == [0; 32] {
        return Err(invalid_key());
    }

    let status = state
        .agent
        .status()
        .await
        .map_err(ApiError::internal("read_vpn_agent_status"))?;
    if status.state != VpnState::Running {
        return Err(ApiError::internal_without_source("vpn_not_running"));
    }
    let server_public_key = status
        .public_key
        .ok_or_else(|| ApiError::internal_without_source("vpn_public_key_missing"))?;
    if request.wireguard_public_key == server_public_key {
        return Err(invalid_key());
    }
    let subnet = Ipv4Net::new(status.address, status.prefix_len)
        .map_err(ApiError::internal("parse_vpn_subnet"))?;
    if subnet.prefix_len() > 30
        || subnet.addr() == subnet.network()
        || subnet.addr() == subnet.broadcast()
    {
        return Err(ApiError::internal_without_source("unusable_vpn_subnet"));
    }

    let new_credential = auth::Credential::generate()?;
    let (device, token) = match state
        .devices
        .register(
            &public_key,
            subnet,
            credential.as_ref().map(|credential| &credential.hash),
            &new_credential.hash,
        )
        .await
        .map_err(ApiError::Internal)?
    {
        Registration::Registered { device, created } => {
            let token = if created {
                new_credential.token
            } else {
                credential.ok_or_else(ApiError::unauthorized)?.token
            };
            (device, token)
        }
        Registration::Unauthorized => return Err(ApiError::unauthorized()),
        Registration::Exhausted => {
            return Err(ApiError::internal_without_source(
                "vpn_address_pool_exhausted",
            ));
        }
    };
    // The reservation above is already committed. If the agent times out or
    // this request is cancelled, cleanup must remove the peer before IP reuse.
    let locked = state
        .devices
        .lock_for_provisioning(device.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::internal_without_source("registration_expired"))?;
    let address = locked
        .device
        .address
        .parse::<Ipv4Addr>()
        .map_err(OperationError::at("parse_reserved_device_address"))
        .map_err(ApiError::Internal)?;
    if !subnet.contains(&address)
        || address == subnet.addr()
        || address == subnet.network()
        || address == subnet.broadcast()
    {
        return Err(ApiError::internal_without_source(
            "reserved_address_outside_vpn_pool",
        ));
    }
    let configured = state
        .agent
        .ensure_peer(public_key, address, locked.device.expires_at.timestamp())
        .await
        .map_err(ApiError::internal("provision_vpn_peer"))?;
    if configured.state != VpnState::Running
        || configured.address != status.address
        || configured.prefix_len != status.prefix_len
        || configured.public_key.as_deref() != Some(server_public_key.as_str())
    {
        return Err(ApiError::internal_without_source(
            "vpn_configuration_changed",
        ));
    }
    let device = locked.commit().await.map_err(ApiError::Internal)?;
    let lifetime_seconds = device.expires_at.timestamp() - Utc::now().timestamp();
    if lifetime_seconds <= 0 {
        return Err(ApiError::internal_without_source("registration_expired"));
    }
    Ok((
        StatusCode::CREATED,
        [(CACHE_CONTROL, "no-store")],
        Json(RegistrationResponse {
            device_id: device.id,
            token,
            address: format!("{address}/32"),
            expires_at: device.expires_at,
            lifetime_seconds,
            wireguard: WireguardConfiguration {
                server_public_key,
                allowed_ips: [format!("{}/{}", subnet.network(), subnet.prefix_len())],
                persistent_keepalive_seconds: 25,
            },
        }),
    ))
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ErrorResponse {
    InvalidRequest { error: &'static str },
    AppKeyRequired { error: &'static str },
    DeviceTokenRequired { error: &'static str },
    PairingTokenRequired { error: &'static str },
    InternalServerError { error: &'static str },
    NotFound { error: &'static str },
    Conflict { error: &'static str },
    Expired { error: &'static str },
    RateLimited { error: &'static str },
    UpstreamFailure { error: &'static str },
    UpstreamTimeout { error: &'static str },
}

enum ApiError {
    Public {
        status: StatusCode,
        body: ErrorResponse,
    },
    Internal(OperationError),
}

impl ApiError {
    fn unauthorized() -> Self {
        Self::Public {
            status: StatusCode::UNAUTHORIZED,
            body: ErrorResponse::DeviceTokenRequired {
                error: "a valid device bearer token is required",
            },
        }
    }

    fn internal<E>(operation: &'static str) -> impl FnOnce(E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        move |error| Self::Internal(OperationError::at(operation)(error))
    }

    fn internal_without_source(operation: &'static str) -> Self {
        Self::Internal(OperationError::without_source(operation))
    }
}

fn internal_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::InternalServerError {
            error: "internal server error",
        }),
    )
        .into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Public { status, body } => (status, Json(body)).into_response(),
            Self::Internal(error) => {
                let mut response = internal_error_response();
                // Carry the source to the HTTP boundary, not into the response body.
                response.extensions_mut().insert(Arc::new(error));
                response
            }
        }
    }
}
