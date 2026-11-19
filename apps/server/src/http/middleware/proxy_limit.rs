use std::{sync::Arc, time::Duration};

use axum::{
    extract::{Request, State},
    http::{HeaderValue, header::CACHE_CONTROL},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::{sync::Semaphore, time::timeout};

use crate::http::responses::ApiError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(75);

// Acquire before reading JSON, bound upload time too, and never queue excess work.
pub(in crate::http) async fn limit_request(
    State(permits): State<Arc<Semaphore>>,
    request: Request,
    next: Next,
) -> Response {
    let result = async {
        let _permit = permits
            .try_acquire()
            .map_err(ApiError::internal("acquire_https_proxy_permit"))?;
        timeout(REQUEST_TIMEOUT, next.run(request))
            .await
            .map_err(|_| ApiError::upstream_timeout())
    }
    .await;
    let mut response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
