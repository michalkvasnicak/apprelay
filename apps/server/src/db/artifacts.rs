use std::num::NonZeroU16;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::OperationError;

#[derive(Clone)]
pub(crate) struct ArtifactRepository {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
pub(crate) struct Artifact {
    pub id: Uuid,
    pub object_key: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
#[error("could not create artifact")]
pub(crate) struct CreateArtifactError(#[source] sqlx::Error);

impl From<CreateArtifactError> for OperationError {
    fn from(error: CreateArtifactError) -> Self {
        Self::at("create_artifact")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not find artifact")]
pub(crate) struct FindArtifactError(#[source] sqlx::Error);

impl From<FindArtifactError> for OperationError {
    fn from(error: FindArtifactError) -> Self {
        Self::at("find_artifact")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not complete artifact")]
pub(crate) struct CompleteArtifactError(#[source] sqlx::Error);

impl From<CompleteArtifactError> for OperationError {
    fn from(error: CompleteArtifactError) -> Self {
        Self::at("complete_artifact")(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not resolve artifact")]
pub(crate) struct ResolveArtifactError(#[source] sqlx::Error);

impl From<ResolveArtifactError> for OperationError {
    fn from(error: ResolveArtifactError) -> Self {
        Self::at("resolve_artifact")(error)
    }
}

impl ArtifactRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        artifact: &Artifact,
        token_hash: &[u8; 32],
        lifetime_days: Option<NonZeroU16>,
    ) -> Result<Option<DateTime<Utc>>, CreateArtifactError> {
        sqlx::query_scalar(
            "INSERT INTO ipa_artifacts
             (id, token_hash, object_key, file_name, size_bytes, sha256, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6,
                 CASE WHEN $7::integer IS NULL THEN NULL
                      ELSE clock_timestamp() + make_interval(days => $7) END)
             RETURNING expires_at",
        )
        .bind(artifact.id)
        .bind(token_hash.as_slice())
        .bind(&artifact.object_key)
        .bind(&artifact.file_name)
        .bind(artifact.size_bytes)
        .bind(&artifact.sha256)
        .bind(lifetime_days.map(|days| i32::from(days.get())))
        .fetch_one(&self.pool)
        .await
        .map_err(CreateArtifactError)
    }

    pub async fn find(&self, id: Uuid) -> Result<Option<Artifact>, FindArtifactError> {
        sqlx::query_as(
            "SELECT id, object_key, file_name, size_bytes, sha256, expires_at
             FROM ipa_artifacts WHERE id = $1
             AND (expires_at IS NULL OR expires_at > clock_timestamp())",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(FindArtifactError)
    }

    pub async fn complete(&self, id: Uuid) -> Result<bool, CompleteArtifactError> {
        Ok(sqlx::query(
            "UPDATE ipa_artifacts SET completed_at = COALESCE(completed_at, clock_timestamp())
             WHERE id = $1 AND (expires_at IS NULL OR expires_at > clock_timestamp())",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(CompleteArtifactError)?
        .rows_affected()
            == 1)
    }

    pub async fn resolve(&self, hash: &[u8; 32]) -> Result<Option<Artifact>, ResolveArtifactError> {
        sqlx::query_as(
            "SELECT id, object_key, file_name, size_bytes, sha256, expires_at
             FROM ipa_artifacts WHERE token_hash = $1
             AND completed_at IS NOT NULL
             AND (expires_at IS NULL OR expires_at > clock_timestamp())",
        )
        .bind(hash.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(ResolveArtifactError)
    }
}
