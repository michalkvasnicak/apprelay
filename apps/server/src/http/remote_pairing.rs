use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use std::num::NonZeroU16;
use uuid::Uuid;

use super::{ApiError, AppState, ErrorResponse, auth::Credential};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemotePairingRequest {
    port: NonZeroU16,
}

pub(super) async fn put(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<Uuid>,
    request: Result<Json<RemotePairingRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let credential = Credential::required(&headers)?;
    let Json(request) = request.map_err(|error| ApiError::Public {
        status: error.status(),
        body: ErrorResponse::InvalidRequest {
            error: "expected a port between 1 and 65535",
        },
    })?;
    let authorized = state
        .devices
        .set_remote_pairing_port(device_id, &credential.hash, request.port.get())
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    if !authorized {
        return Err(ApiError::unauthorized());
    }

    Ok(StatusCode::NO_CONTENT)
}
