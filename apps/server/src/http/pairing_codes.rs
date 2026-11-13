use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::{ApiError, AppState, auth::Credential};
use crate::db::pairing_codes::CodeLookup;

#[derive(Serialize)]
struct PairingCodeResponse {
    code: String,
    expires_at: DateTime<Utc>,
}

pub(super) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<Uuid>,
) -> Response {
    let result = async {
        let credential = Credential::required(&headers)?;
        match state
            .codes
            .for_device(device_id, &credential.hash)
            .await
            .map_err(|_| ApiError::unavailable("pairing code storage is unavailable"))?
        {
            CodeLookup::Forbidden => Err(ApiError::unauthorized()),
            CodeLookup::Missing => Err(ApiError {
                status: StatusCode::NOT_FOUND,
                message: "no active pairing code",
            }),
            CodeLookup::Active(code) => Ok(Json(PairingCodeResponse {
                code: code.code,
                expires_at: code.expires_at,
            })),
        }
    }
    .await;
    let mut response = result.into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
