use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};
use fred::{
    interfaces::ClientLike,
    prelude::Pool,
    types::{
        Builder,
        config::{Config, ReconnectPolicy},
    },
};
use ipnet::IpNet;
use tokio::time::timeout;
use tower_rate_limit_fred::rate_limit;
use zeroize::Zeroizing;

use super::ApiError;

const CAPACITY: u16 = 10;
const EMISSION_INTERVAL: Duration = Duration::from_secs(6);

#[derive(Clone)]
pub struct RegistrationRateLimiter {
    pool: Pool,
    trusted_proxies: Vec<IpNet>,
}

impl RegistrationRateLimiter {
    pub async fn from_env() -> io::Result<Self> {
        let proxies = std::env::var("APPRELAY_TRUSTED_PROXY_CIDRS")
            .or_else(|error| match error {
                std::env::VarError::NotPresent => Ok(String::new()),
                error => Err(error),
            })
            .map_err(|_| io::Error::other("APPRELAY_TRUSTED_PROXY_CIDRS must be valid Unicode"))?;
        let trusted_proxies = if proxies.trim().is_empty() {
            Vec::new()
        } else {
            proxies
                .split(',')
                .map(|value| {
                    value.trim().parse::<IpNet>().map_err(|_| {
                        io::Error::other(
                            "APPRELAY_TRUSTED_PROXY_CIDRS must be comma-separated IP CIDRs",
                        )
                    })
                })
                .collect::<io::Result<Vec<_>>>()?
        };
        let url =
            Zeroizing::new(std::env::var("REDIS_URL").map_err(|_| {
                io::Error::other("REDIS_URL is required and must be valid Unicode")
            })?);
        let config = Config::from_url(&url)
            .map_err(|_| io::Error::other("REDIS_URL must be a valid Redis connection URL"))?;
        let pool = Builder::from_config(config)
            .set_policy(ReconnectPolicy::new_exponential(0, 100, 5000, 2))
            .with_performance_config(|config| {
                config.default_command_timeout = Duration::from_millis(300);
            })
            .build_pool(2)
            .map_err(|_| io::Error::other("could not create rate-limiting Redis pool"))?;
        timeout(Duration::from_secs(5), pool.init())
            .await
            .map_err(|_| io::Error::other("rate-limiting Redis connection timed out"))?
            .map_err(|_| io::Error::other("could not connect to rate-limiting Redis"))?;
        Ok(Self {
            pool,
            trusted_proxies,
        })
    }

    fn client_ip(&self, request: &Request) -> Result<IpAddr, ApiError> {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or_else(|| {
                ApiError::Internal(crate::error::OperationError::without_source(
                    "missing_http_connect_info",
                ))
            })?
            .0
            .ip()
            .to_canonical();
        if !self
            .trusted_proxies
            .iter()
            .any(|proxy| proxy.contains(&peer))
        {
            // Never use client-controlled forwarding headers from direct peers.
            return Ok(peer);
        }
        let invalid = || ApiError::Public {
            status: StatusCode::BAD_REQUEST,
            body: super::ErrorResponse::InvalidRequest {
                error: "trusted proxy must supply one valid CF-Connecting-IP header",
            },
        };
        let mut values = request.headers().get_all("cf-connecting-ip").iter();
        let value = values.next().ok_or_else(invalid)?;
        if values.next().is_some() {
            return Err(invalid());
        }
        value
            .to_str()
            .map_err(|_| invalid())?
            .parse::<IpAddr>()
            .map(|ip| ip.to_canonical())
            .map_err(|_| invalid())
    }
}

pub(super) async fn limit_registration(
    State(limiter): State<RegistrationRateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    limit(
        limiter,
        request,
        next,
        "registration",
        CAPACITY,
        EMISSION_INTERVAL,
    )
    .await
}

pub(super) async fn limit_pairing_attempts(
    State(limiter): State<RegistrationRateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let polling = request.method() == axum::http::Method::GET;
    let (scope, capacity, emission) = if polling {
        ("pairing-poll", 60, Duration::from_secs(1))
    } else {
        ("pairing-create", CAPACITY, EMISSION_INTERVAL)
    };
    limit(limiter, request, next, scope, capacity, emission).await
}

pub(super) async fn limit_artifacts(
    State(limiter): State<RegistrationRateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let (scope, capacity, emission) = if request.method() == axum::http::Method::GET {
        ("artifact-read", 60, Duration::from_secs(1))
    } else {
        ("artifact-write", CAPACITY, EMISSION_INTERVAL)
    };
    limit(limiter, request, next, scope, capacity, emission).await
}

pub(super) async fn limit_install_jobs(
    State(limiter): State<RegistrationRateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let (scope, capacity, emission) = match *request.method() {
        axum::http::Method::GET | axum::http::Method::HEAD => {
            ("install-poll", 60, Duration::from_secs(1))
        }
        _ => ("install-write", CAPACITY, EMISSION_INTERVAL),
    };
    limit(limiter, request, next, scope, capacity, emission).await
}

async fn limit(
    limiter: RegistrationRateLimiter,
    request: Request,
    next: Next,
    scope: &str,
    capacity: u16,
    emission: Duration,
) -> Result<Response, ApiError> {
    let ip = limiter.client_ip(&request)?;
    let key = format!("apprelay:rate-limit:{scope}:{ip}");
    // The library owns the GCRA algorithm and atomic Redis script. Bound the
    // entire check, including script loading, rather than just each command.
    match timeout(
        Duration::from_secs(1),
        rate_limit(&limiter.pool, key, capacity, emission),
    )
    .await
    {
        Ok(Ok(None)) => Ok(next.run(request).await),
        Ok(Ok(Some(retry_after))) => {
            let mut response = ApiError::Public {
                status: StatusCode::TOO_MANY_REQUESTS,
                body: super::ErrorResponse::RateLimited {
                    error: "too many requests",
                },
            }
            .into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, retry_after.into());
            Ok(response)
        }
        Ok(Err(error)) => Err(ApiError::internal("check_http_rate_limit")(error)),
        Err(error) => Err(ApiError::internal("check_http_rate_limit_timeout")(error)),
    }
}
