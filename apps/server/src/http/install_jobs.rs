use std::{collections::BTreeMap, time::Duration};

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ApiError, ArtifactApi, ErrorResponse, artifacts::validate_upload, auth::Credential};
use crate::{db::install_jobs::object_key, r2::Verification};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateInstallJob {
    size_bytes: i64,
    /// Base64-encoded SHA-256 of the exact signed IPA to be uploaded.
    sha256: String,
}

#[derive(Serialize, ToSchema)]
struct InstallUpload {
    job_id: Uuid,
    upload_url: String,
    upload_expires_at: DateTime<Utc>,
    /// Send these headers with the PUT. Browsers set Content-Length automatically.
    upload_headers: BTreeMap<String, String>,
}

fn pairing_credential(headers: &HeaderMap) -> Result<Credential, ApiError> {
    Credential::required(headers).map_err(|_| ApiError {
        status: StatusCode::UNAUTHORIZED,
        body: ErrorResponse::PairingTokenRequired {
            error: "a valid browser pairing bearer token is required",
        },
    })
}

fn database(_: sqlx::Error) -> ApiError {
    ApiError::unavailable("install job database is unavailable")
}

fn missing() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        body: ErrorResponse::NotFound {
            error: "install job or active browser pairing not found",
        },
    }
}

fn unavailable() -> ApiError {
    // Storage errors may include presigned URLs and must not be logged.
    ApiError::unavailable("install artifact storage is unavailable")
}

#[utoipa::path(
    post, path = "/v1/install-jobs", operation_id = "create_install_job",
    request_body = CreateInstallJob,
    responses(
        (status = 201, description = "Private R2 upload authorized", body = InstallUpload),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    ),
    security(("pairing_token" = []))
)]
pub(super) async fn create(
    State(api): State<Option<ArtifactApi>>,
    headers: HeaderMap,
    request: Result<Json<CreateInstallJob>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = pairing_credential(&headers)?;
    let api = api.ok_or_else(unavailable)?;
    let Json(request) = request.map_err(|error| ApiError {
        status: error.status(),
        body: ErrorResponse::InvalidRequest {
            error: "expected size_bytes and base64 sha256",
        },
    })?;
    validate_upload(request.size_bytes, &request.sha256)?;

    let id = Uuid::new_v4();
    let deadline = api
        .install_jobs
        .create(&credential.hash, id, request.size_bytes, &request.sha256)
        .await
        .map_err(database)?
        .ok_or_else(missing)?;
    let now = Utc::now();
    let seconds = (deadline - now).num_seconds().min(900);
    if seconds <= 0 {
        return Err(missing());
    }
    let upload = api
        .storage
        .upload(
            &object_key(id),
            request.size_bytes,
            &request.sha256,
            Duration::from_secs(seconds as u64),
        )
        .await
        .map_err(|_| unavailable())?;

    Ok((
        StatusCode::CREATED,
        Json(InstallUpload {
            job_id: id,
            upload_url: upload.url,
            upload_expires_at: now + chrono::Duration::seconds(seconds),
            upload_headers: upload.headers,
        }),
    ))
}

#[utoipa::path(
    post, path = "/v1/install-jobs/{job_id}/complete", operation_id = "complete_install_upload",
    params(("job_id" = Uuid, Path)),
    responses(
        (status = 202, description = "Install job durably queued; safe to retry"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    ),
    security(("pairing_token" = []))
)]
pub(super) async fn complete(
    State(api): State<Option<ArtifactApi>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let credential = pairing_credential(&headers)?;
    let api = api.ok_or_else(unavailable)?;
    let job = api
        .install_jobs
        .find(&credential.hash, id)
        .await
        .map_err(database)?
        .ok_or_else(missing)?;
    if job.queued_at.is_some() {
        return Ok(StatusCode::ACCEPTED);
    }
    let verification = api
        .storage
        .verify(&object_key(id), job.size_bytes, &job.sha256)
        .await
        .map_err(|_| unavailable())?;
    let error = match verification {
        Verification::Valid => None,
        Verification::Missing => Some("upload has not completed"),
        Verification::Mismatch => {
            Some("uploaded object does not match the declared size or checksum")
        }
    };
    if let Some(error) = error {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            body: ErrorResponse::Conflict { error },
        });
    }
    if !api
        .install_jobs
        .queue(&credential.hash, id)
        .await
        .map_err(database)?
    {
        return Err(missing());
    }
    Ok(StatusCode::ACCEPTED)
}

impl ArtifactApi {
    /// Keep retrying if R2 is unavailable; do not forget the key before deletion.
    pub async fn clean_install_uploads(self) {
        let mut ticks = tokio::time::interval(Duration::from_secs(60));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let Ok(ids) = self.install_jobs.expired().await else {
                crate::telemetry::report("Could not list expired install uploads");
                continue;
            };
            for id in ids {
                if self.storage.delete(&object_key(id)).await.is_err() {
                    crate::telemetry::report("Could not delete expired install upload from R2");
                    continue;
                }
                if self.install_jobs.remove(id).await.is_err() {
                    crate::telemetry::report("Could not remove expired install job");
                    break;
                }
            }
        }
    }
}
