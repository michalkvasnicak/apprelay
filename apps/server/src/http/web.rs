use std::path::PathBuf;

use axum::{
    Router,
    extract::Request,
    http::{HeaderName, HeaderValue, StatusCode, header::CACHE_CONTROL},
    middleware::{self, Next},
    response::Response,
};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

pub(super) fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let directory = std::env::var_os("APPRELAY_WEB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist"));

    let workspace = ServeFile::new(directory.join("workspace.html"));
    Router::new()
        .route_service("/sign", workspace.clone())
        .route_service("/s/{token}", workspace.clone())
        .route_service("/pair", workspace)
        .route_layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-robots-tag"),
            HeaderValue::from_static("noindex"),
        ))
        .fallback_service(ServeDir::new(directory))
        .layer(middleware::from_fn(cache_assets))
}

async fn cache_assets(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let cache_control = if path.starts_with("/assets/build/") {
        // Vite fingerprints its build outputs; public files keep their names.
        Some("public, max-age=31536000, immutable")
    } else if path.starts_with("/assets/") {
        Some("public, max-age=604800")
    } else {
        None
    };

    let mut response = next.run(request).await;
    if let Some(cache_control) = cache_control
        && (response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED)
    {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    }
    response
}
