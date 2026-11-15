mod app_key;
mod auth;
mod openapi;
mod pairing_codes;
mod push_tokens;
mod rate_limit;

pub use app_key::AppKey;
pub use openapi::document;
pub use rate_limit::RegistrationRateLimiter;

use std::{net::Ipv4Addr, time::Instant};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
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
};

#[derive(Clone)]
struct AppState {
    devices: DeviceRepository,
    agent: Client,
    push_tokens: PushTokenRepository,
    codes: PairingCodeRepository,
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

pub fn router(
    devices: DeviceRepository,
    agent: Client,
    push_tokens: PushTokenRepository,
    codes: PairingCodeRepository,
    app_key: AppKey,
    registration_limiter: RegistrationRateLimiter,
) -> Router {
    Router::new()
        .route(
            "/v1/devices",
            post(register_device).layer(middleware::from_fn_with_state(
                registration_limiter,
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
        .route_layer(middleware::from_fn_with_state(
            app_key,
            app_key::require_app_key,
        ))
        .route("/health", get(health))
        .route("/openapi.json", get(|| async { Json(document()) }))
        .layer(DefaultBodyLimit::max(1024))
        .layer(middleware::from_fn(log_request))
        .with_state(AppState {
            devices,
            agent,
            push_tokens,
            codes,
        })
}

async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    eprintln!(
        "HTTP {method} {path:?} -> {} ({} ms)",
        response.status().as_u16(),
        started.elapsed().as_millis(),
    );
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
        (status = 503, description = "Provisioning unavailable", body = ErrorResponse)
    ),
    security(("app_key" = []))
)]
async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<RegistrationRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = auth::Credential::optional(&headers)?;
    let Json(request) = request.map_err(|error| ApiError {
        status: error.status(),
        message: "invalid registration JSON; expected wireguard_public_key",
    })?;
    let invalid_key = || ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "wireguard_public_key must be a nonzero, base64-encoded 32-byte key",
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

    let status = state.agent.status().await.map_err(ApiError::agent)?;
    if status.state != VpnState::Running {
        return Err(ApiError::unavailable("VPN is not running"));
    }
    let server_public_key = status
        .public_key
        .ok_or_else(|| ApiError::unavailable("VPN public key is unavailable"))?;
    if request.wireguard_public_key == server_public_key {
        return Err(invalid_key());
    }
    let subnet = Ipv4Net::new(status.address, status.prefix_len)
        .map_err(|_| ApiError::unavailable("invalid VPN subnet"))?;
    if subnet.prefix_len() > 30
        || subnet.addr() == subnet.network()
        || subnet.addr() == subnet.broadcast()
    {
        return Err(ApiError::unavailable(
            "VPN subnet has no usable device pool",
        ));
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
        .map_err(ApiError::database)?
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
            return Err(ApiError::unavailable("VPN address pool is exhausted"));
        }
    };
    // The reservation above is already committed. If the agent times out or
    // this request is cancelled, cleanup must remove the peer before IP reuse.
    let locked = state
        .devices
        .lock_for_provisioning(device.id)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(|| ApiError::unavailable("registration expired; retry registration"))?;
    let address = locked
        .device
        .address
        .parse::<Ipv4Addr>()
        .map_err(|_| ApiError::unavailable("device address is invalid"))?;
    if !subnet.contains(&address)
        || address == subnet.addr()
        || address == subnet.network()
        || address == subnet.broadcast()
    {
        return Err(ApiError::unavailable(
            "reserved address does not match the VPN subnet",
        ));
    }
    let configured = state
        .agent
        .ensure_peer(public_key, address, locked.device.expires_at.timestamp())
        .await
        .map_err(ApiError::agent)?;
    if configured.state != VpnState::Running
        || configured.address != status.address
        || configured.prefix_len != status.prefix_len
        || configured.public_key.as_deref() != Some(server_public_key.as_str())
    {
        return Err(ApiError::unavailable(
            "VPN configuration changed; retry registration",
        ));
    }
    let device = locked.commit().await.map_err(ApiError::database)?;
    let lifetime_seconds = device.expires_at.timestamp() - Utc::now().timestamp();
    if lifetime_seconds <= 0 {
        return Err(ApiError::unavailable(
            "registration expired; retry registration",
        ));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Err(error) = crate::temporal_pairing::start(device.id).await {
        // Registration remains valid even when the pairing queue is unavailable.
        eprintln!("Could not queue pairing for device {}: {error}", device.id);
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
struct ErrorResponse {
    error: &'static str,
}

struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "a valid device bearer token is required",
        }
    }

    fn unavailable(message: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message,
        }
    }

    fn database(_: sqlx::Error) -> Self {
        eprintln!("Device registration database operation failed");
        Self::unavailable("device database is unavailable")
    }

    fn agent(error: std::io::Error) -> Self {
        eprintln!("Device registration VPN agent error: {error}");
        Self::unavailable("VPN provisioning is unavailable; retry registration")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}
