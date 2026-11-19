use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ApiError, AppState, ErrorResponse, auth::Credential};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PushTokenRequest {
    token: String,
}

#[utoipa::path(
    put,
    path = "/v1/devices/{device_id}/push-token",
    params(("device_id" = Uuid, Path, description = "Registered device ID")),
    request_body = PushTokenRequest,
    responses(
        (status = 204, description = "Push token saved"),
        (status = 400, description = "Invalid device ID or token"),
        (status = 401, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("app_key" = [], "device_token" = []))
)]
pub(super) async fn put(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<Uuid>,
    request: Result<Json<PushTokenRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(request) = request.map_err(|error| ApiError::Public {
        status: error.status(),
        body: super::ErrorResponse::InvalidRequest {
            error: "invalid push token JSON; expected token",
        },
    })?;
    let token = decode_token(&request.token).ok_or(ApiError::Public {
        status: StatusCode::BAD_REQUEST,
        body: super::ErrorResponse::InvalidRequest {
            error: "token must be a nonempty, even-length hexadecimal string",
        },
    })?;
    update(&state, &headers, device_id, Some(&token)).await
}

#[utoipa::path(
    delete,
    path = "/v1/devices/{device_id}/push-token",
    params(("device_id" = Uuid, Path, description = "Registered device ID")),
    responses(
        (status = 204, description = "Push token removed"),
        (status = 400, description = "Invalid device ID"),
        (status = 401, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("app_key" = [], "device_token" = []))
)]
pub(super) async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    update(&state, &headers, device_id, None).await
}

fn decode_token(token: &str) -> Option<Vec<u8>> {
    if token.is_empty()
        || !token.len().is_multiple_of(2)
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    token
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

async fn update(
    state: &AppState,
    headers: &HeaderMap,
    device_id: Uuid,
    token: Option<&[u8]>,
) -> Result<StatusCode, ApiError> {
    let credential = Credential::required(headers)?;
    let authorized = state
        .push_tokens
        .set_token(device_id, &credential.hash, token)
        .await
        .map_err(ApiError::Internal)?;
    if !authorized {
        return Err(ApiError::unauthorized());
    }
    Ok(StatusCode::NO_CONTENT)
}
