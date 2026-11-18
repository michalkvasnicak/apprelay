use std::{
    net::{Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    sync::atomic::{AtomicU32, Ordering},
};

use apprelay_server::{
    agent::{VpnState, VpnStatus, testing::FakeAgent},
    db::{
        devices::DeviceRepository, pairing_attempts::PairingAttemptRepository,
        pairing_codes::PairingCodeRepository, push_tokens::PushTokenRepository,
    },
    http::{self, AppKey, HttpsProxy, RegistrationRateLimiter, RouterConfig},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode, request::Builder},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const LIFETIME_SECONDS: u32 = 3600;
pub const DEVICE_KEY: [u8; 32] = [1; 32];
pub const SERVER_KEY: [u8; 32] = [9; 32];
static NEXT_IP: AtomicU32 = AtomicU32::new(1);

pub fn running_status() -> VpnStatus {
    VpnStatus {
        state: VpnState::Running,
        interface: Some("test-vpn".to_owned()),
        address: Ipv4Addr::new(10, 77, 0, 1),
        prefix_len: 24,
        listen_port: 51820,
        public_key: Some(STANDARD.encode(SERVER_KEY)),
    }
}

pub struct TestApi {
    pub pool: PgPool,
    pub agent: FakeAgent,
    pub app_key: String,
    router: Router,
    peer: SocketAddr,
}

impl TestApi {
    pub async fn new(pool: PgPool) -> Self {
        let agent = FakeAgent::new(running_status());
        let router = http::router(
            DeviceRepository::new(pool.clone(), NonZeroU32::new(LIFETIME_SECONDS).unwrap()),
            agent.client(),
            PushTokenRepository::from_env(pool.clone()).unwrap(),
            PairingCodeRepository::from_env(pool.clone()).unwrap(),
            PairingAttemptRepository::new(pool.clone()),
            RouterConfig {
                app_key: AppKey::from_env().unwrap(),
                registration_limiter: RegistrationRateLimiter::from_env().await.unwrap(),
                https_proxy: HttpsProxy::new("https://anisette.example.com").unwrap(),
                artifacts: None,
            },
        );
        // Rate limits are shared in Redis. Each fixture gets its own client IP,
        // while repeated requests within a fixture exercise the same bucket.
        let ip = Ipv4Addr::from(0x7f00_0000 + NEXT_IP.fetch_add(1, Ordering::Relaxed));
        Self {
            pool,
            agent,
            app_key: std::env::var("APPRELAY_API_KEY").unwrap(),
            router,
            peer: SocketAddr::from((ip, 12345)),
        }
    }

    pub fn request(&self) -> Builder {
        Request::post("/v1/devices")
            .header("x-app-key", &self.app_key)
            .header("content-type", "application/json")
    }

    pub async fn send(&self, mut request: Request<Body>) -> TestResponse {
        request.extensions_mut().insert(ConnectInfo(self.peer));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, 16 * 1024).await.unwrap();
        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body: serde_json::from_slice(&bytes).unwrap(),
        }
    }

    pub async fn register(&self, public_key: [u8; 32], token: Option<&str>) -> TestResponse {
        let mut request = self.request();
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        self.send(
            request
                .body(Body::from(
                    json!({ "wireguard_public_key": STANDARD.encode(public_key) }).to_string(),
                ))
                .unwrap(),
        )
        .await
    }

    pub async fn device_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM devices")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn stored_device(&self, id: Uuid) -> StoredDevice {
        sqlx::query_as(
            "SELECT id, wireguard_public_key, host(address) AS address,
                    expires_at, bearer_token_hash FROM devices WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    pub async fn database_now(&self) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn expire(&self, id: Uuid) -> DateTime<Utc> {
        sqlx::query_scalar(
            "UPDATE devices SET expires_at = clock_timestamp() - INTERVAL '3 days'
             WHERE id = $1 RETURNING expires_at",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl TestResponse {
    pub fn registration(self) -> Registration {
        assert_eq!(self.status, StatusCode::CREATED, "{}", self.body);
        assert_eq!(self.headers["cache-control"], "no-store");
        serde_json::from_value(self.body).unwrap()
    }

    pub fn assert_error(&self, status: StatusCode, code: &str, message: &str) {
        assert_eq!(self.status, status, "{}", self.body);
        assert_eq!(self.body, json!({ "code": code, "error": message }));
        assert_eq!(self.headers["cache-control"], "no-store");
    }
}

#[derive(Debug, Deserialize)]
pub struct Registration {
    pub device_id: Uuid,
    pub token: String,
    pub address: String,
    pub expires_at: DateTime<Utc>,
    pub lifetime_seconds: i64,
    pub wireguard: WireguardConfiguration,
}

#[derive(Debug, Deserialize)]
pub struct WireguardConfiguration {
    pub server_public_key: String,
    pub allowed_ips: Vec<String>,
    pub persistent_keepalive_seconds: u16,
}

#[derive(sqlx::FromRow)]
pub struct StoredDevice {
    pub id: Uuid,
    pub wireguard_public_key: Vec<u8>,
    pub address: String,
    pub expires_at: DateTime<Utc>,
    pub bearer_token_hash: Option<Vec<u8>>,
}
