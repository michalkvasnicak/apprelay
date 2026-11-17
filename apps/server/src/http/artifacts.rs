use std::{collections::BTreeMap, io, num::NonZeroU16};

use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Utc};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ApiError, ErrorResponse, RegistrationRateLimiter, auth::Credential, rate_limit};
use crate::{
    db::artifacts::{Artifact, ArtifactRepository},
    r2::{R2Storage, Verification},
};

const MAX_BYTES: i64 = 100 * 1024 * 1024;
const ENVIRONMENT: [&str; 6] = [
    "APPRELAY_R2_ACCOUNT_ID",
    "APPRELAY_R2_BUCKET",
    "APPRELAY_R2_ACCESS_KEY_ID",
    "APPRELAY_R2_SECRET_ACCESS_KEY",
    "APPRELAY_PUBLIC_ORIGIN",
    "APPRELAY_IPA_LIFETIME_DAYS",
];

#[derive(Clone)]
pub struct ArtifactApi {
    repository: ArtifactRepository,
    storage: R2Storage,
    public_origin: String,
    lifetime_days: Option<NonZeroU16>,
}

impl ArtifactApi {
    /// Disabled only when none of the artifact configuration variables are set.
    pub fn from_env(pool: PgPool) -> io::Result<Option<Self>> {
        if ENVIRONMENT
            .iter()
            .all(|name| std::env::var_os(name).is_none())
        {
            return Ok(None);
        }
        let required = |name: &str| -> io::Result<String> {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    io::Error::other(format!(
                        "{name} is required when artifact uploads are enabled"
                    ))
                })
        };
        let lifetime_days = match std::env::var("APPRELAY_IPA_LIFETIME_DAYS") {
            Ok(value) => Some(value.parse::<NonZeroU16>().map_err(|_| {
                io::Error::other(
                    "APPRELAY_IPA_LIFETIME_DAYS must be an integer between 1 and 65535",
                )
            })?),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => {
                return Err(io::Error::other(
                    "APPRELAY_IPA_LIFETIME_DAYS must be valid Unicode",
                ));
            }
        };
        let origin = required("APPRELAY_PUBLIC_ORIGIN")?;
        let url = reqwest::Url::parse(&origin)
            .map_err(|_| io::Error::other("APPRELAY_PUBLIC_ORIGIN must be an HTTPS origin"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(io::Error::other(
                "APPRELAY_PUBLIC_ORIGIN must be an HTTPS origin without credentials, path, query, or fragment",
            ));
        }
        let storage = R2Storage::new(
            &required("APPRELAY_R2_ACCOUNT_ID")?,
            required("APPRELAY_R2_BUCKET")?,
            required("APPRELAY_R2_ACCESS_KEY_ID")?,
            required("APPRELAY_R2_SECRET_ACCESS_KEY")?,
        )?;
        Ok(Some(Self {
            repository: ArtifactRepository::new(pool),
            storage,
            public_origin: url.origin().ascii_serialization(),
            lifetime_days,
        }))
    }
}

pub(super) fn router<S>(api: Option<ArtifactApi>, limiter: RegistrationRateLimiter) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/v1/artifacts", post(create))
        .route("/v1/artifacts/{artifact_id}/complete", post(complete))
        .route("/v1/artifacts/{token}", get(resolve))
        .route_layer(middleware::from_fn_with_state(
            limiter,
            rate_limit::limit_artifacts,
        ))
        .with_state(api)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateRequest {
    file_name: String,
    size_bytes: i64,
    /// Base64-encoded SHA-256 digest of the complete IPA (32 bytes, not hex).
    sha256: String,
}

#[derive(Serialize, ToSchema)]
struct CreatedArtifact {
    artifact_id: Uuid,
    /// Browser signing page URL.
    share_url: String,
    /// Null when the IPA is retained indefinitely.
    expires_at: Option<DateTime<Utc>>,
    upload_url: String,
    upload_expires_at: DateTime<Utc>,
    /// Send these headers verbatim with the PUT. Browsers set Content-Length automatically.
    upload_headers: BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
pub(super) struct ResolvedArtifact {
    file_name: String,
    size_bytes: i64,
    sha256: String,
    /// Null when the IPA is retained indefinitely.
    expires_at: Option<DateTime<Utc>>,
    download_url: String,
    download_expires_at: DateTime<Utc>,
}

fn invalid(message: &'static str) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        body: ErrorResponse::InvalidRequest { error: message },
    }
}

fn missing() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        body: ErrorResponse::NotFound {
            error: "artifact not found or expired",
        },
    }
}

fn unavailable() -> ApiError {
    // Never log SDK errors: they can contain credentials or presigned URLs.
    ApiError::unavailable("artifact storage is unavailable")
}

fn database(_: sqlx::Error) -> ApiError {
    ApiError::unavailable("artifact database is unavailable")
}

#[utoipa::path(
    post, path = "/v1/artifacts", request_body = CreateRequest,
    responses(
        (status = 201, body = CreatedArtifact),
        (status = 400, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub(super) async fn create(
    State(api): State<Option<ArtifactApi>>,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let api = api.ok_or_else(unavailable)?;
    let Json(request) = request.map_err(|error| ApiError {
        status: error.status(),
        body: ErrorResponse::InvalidRequest {
            error: "expected file_name, size_bytes, and base64 sha256",
        },
    })?;
    if !(5..=200).contains(&request.file_name.chars().count())
        || !request.file_name.to_ascii_lowercase().ends_with(".ipa")
        || request
            .file_name
            .chars()
            .any(|ch| ch.is_control() || ch == '/' || ch == '\\')
    {
        return Err(invalid(
            "file_name must be an IPA basename of at most 200 characters",
        ));
    }
    if !(1..=MAX_BYTES).contains(&request.size_bytes) {
        return Err(invalid("size_bytes must be between 1 and 104857600"));
    }
    if request.sha256.len() != 44
        || STANDARD
            .decode(&request.sha256)
            .map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(invalid("sha256 must be a base64-encoded 32-byte digest"));
    }
    let credential = Credential::generate()?;
    let id = Uuid::new_v4();
    let artifact = Artifact {
        id,
        // Separate retention classes so lifecycle rules cannot delete permanent IPAs.
        object_key: match api.lifetime_days {
            Some(days) => format!("originals/retained-{days}d/{id}.ipa"),
            None => format!("originals/permanent/{id}.ipa"),
        },
        file_name: request.file_name,
        size_bytes: request.size_bytes,
        sha256: request.sha256,
        expires_at: None,
    };
    // Presign before inserting, so signing failures do not create unusable records.
    let upload_expires_at = Utc::now() + chrono::Duration::minutes(15);
    let upload = api
        .storage
        .upload(&artifact.object_key, artifact.size_bytes, &artifact.sha256)
        .await
        .map_err(|_| unavailable())?;
    let expires_at = api
        .repository
        .create(&artifact, &credential.hash, api.lifetime_days)
        .await
        .map_err(database)?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedArtifact {
            artifact_id: id,
            share_url: format!("{}/s/{}", api.public_origin, credential.token),
            expires_at,
            upload_url: upload.url,
            upload_expires_at,
            upload_headers: upload.headers,
        }),
    ))
}

#[utoipa::path(
    post, path = "/v1/artifacts/{artifact_id}/complete",
    params(("artifact_id" = Uuid, Path)),
    responses(
        (status = 204, description = "Object size and checksum-bound metadata verified; safe to retry"),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub(super) async fn complete(
    State(api): State<Option<ArtifactApi>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let api = api.ok_or_else(unavailable)?;
    let artifact = api
        .repository
        .find(id)
        .await
        .map_err(database)?
        .ok_or_else(missing)?;
    match api
        .storage
        .verify(&artifact.object_key, artifact.size_bytes, &artifact.sha256)
        .await
        .map_err(|_| unavailable())?
    {
        Verification::Valid => {}
        Verification::Missing => {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                body: ErrorResponse::Conflict {
                    error: "upload has not completed",
                },
            });
        }
        Verification::Mismatch => {
            return Err(ApiError {
                status: StatusCode::CONFLICT,
                body: ErrorResponse::Conflict {
                    error: "uploaded object does not match the declared size or checksum",
                },
            });
        }
    }
    if !api.repository.complete(id).await.map_err(database)? {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/v1/artifacts/{token}",
    params(("token" = String, Path, description = "Secret share capability")),
    responses(
        (status = 200, body = ResolvedArtifact),
        (status = 404, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub(super) async fn resolve(
    State(api): State<Option<ArtifactApi>>,
    Path(token): Path<String>,
) -> Result<Json<ResolvedArtifact>, ApiError> {
    let api = api.ok_or_else(unavailable)?;
    if token.len() != 43 {
        return Err(missing());
    }
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| missing())?
        .try_into()
        .map_err(|_| missing())?;
    let hash: [u8; 32] = digest(&SHA256, &bytes).as_ref().try_into().unwrap();
    let artifact = api
        .repository
        .resolve(&hash)
        .await
        .map_err(database)?
        .ok_or_else(missing)?;
    let now = Utc::now();
    let seconds = artifact
        .expires_at
        .map_or(60, |expiry| (expiry - now).num_seconds().min(60));
    if seconds <= 0 {
        return Err(missing());
    }
    let download_url = api
        .storage
        .download(&artifact.object_key, seconds as u64)
        .await
        .map_err(|_| unavailable())?;
    Ok(Json(ResolvedArtifact {
        file_name: artifact.file_name,
        size_bytes: artifact.size_bytes,
        sha256: artifact.sha256,
        expires_at: artifact.expires_at,
        download_url,
        download_expires_at: now + chrono::Duration::seconds(seconds),
    }))
}
