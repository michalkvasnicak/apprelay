//! Stateless relay for the browser Apple runtime. Never log upstream URLs,
//! headers, bodies, or reqwest errors: they can contain Apple credentials.

use std::{collections::BTreeMap, io, net::IpAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::{HeaderName, HeaderValue, StatusCode, header::CACHE_CONTROL},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Method, Url,
    dns::{Addrs, Name, Resolve, Resolving},
};
use serde::{Deserialize, Serialize};
use tokio::{net::lookup_host, sync::Semaphore, time::timeout};
use utoipa::ToSchema;

use super::ApiError;

const MAX_BODY_BYTES: usize = 2_000_000;
// Base64 expansion plus bounded JSON/header overhead.
const MAX_ENVELOPE_BYTES: usize = 2_800_000;
const MAX_HEADER_BYTES: usize = 32_768;
const MAX_CONCURRENT_REQUESTS: usize = 8;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Clone)]
pub struct HttpsProxy {
    client: Client,
    permits: Arc<Semaphore>,
}

impl HttpsProxy {
    pub fn new() -> io::Result<Self> {
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .dns_resolver(Arc::new(PublicDns))
            .build()
            .map_err(|_| io::Error::other("could not initialize HTTPS proxy"))?;
        Ok(Self {
            client,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        })
    }
}

pub(super) fn router(proxy: HttpsProxy) -> Router<super::AppState> {
    Router::new()
        .route("/v1/apple/proxy", post(relay))
        .layer(DefaultBodyLimit::max(MAX_ENVELOPE_BYTES))
        .layer(middleware::from_fn_with_state(proxy.clone(), limit_request))
        .with_state(proxy)
}

// Acquire before reading JSON, bound upload time too, and never queue excess work.
async fn limit_request(State(proxy): State<HttpsProxy>, request: Request, next: Next) -> Response {
    let result = async {
        let _permit = proxy.permits.try_acquire().map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "HTTPS proxy is busy; retry later",
        })?;
        timeout(REQUEST_TIMEOUT, next.run(request))
            .await
            .map_err(|_| upstream_timeout())
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

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct ProxyRequest {
    /// HTTPS URL on an explicitly permitted Apple or anisette endpoint.
    url: String,
    method: ProxyMethod,
    /// Only protocol headers are allowed; never include the AppRelay API key.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Standard base64-encoded bytes. Omit for an empty request body.
    #[serde(default)]
    body: String,
}

#[derive(Deserialize, ToSchema)]
enum ProxyMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

#[derive(Serialize, ToSchema)]
struct ProxyResponse {
    /// Upstream HTTP status, including rejections and redirects (never followed).
    status: u16,
    /// Only content-type, retry-after, and Apple routing info are returned.
    headers: BTreeMap<String, String>,
    /// Standard base64-encoded upstream response bytes.
    body: String,
}

#[utoipa::path(
    post,
    path = "/v1/apple/proxy",
    request_body = ProxyRequest,
    responses(
        (status = 200, description = "Upstream response; inspect the envelope status", body = ProxyResponse),
        (status = 400, description = "Invalid proxy request or destination/header policy", body = super::ErrorResponse),
        (status = 413, description = "Request too large", body = super::ErrorResponse),
        (status = 415, description = "Expected application/json", body = super::ErrorResponse),
        (status = 422, description = "Invalid JSON fields", body = super::ErrorResponse),
        (status = 502, description = "Upstream TLS, DNS, transport, encoding or response-size failure", body = super::ErrorResponse),
        (status = 503, description = "Proxy concurrency limit reached", body = super::ErrorResponse),
        (status = 504, description = "Proxy timed out", body = super::ErrorResponse)
    )
)]
async fn relay(
    State(proxy): State<HttpsProxy>,
    request: Result<Json<ProxyRequest>, JsonRejection>,
) -> Result<Json<ProxyResponse>, ApiError> {
    let Json(request) = request.map_err(|error| ApiError {
        status: error.status(),
        message: "invalid HTTPS proxy JSON",
    })?;
    let url = Url::parse(&request.url).map_err(|_| policy_error())?;
    if request.url.len() > 2048
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || !allowed_endpoint(&url, &request.method)
    {
        return Err(policy_error());
    }
    if request.body.len() > MAX_BODY_BYTES.div_ceil(3) * 4 {
        return Err(request_too_large());
    }
    let body = STANDARD.decode(&request.body).map_err(|_| policy_error())?;
    if body.len() > MAX_BODY_BYTES {
        return Err(request_too_large());
    }
    if matches!(request.method, ProxyMethod::Get) && !body.is_empty() {
        return Err(policy_error());
    }
    let apple = url.host_str() != Some("mac-ani.jkcoxson.com");
    let method = match request.method {
        ProxyMethod::Get => Method::GET,
        ProxyMethod::Post => Method::POST,
    };
    let mut outgoing = proxy.client.request(method, url);
    let mut headers = reqwest::header::HeaderMap::new();
    let mut header_bytes = 0usize;
    for (name, value) in request.headers {
        header_bytes += name.len() + value.len();
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| policy_error())?;
        if header_bytes > MAX_HEADER_BYTES
            || !allowed_header(name.as_str(), apple)
            || headers.contains_key(&name)
            || (name == "x-http-method-override"
                && !matches!(value.as_str(), "GET" | "POST" | "PUT" | "DELETE"))
        {
            return Err(policy_error());
        }
        headers.insert(
            name,
            HeaderValue::from_str(&value).map_err(|_| policy_error())?,
        );
    }
    outgoing = outgoing
        .headers(headers)
        .header("accept-encoding", "identity")
        .body(body);
    let mut response = outgoing.send().await.map_err(transport_error)?;
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BODY_BYTES as u64)
    {
        return Err(upstream_error());
    }
    // Compression is not requested. Fail closed rather than forwarding compressed
    // bytes as a plist or decompressing an unbounded upstream payload.
    for encoding in response.headers().get_all("content-encoding") {
        if !encoding.as_bytes().eq_ignore_ascii_case(b"identity") {
            return Err(upstream_error());
        }
    }
    let mut headers = BTreeMap::new();
    for name in ["content-type", "retry-after", "x-apple-i-md-rinfo"] {
        if let Some(value) = response.headers().get(name) {
            let value = value.to_str().map_err(|_| upstream_error())?;
            headers.insert(name.to_owned(), value.to_owned());
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > MAX_BODY_BYTES - body.len() {
            return Err(upstream_error());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Json(ProxyResponse {
        status,
        headers,
        body: STANDARD.encode(body),
    }))
}

fn allowed_endpoint(url: &Url, method: &ProxyMethod) -> bool {
    let path = url.path();
    match (url.host_str(), method) {
        (Some("gsa.apple.com"), ProxyMethod::Get) => matches!(
            path,
            "/grandslam/GsService2/lookup"
                | "/grandslam/GsService2/validate"
                | "/auth/verify/trusteddevice"
        ),
        (Some("gsa.apple.com"), ProxyMethod::Post) => matches!(
            path,
            "/grandslam/GsService2"
                | "/grandslam/GsService2/midStartProvisioning"
                | "/grandslam/GsService2/midFinishProvisioning"
                | "/grandslam/MidService/startMachineProvisioning"
                | "/grandslam/MidService/finishMachineProvisioning"
        ),
        (Some("developerservices2.apple.com"), ProxyMethod::Post) => {
            path.starts_with("/services/QH65B2/")
                || path == "/services/v1/certificates"
                || path.starts_with("/services/v1/certificates/")
        }
        (Some("mac-ani.jkcoxson.com"), ProxyMethod::Get) => path == "/v3/client_info",
        (Some("mac-ani.jkcoxson.com"), ProxyMethod::Post) => path == "/v3/get_headers",
        _ => false,
    }
}

fn allowed_header(name: &str, apple: bool) -> bool {
    matches!(
        name,
        "accept" | "accept-language" | "content-type" | "user-agent"
    ) || (apple
        && matches!(
            name,
            "security-code"
                | "x-apple-app-info"
                | "x-xcode-version"
                | "x-apple-identity-token"
                | "x-apple-i-identity-id"
                | "x-apple-gs-token"
                | "x-apple-i-md"
                | "x-apple-i-md-m"
                | "x-apple-i-md-lu"
                | "x-apple-i-md-rinfo"
                | "x-apple-i-srl-no"
                | "x-mme-device-id"
                | "x-mme-client-info"
                | "x-apple-i-client-time"
                | "x-apple-locale"
                | "x-apple-i-locale"
                | "x-apple-i-timezone"
                | "x-apple-client-app-name"
                | "x-http-method-override"
        ))
}

// Validate the actual socket addresses handed to reqwest, not a separate DNS
// preflight. Reject mixed public/private answers and disable environment proxies.
struct PublicDns;

impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = lookup_host((name.as_str(), 443)).await?.collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !public_address(address.ip()))
            {
                return Err(io::Error::other("upstream DNS policy rejected").into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && ((b == 0 && c == 0) || (b == 88 && c == 99)))
                && !(a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Only global unicast; exclude special-purpose, 6to4 and docs.
            segments[0] & 0xe000 == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

fn policy_error() -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "HTTPS proxy request violates destination, method, body or header policy",
    }
}

fn request_too_large() -> ApiError {
    ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        message: "HTTPS proxy request is too large",
    }
}

fn upstream_error() -> ApiError {
    ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: "HTTPS proxy upstream request failed",
    }
}

fn upstream_timeout() -> ApiError {
    ApiError {
        status: StatusCode::GATEWAY_TIMEOUT,
        message: "HTTPS proxy request timed out",
    }
}

fn transport_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        upstream_timeout()
    } else {
        upstream_error()
    }
}
