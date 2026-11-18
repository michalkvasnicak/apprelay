//! Stateless relay for the browser Apple runtime. Never log upstream URLs,
//! headers, bodies, or reqwest errors: they can contain Apple credentials.

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    io::{self, Read},
    net::IpAddr,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_ENCODING},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use brotli_decompressor::Decompressor;
use flate2::read::{MultiGzDecoder, ZlibDecoder};
use reqwest::{
    Client, Method, Url,
    dns::{Addrs, Name, Resolve, Resolving},
};
use rustls::pki_types::{CertificateDer, pem::PemObject};
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
// Apple Root CA from https://www.apple.com/appleca/AppleIncRootCertificate.cer.
const APPLE_ROOT_CA: &[u8] = include_bytes!("apple-root-ca.crt");
const ANISETTE_URL_ENV: &str = "APPRELAY_ANISETTE_URL";

#[derive(Clone)]
pub struct HttpsProxy {
    apple_client: Client,
    anisette_client: Client,
    anisette_url: Url,
    permits: Arc<Semaphore>,
}

impl HttpsProxy {
    pub fn from_env() -> io::Result<Self> {
        let url = env::var(ANISETTE_URL_ENV).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "APPRELAY_ANISETTE_URL is required",
            )
        })?;
        Self::new(&url)
    }

    pub fn new(anisette_url: &str) -> io::Result<Self> {
        let invalid_url = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "APPRELAY_ANISETTE_URL must be an HTTPS origin on a separate DNS hostname, using port 443 and no credentials, path, query, or fragment",
            )
        };
        let anisette_url = Url::parse(anisette_url).map_err(|_| invalid_url())?;
        if anisette_url.scheme() != "https"
            || anisette_url.domain().is_none()
            || matches!(
                anisette_url
                    .host_str()
                    .map(|host| host.trim_end_matches('.')),
                Some("gsa.apple.com" | "developerservices2.apple.com")
            )
            || anisette_url.port_or_known_default() != Some(443)
            || !anisette_url.username().is_empty()
            || anisette_url.password().is_some()
            || anisette_url.path() != "/"
            || anisette_url.query().is_some()
            || anisette_url.fragment().is_some()
        {
            return Err(invalid_url());
        }

        let builder = || {
            Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                // Count wire bytes before doing any decompression in the relay.
                .no_gzip()
                .no_deflate()
                .no_brotli()
                .no_zstd()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .dns_resolver(Arc::new(PublicDns))
        };
        // Extend the default roots for Apple only; keep TLS and hostname verification enabled.
        let apple_root = CertificateDer::from_pem_slice(APPLE_ROOT_CA)
            .map_err(|_| io::Error::other("could not load Apple Root CA"))?;
        let mut roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        roots
            .add(apple_root)
            .map_err(|_| io::Error::other("could not trust Apple Root CA"))?;
        let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| io::Error::other("could not configure Apple TLS protocol versions"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
        // Fresh connections alone do not prevent TLS session resumption.
        tls.resumption = rustls::client::Resumption::disabled();
        let apple_client = builder()
            .use_preconfigured_tls(tls)
            .http1_only()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| io::Error::other("could not initialize Apple HTTPS client"))?;
        let anisette_client = builder()
            .build()
            .map_err(|_| io::Error::other("could not initialize anisette HTTPS client"))?;
        Ok(Self {
            apple_client,
            anisette_client,
            anisette_url,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        })
    }
}

pub(super) fn router(proxy: HttpsProxy) -> Router<super::AppState> {
    Router::new()
        .route("/v1/apple/proxy", post(relay))
        .layer(DefaultBodyLimit::max(MAX_ENVELOPE_BYTES))
        .layer(middleware::from_fn_with_state(proxy.clone(), limit_request))
        .route("/v1/apple/anisette-provider", get(anisette_provider))
        .with_state(proxy)
}

// Acquire before reading JSON, bound upload time too, and never queue excess work.
async fn limit_request(State(proxy): State<HttpsProxy>, request: Request, next: Next) -> Response {
    let result = async {
        let _permit = proxy.permits.try_acquire().map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            body: super::ErrorResponse::ServiceUnavailable {
                error: "HTTPS proxy is busy; retry later",
            },
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

#[derive(Serialize, ToSchema)]
struct AnisetteProvider {
    url: String,
}

#[utoipa::path(
    get,
    path = "/v1/apple/anisette-provider",
    responses((status = 200, description = "Public anisette provider URL", body = AnisetteProvider))
)]
async fn anisette_provider(State(proxy): State<HttpsProxy>) -> Json<AnisetteProvider> {
    Json(AnisetteProvider {
        url: proxy.anisette_url.as_str().to_owned(),
    })
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
        body: super::ErrorResponse::InvalidRequest {
            error: "invalid HTTPS proxy JSON",
        },
    })?;
    let url = Url::parse(&request.url).map_err(|_| policy_error())?;
    let anisette = url.host_str() == proxy.anisette_url.host_str();
    if request.url.len() > 2048
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || !allowed_endpoint(&url, &request.method, anisette)
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
    let client = if anisette {
        &proxy.anisette_client
    } else {
        &proxy.apple_client
    };
    let method = match request.method {
        ProxyMethod::Get => Method::GET,
        ProxyMethod::Post => Method::POST,
    };
    let upstream = match url.host_str() {
        Some("gsa.apple.com") => "apple-auth",
        Some("developerservices2.apple.com") => "apple-developer",
        _ => "anisette",
    };
    let mut outgoing = client.request(method, url);
    let mut headers = reqwest::header::HeaderMap::new();
    let mut header_bytes = 0usize;
    for (name, value) in request.headers {
        header_bytes += name.len() + value.len();
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| policy_error())?;
        if header_bytes > MAX_HEADER_BYTES
            || !allowed_header(name.as_str(), !anisette)
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
    if !anisette {
        outgoing = outgoing.header("connection", "close");
    }
    let mut response = outgoing
        .send()
        .await
        .map_err(|error| transport_error(upstream, "send", error))?;
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BODY_BYTES as u64)
    {
        return Err(upstream_error(upstream, "response-too-large"));
    }
    // The developer portal can compress replies even when we request identity.
    let encodings = response_encodings(upstream, response.headers())?;
    let mut headers = BTreeMap::new();
    for name in ["content-type", "retry-after", "x-apple-i-md-rinfo"] {
        if let Some(value) = response.headers().get(name) {
            let value = value
                .to_str()
                .map_err(|_| upstream_error(upstream, "response-header"))?;
            headers.insert(name.to_owned(), value.to_owned());
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport_error(upstream, "response-read", error))?
    {
        if chunk.len() > MAX_BODY_BYTES - body.len() {
            return Err(upstream_error(upstream, "response-too-large"));
        }
        body.extend_from_slice(&chunk);
    }
    // Content-Encoding lists codings in the order they were applied.
    for encoding in encodings.into_iter().rev() {
        body = match encoding {
            ResponseEncoding::Gzip => decode_response(upstream, MultiGzDecoder::new(&body[..])),
            ResponseEncoding::Deflate => decode_response(upstream, ZlibDecoder::new(&body[..])),
            ResponseEncoding::Brotli => {
                decode_response(upstream, Decompressor::new(&body[..], 4096))
            }
        }?;
    }
    Ok(Json(ProxyResponse {
        status,
        headers,
        body: STANDARD.encode(body),
    }))
}

enum ResponseEncoding {
    Gzip,
    Deflate,
    Brotli,
}

fn response_encodings(
    upstream: &'static str,
    headers: &HeaderMap,
) -> Result<Vec<ResponseEncoding>, ApiError> {
    let mut encodings = Vec::new();
    let mut count = 0;
    let encoding_error = || upstream_error(upstream, "response-encoding");
    for value in headers.get_all(CONTENT_ENCODING) {
        let value = value.to_str().map_err(|_| encoding_error())?;
        for encoding in value.split(',').map(str::trim) {
            count += 1;
            if count > 3 {
                return Err(encoding_error());
            }
            if encoding.eq_ignore_ascii_case("identity") {
                continue;
            }
            if upstream != "apple-developer" {
                return Err(encoding_error());
            }
            encodings.push(if encoding.eq_ignore_ascii_case("gzip") {
                ResponseEncoding::Gzip
            } else if encoding.eq_ignore_ascii_case("deflate") {
                ResponseEncoding::Deflate
            } else if encoding.eq_ignore_ascii_case("br") {
                ResponseEncoding::Brotli
            } else {
                return Err(encoding_error());
            });
        }
    }
    Ok(encodings)
}

fn decode_response(upstream: &'static str, decoder: impl Read) -> Result<Vec<u8>, ApiError> {
    let mut decoded = Vec::new();
    decoder
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| upstream_error(upstream, "response-encoding"))?;
    if decoded.len() > MAX_BODY_BYTES {
        return Err(upstream_error(upstream, "response-too-large"));
    }
    Ok(decoded)
}

fn allowed_endpoint(url: &Url, method: &ProxyMethod, anisette: bool) -> bool {
    let path = url.path();
    if anisette {
        return url.query().is_none()
            && matches!(
                (method, path),
                (ProxyMethod::Get, "/v3/client_info") | (ProxyMethod::Post, "/v3/get_headers")
            );
    }
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
        body: super::ErrorResponse::InvalidRequest {
            error: "HTTPS proxy request violates destination, method, body or header policy",
        },
    }
}

fn request_too_large() -> ApiError {
    ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        body: super::ErrorResponse::InvalidRequest {
            error: "HTTPS proxy request is too large",
        },
    }
}

fn upstream_error(upstream: &'static str, failure: &'static str) -> ApiError {
    tag_failure(upstream, failure);
    ApiError {
        status: StatusCode::BAD_GATEWAY,
        body: super::ErrorResponse::UpstreamFailure {
            error: "HTTPS proxy upstream request failed",
        },
    }
}

fn upstream_timeout() -> ApiError {
    ApiError {
        status: StatusCode::GATEWAY_TIMEOUT,
        body: super::ErrorResponse::UpstreamTimeout {
            error: "HTTPS proxy request timed out",
        },
    }
}

fn transport_error(upstream: &'static str, stage: &'static str, error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        tag_failure(upstream, "timeout");
        return upstream_timeout();
    }
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(tls) = cause.downcast_ref::<rustls::Error>() {
            let failure = match tls {
                rustls::Error::InvalidCertificate(_) => "tls-certificate",
                rustls::Error::DecryptError
                | rustls::Error::AlertReceived(rustls::AlertDescription::BadRecordMac) => {
                    "tls-decrypt"
                }
                _ => "tls",
            };
            return upstream_error(upstream, failure);
        }
        source = cause
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
            .map(|inner| inner as &dyn Error)
            .or_else(|| cause.source());
    }
    if error.is_connect() {
        upstream_error(upstream, "connect")
    } else {
        upstream_error(upstream, stage)
    }
}

// Annotate the existing HTTP failure event without recording upstream request data.
fn tag_failure(upstream: &'static str, failure: &'static str) {
    sentry::configure_scope(|scope| {
        scope.set_tag("apple_proxy.upstream", upstream);
        scope.set_tag("apple_proxy.failure", failure);
    });
}
