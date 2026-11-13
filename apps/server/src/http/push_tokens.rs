use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{ApiError, AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PushTokenRequest {
    token: String,
}

pub(super) async fn put(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Path(device_id): Path<Uuid>,
    request: Result<Json<PushTokenRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(request) = request.map_err(|error| ApiError {
        status: error.status(),
        message: "invalid push token JSON; expected token",
    })?;
    let token = decode_token(&request.token).ok_or(ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "token must be a nonempty, even-length hexadecimal string",
    })?;
    update(&state, source, device_id, Some(&token)).await
}

pub(super) async fn delete(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Path(device_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    update(&state, source, device_id, None).await
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
    source: SocketAddr,
    device_id: Uuid,
    token: Option<&[u8]>,
) -> Result<StatusCode, ApiError> {
    let source = super::vpn_source(state, source).await?;
    let authorized = state
        .push_tokens
        .set_token(device_id, source, token)
        .await
        .map_err(|_| {
            // Do not print SQL errors: constraint details may include the token.
            eprintln!("Push token database operation failed");
            ApiError::unavailable("push token storage is unavailable")
        })?;
    if !authorized {
        return Err(super::vpn_required());
    }
    Ok(StatusCode::NO_CONTENT)
}
