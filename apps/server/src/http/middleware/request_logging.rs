use std::{sync::Arc, time::Instant};

use axum::{
    extract::{MatchedPath, Request},
    http::{StatusCode, header::CACHE_CONTROL},
    middleware::Next,
    response::Response,
};

use crate::{error::OperationError, http::responses::internal_error_response};

pub(in crate::http) async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    // Matched route patterns contain placeholders, never capability tokens/IDs.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    sentry::configure_scope(|scope| {
        scope.set_tag("http.method", method.as_str());
        scope.set_tag("http.route", &route);
    });
    let started = Instant::now();
    let mut response = next.run(request).await;
    if response.status().is_server_error() {
        let error = response.extensions_mut().remove::<Arc<OperationError>>();
        let status = response.status();
        sentry::with_scope(
            |scope| {
                scope.set_tag("http.status_code", status.as_str());
                let mut fingerprint =
                    vec!["http", method.as_str(), route.as_str(), status.as_str()];
                if let Some(error) = &error {
                    fingerprint.push(error.operation);
                }
                scope.set_fingerprint(Some(&fingerprint));
            },
            || {
                if let Some(error) = &error {
                    crate::telemetry::report_operation(
                        "HTTP request failed",
                        error.operation,
                        error.cause_code(),
                    );
                } else {
                    crate::telemetry::report("HTTP request failed");
                }
            },
        );
        if status == StatusCode::INTERNAL_SERVER_ERROR && error.is_none() {
            // Normalize framework/service 500s that bypass ApiError as well.
            response = internal_error_response();
        }
    }
    response
        .headers_mut()
        .entry(CACHE_CONTROL)
        .or_insert(axum::http::HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    let status = response.status();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if status.is_server_error() {
        tracing::error!(
            method = method.as_str(),
            route = route.as_str(),
            status = status.as_u16(),
            elapsed_ms,
            "HTTP request completed"
        );
    } else {
        tracing::info!(
            method = method.as_str(),
            route = route.as_str(),
            status = status.as_u16(),
            elapsed_ms,
            "HTTP request completed"
        );
    }
    response
}
