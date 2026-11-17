use std::path::PathBuf;

use axum::{Router, http::header::CACHE_CONTROL};
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

    Router::new()
        .route_service("/s/{token}", ServeFile::new(directory.join("index.html")))
        .fallback_service(ServeDir::new(directory))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
}
