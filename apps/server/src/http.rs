use std::net::Ipv4Addr;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    agent::{Client, VpnState},
    db::devices::DeviceRepository,
};

#[derive(Clone)]
struct AppState {
    devices: DeviceRepository,
    agent: Client,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationRequest {
    wireguard_public_key: String,
}

#[derive(Serialize)]
struct RegistrationResponse {
    device_id: Uuid,
    address: String,
    expires_at: DateTime<Utc>,
    lifetime_seconds: i64,
    wireguard: WireguardConfiguration,
}

#[derive(Serialize)]
struct WireguardConfiguration {
    server_public_key: String,
    allowed_ips: [String; 1],
    persistent_keepalive_seconds: u16,
}

pub fn router(devices: DeviceRepository, agent: Client) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/devices", post(register_device))
        .layer(DefaultBodyLimit::max(1024))
        .with_state(AppState { devices, agent })
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn register_device(
    State(state): State<AppState>,
    request: Result<Json<RegistrationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RegistrationResponse>), ApiError> {
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

    let device = state
        .devices
        .register(&public_key, subnet)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(|| ApiError::unavailable("VPN address pool is exhausted"))?;
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
        .as_deref()
        .and_then(|address| address.parse::<Ipv4Addr>().ok())
        .ok_or_else(|| ApiError::unavailable("device address is unavailable"))?;
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
    Ok((
        StatusCode::CREATED,
        Json(RegistrationResponse {
            device_id: device.id,
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

struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn unavailable(message: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message,
        }
    }

    fn database(error: sqlx::Error) -> Self {
        eprintln!("Device registration database error: {error}");
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
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}
