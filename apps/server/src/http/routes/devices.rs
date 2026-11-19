use std::net::Ipv4Addr;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    db::devices::Registration,
    error::OperationError,
    http::{
        AppState,
        auth::Credential,
        responses::{ApiError, ErrorResponse},
    },
    vpn::agent::VpnState,
};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct RegistrationRequest {
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

#[utoipa::path(
    post,
    path = "/v1/devices",
    // Preserve the OpenAPI tag from before the handler moved into this module.
    tag = "super",
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
pub(in crate::http) async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<RegistrationRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = Credential::optional(&headers)?;
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

    let new_credential = Credential::generate()?;
    let (device, token) = match state
        .devices
        .register(
            &public_key,
            subnet,
            credential.as_ref().map(|credential| &credential.hash),
            &new_credential.hash,
        )
        .await
        .map_err(|error| ApiError::Internal(error.into()))?
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
        .map_err(|error| ApiError::Internal(error.into()))?
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
    let device = locked
        .commit()
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
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
