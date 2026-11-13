use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::{ApiError, AppState, vpn_required, vpn_source};
use crate::db::pairing_codes::CodeLookup;

#[derive(Serialize)]
struct PairingCodeResponse {
    code: String,
    expires_at: DateTime<Utc>,
}

pub(super) async fn get(
    State(state): State<AppState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Path(device_id): Path<Uuid>,
) -> Response {
    let result = async {
        let source = vpn_source(&state, source).await?;
        match state
            .codes
            .for_device(device_id, source)
            .await
            .map_err(|_| ApiError::unavailable("pairing code storage is unavailable"))?
        {
            CodeLookup::Forbidden => Err(vpn_required()),
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
