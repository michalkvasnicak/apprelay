use std::sync::Arc;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::OperationError;

#[derive(Serialize, ToSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(super) enum ErrorResponse {
    InvalidRequest { error: &'static str },
    AppKeyRequired { error: &'static str },
    DeviceTokenRequired { error: &'static str },
    PairingTokenRequired { error: &'static str },
    InternalServerError { error: &'static str },
    NotFound { error: &'static str },
    Conflict { error: &'static str },
    Expired { error: &'static str },
    RateLimited { error: &'static str },
    UpstreamFailure { error: &'static str },
    UpstreamTimeout { error: &'static str },
}

pub(super) enum ApiError {
    Public {
        status: StatusCode,
        body: ErrorResponse,
    },
    Internal(OperationError),
}

impl ApiError {
    pub(super) fn unauthorized() -> Self {
        Self::Public {
            status: StatusCode::UNAUTHORIZED,
            body: ErrorResponse::DeviceTokenRequired {
                error: "a valid device bearer token is required",
            },
        }
    }

    pub(super) fn internal<E>(operation: &'static str) -> impl FnOnce(E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        move |error| Self::Internal(OperationError::at(operation)(error))
    }

    pub(super) fn internal_without_source(operation: &'static str) -> Self {
        Self::Internal(OperationError::without_source(operation))
    }

    pub(super) fn upstream_timeout() -> Self {
        Self::Public {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: ErrorResponse::UpstreamTimeout {
                error: "HTTPS proxy request timed out",
            },
        }
    }
}

pub(super) fn internal_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::InternalServerError {
            error: "internal server error",
        }),
    )
        .into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Public { status, body } => (status, Json(body)).into_response(),
            Self::Internal(error) => {
                let mut response = internal_error_response();
                // Carry the source to the HTTP boundary, not into the response body.
                response.extensions_mut().insert(Arc::new(error));
                response
            }
        }
    }
}
