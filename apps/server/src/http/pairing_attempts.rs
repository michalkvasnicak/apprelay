use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ApiError, AppState, ErrorResponse, auth::Credential};
use crate::db::pairing_attempts::{AttemptStatus, Claim};

#[derive(Serialize, ToSchema)]
struct CreatedAttempt {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    token: String,
}

fn token_hash(token: &str) -> Result<[u8; 32], ApiError> {
    let missing = || ApiError::Public {
        status: StatusCode::NOT_FOUND,
        body: super::ErrorResponse::NotFound {
            error: "pairing attempt not found",
        },
    };
    if token.len() != 43 {
        return Err(missing());
    }
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| missing())?
        .try_into()
        .map_err(|_| missing())?;
    Ok(digest(&SHA256, &bytes).as_ref().try_into().unwrap())
}

#[utoipa::path(
    post, path = "/v1/pairing-attempts", operation_id = "create_pairing_attempt",
    responses(
        (status = 201, body = CreatedAttempt),
        (status = 429, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn create(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let credential = Credential::generate()?;
    let expires_at = state
        .attempts
        .create(&credential.hash)
        .await
        .map_err(ApiError::Internal)?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedAttempt {
            token: credential.token,
            expires_at,
        }),
    ))
}

#[utoipa::path(
    get, path = "/v1/pairing-attempts/{token}", operation_id = "get_pairing_attempt",
    params(("token" = String, Path, description = "Secret pairing attempt capability")),
    responses(
        (status = 200, body = AttemptStatus),
        (status = 404, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
pub(super) async fn get(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let status = state
        .attempts
        .status(&token_hash(&token)?, &state.codes)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Public {
            status: StatusCode::NOT_FOUND,
            body: super::ErrorResponse::NotFound {
                error: "pairing attempt not found",
            },
        })?;
    Ok(Json(status))
}

#[utoipa::path(
    post, path = "/v1/devices/{device_id}/pairing", operation_id = "start_pairing_attempt",
    params(("device_id" = Uuid, Path)), request_body = StartRequest,
    responses(
        (status = 201, description = "Attempt claimed and durably queued; same-device retries are safe"),
        (status = 400, body = ErrorResponse), (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse),
        (status = 410, body = ErrorResponse), (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse), (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ), security(("app_key" = [], "device_token" = []))
)]
pub(super) async fn start(
    State(state): State<AppState>,
    Path(device_id): Path<Uuid>,
    headers: HeaderMap,
    request: Result<Json<StartRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let credential = Credential::required(&headers)?;
    let Json(request) = request.map_err(|error| ApiError::Public {
        status: error.status(),
        body: super::ErrorResponse::InvalidRequest {
            error: "invalid pairing JSON; expected token",
        },
    })?;
    match state
        .attempts
        .claim(&token_hash(&request.token)?, device_id, &credential.hash)
        .await
        .map_err(ApiError::Internal)?
    {
        Claim::Accepted => Ok(StatusCode::CREATED),
        Claim::Unauthorized => Err(ApiError::unauthorized()),
        Claim::Missing => Err(ApiError::Public {
            status: StatusCode::NOT_FOUND,
            body: super::ErrorResponse::NotFound {
                error: "pairing attempt not found",
            },
        }),
        Claim::Conflict => Err(ApiError::Public {
            status: StatusCode::CONFLICT,
            body: super::ErrorResponse::Conflict {
                error: "attempt already used or device has an active attempt",
            },
        }),
        Claim::Expired => Err(ApiError::Public {
            status: StatusCode::GONE,
            body: super::ErrorResponse::Expired {
                error: "pairing attempt expired",
            },
        }),
    }
}
