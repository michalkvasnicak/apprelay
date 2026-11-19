#![cfg(unix)]

mod support;

use apprelay_server::vpn::agent::{VpnState, testing::AgentRequest};
use axum::{
    body::Body,
    http::{HeaderValue, StatusCode},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{Duration, Utc};
use ring::digest::{SHA256, digest};
use serde_json::json;
use sqlx::PgPool;

use support::{DEVICE_KEY, LIFETIME_SECONDS, SERVER_KEY, TestApi, running_status};

const APP_KEY_ERROR: &str = "a valid app key is required";
const TOKEN_ERROR: &str = "a valid device bearer token is required";
const INVALID_KEY_ERROR: &str =
    "wireguard_public_key must be a nonzero, base64-encoded 32-byte key";
const INVALID_JSON_ERROR: &str = "invalid registration JSON; expected wireguard_public_key";

#[sqlx::test(migrations = "./migrations")]
async fn registers_a_new_device_and_provisions_its_reserved_address(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let device = api.register(DEVICE_KEY, None).await.registration();

    assert_eq!(device.address, "10.77.0.2/32");
    assert_eq!(
        device.wireguard.server_public_key,
        STANDARD.encode(SERVER_KEY)
    );
    assert_eq!(device.wireguard.allowed_ips, ["10.77.0.0/24"]);
    assert_eq!(device.wireguard.persistent_keepalive_seconds, 25);
    assert!(device.lifetime_seconds > 0);
    assert!(device.lifetime_seconds <= i64::from(LIFETIME_SECONDS));
    assert!(device.expires_at > Utc::now());
    assert_eq!(device.token.len(), 43);
    let token_bytes = URL_SAFE_NO_PAD.decode(&device.token).unwrap();
    assert_eq!(token_bytes.len(), 32);

    let stored = api.stored_device(device.device_id).await;
    assert_eq!(stored.id, device.device_id);
    assert_eq!(stored.wireguard_public_key, DEVICE_KEY);
    assert_eq!(stored.address, "10.77.0.2");
    assert_eq!(stored.expires_at, device.expires_at);
    assert_eq!(
        stored.bearer_token_hash.as_deref(),
        Some(digest(&SHA256, &token_bytes).as_ref()),
    );
    assert_eq!(api.device_count().await, 1);

    let requests = api.agent.requests();
    let [
        AgentRequest::Status,
        AgentRequest::EnsurePeer {
            public_key,
            address,
            expires_at,
        },
    ] = requests.as_slice()
    else {
        panic!("unexpected agent requests: {requests:?}");
    };
    assert_eq!(*public_key, DEVICE_KEY);
    assert_eq!(address.to_string(), stored.address);
    assert_eq!(*expires_at, device.expires_at.timestamp());
}

#[sqlx::test(migrations = "./migrations")]
async fn authenticated_retry_preserves_identity_token_address_and_expiry(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    let retry = api
        .register(DEVICE_KEY, Some(&first.token))
        .await
        .registration();

    assert_eq!(retry.device_id, first.device_id);
    assert_eq!(retry.token, first.token);
    assert_eq!(retry.address, first.address);
    assert_eq!(retry.expires_at, first.expires_at);
    assert_eq!(api.device_count().await, 1);
    assert_eq!(api.agent.requests().len(), 4);
}

#[sqlx::test(migrations = "./migrations")]
async fn matching_token_renews_a_device_that_expired_days_ago(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    let expired_at = api.expire(first.device_id).await;
    let before = api.database_now().await;
    let renewed = api
        .register(DEVICE_KEY, Some(&first.token))
        .await
        .registration();
    let after = api.database_now().await;

    assert_eq!(renewed.device_id, first.device_id);
    assert_eq!(renewed.token, first.token);
    assert_eq!(renewed.address, first.address);
    assert!(renewed.expires_at > expired_at);
    let lifetime = Duration::seconds(i64::from(LIFETIME_SECONDS));
    assert!(renewed.expires_at >= before + lifetime);
    assert!(renewed.expires_at <= after + lifetime);
    assert_eq!(
        api.stored_device(first.device_id).await.expires_at,
        renewed.expires_at
    );
    assert_eq!(api.device_count().await, 1);

    let requests = api.agent.requests();
    assert!(matches!(
        requests.last(),
        Some(AgentRequest::EnsurePeer { expires_at, .. })
            if *expires_at == renewed.expires_at.timestamp()
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn existing_device_requires_its_own_token(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    let other = api.register([2; 32], None).await.registration();
    let wrong_token = URL_SAFE_NO_PAD.encode([7; 32]);

    for token in [None, Some(wrong_token.as_str()), Some(other.token.as_str())] {
        api.register(DEVICE_KEY, token).await.assert_error(
            StatusCode::UNAUTHORIZED,
            "device_token_required",
            TOKEN_ERROR,
        );
    }
    let stored = api.stored_device(first.device_id).await;
    assert_eq!(stored.expires_at, first.expires_at);
    assert_eq!(api.device_count().await, 2);
    assert_eq!(
        api.agent
            .requests()
            .iter()
            .filter(|request| matches!(request, AgentRequest::EnsurePeer { .. }))
            .count(),
        2,
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_device_cannot_renew_without_its_matching_token(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    let expired_at = api.expire(first.device_id).await;
    let wrong_token = URL_SAFE_NO_PAD.encode([7; 32]);

    for token in [None, Some(wrong_token.as_str())] {
        api.register(DEVICE_KEY, token).await.assert_error(
            StatusCode::UNAUTHORIZED,
            "device_token_required",
            TOKEN_ERROR,
        );
    }
    assert_eq!(
        api.stored_device(first.device_id).await.expires_at,
        expired_at
    );
    assert_eq!(api.device_count().await, 1);
    assert_eq!(api.agent.requests().len(), 4);
}

#[sqlx::test(migrations = "./migrations")]
async fn legacy_device_without_a_token_hash_cannot_renew(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    sqlx::query("UPDATE devices SET bearer_token_hash = NULL WHERE id = $1")
        .bind(first.device_id)
        .execute(&api.pool)
        .await
        .unwrap();
    let expired_at = api.expire(first.device_id).await;

    api.register(DEVICE_KEY, Some(&first.token))
        .await
        .assert_error(
            StatusCode::UNAUTHORIZED,
            "device_token_required",
            TOKEN_ERROR,
        );
    let stored = api.stored_device(first.device_id).await;
    assert_eq!(stored.expires_at, expired_at);
    assert!(stored.bearer_token_hash.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_missing_duplicate_malformed_and_incorrect_app_keys(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let mut wrong_key = api.app_key.clone();
    wrong_key.replace_range(..1, if wrong_key.starts_with('0') { "1" } else { "0" });
    let non_hex_key = "g".repeat(64);
    let cases = [
        vec![],
        vec![api.app_key.as_str(), api.app_key.as_str()],
        vec!["short"],
        vec![non_hex_key.as_str()],
        vec![wrong_key.as_str()],
    ];
    for keys in cases {
        let mut request = api.request().body(Body::from("{}")).unwrap();
        request.headers_mut().remove("x-app-key");
        for key in keys {
            request
                .headers_mut()
                .append("x-app-key", HeaderValue::from_str(key).unwrap());
        }
        api.send(request).await.assert_error(
            StatusCode::UNAUTHORIZED,
            "app_key_required",
            APP_KEY_ERROR,
        );
    }
    assert_eq!(api.device_count().await, 0);
    assert!(api.agent.requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_malformed_bearer_headers_even_for_new_devices(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let body = json!({ "wireguard_public_key": STANDARD.encode(DEVICE_KEY) }).to_string();
    let invalid_alphabet = format!("Bearer {}", "*".repeat(43));
    for value in [
        "Basic abc",
        "Bearer",
        "Bearer short",
        invalid_alphabet.as_str(),
    ] {
        let request = api
            .request()
            .header("authorization", value)
            .body(Body::from(body.clone()))
            .unwrap();
        api.send(request).await.assert_error(
            StatusCode::UNAUTHORIZED,
            "device_token_required",
            TOKEN_ERROR,
        );
    }
    let token = format!("Bearer {}", URL_SAFE_NO_PAD.encode([7; 32]));
    let duplicate = api
        .request()
        .header("authorization", &token)
        .header("authorization", &token)
        .body(Body::from(body.clone()))
        .unwrap();
    api.send(duplicate).await.assert_error(
        StatusCode::UNAUTHORIZED,
        "device_token_required",
        TOKEN_ERROR,
    );
    let non_text = api
        .request()
        .header("authorization", HeaderValue::from_bytes(&[0xff]).unwrap())
        .body(Body::from(body))
        .unwrap();
    api.send(non_text).await.assert_error(
        StatusCode::UNAUTHORIZED,
        "device_token_required",
        TOKEN_ERROR,
    );
    assert_eq!(api.device_count().await, 0);
    assert!(api.agent.requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_invalid_json_fields_content_type_and_oversized_bodies(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let cases = [
        ("{".to_owned(), StatusCode::BAD_REQUEST),
        ("{}".to_owned(), StatusCode::UNPROCESSABLE_ENTITY),
        (
            json!({ "wireguard_public_key": 123 }).to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({ "wireguard_public_key": STANDARD.encode(DEVICE_KEY), "unknown": true })
                .to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (" ".repeat(1025), StatusCode::PAYLOAD_TOO_LARGE),
    ];
    for (body, status) in cases {
        let request = api.request().body(Body::from(body)).unwrap();
        api.send(request)
            .await
            .assert_error(status, "invalid_request", INVALID_JSON_ERROR);
    }
    let mut request = api.request().body(Body::from("{}")).unwrap();
    request.headers_mut().remove("content-type");
    api.send(request).await.assert_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "invalid_request",
        INVALID_JSON_ERROR,
    );
    assert_eq!(api.device_count().await, 0);
    assert!(api.agent.requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_zero_invalid_base64_and_wrong_length_public_keys(pool: PgPool) {
    let api = TestApi::new(pool).await;
    for key in [
        String::new(),
        "not a key".to_owned(),
        STANDARD.encode([0; 32]),
        STANDARD.encode([1; 31]),
        STANDARD.encode([1; 33]),
        "*".repeat(44),
    ] {
        let request = api
            .request()
            .body(Body::from(
                json!({ "wireguard_public_key": key }).to_string(),
            ))
            .unwrap();
        api.send(request).await.assert_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            INVALID_KEY_ERROR,
        );
    }
    assert_eq!(api.device_count().await, 0);
    assert!(api.agent.requests().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_the_servers_own_public_key(pool: PgPool) {
    let api = TestApi::new(pool).await;
    api.register(SERVER_KEY, None).await.assert_error(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        INVALID_KEY_ERROR,
    );
    assert_eq!(api.device_count().await, 0);
    assert!(matches!(
        api.agent.requests().as_slice(),
        [AgentRequest::Status]
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn status_failure_does_not_reserve_a_device(pool: PgPool) {
    let api = TestApi::new(pool).await;
    api.agent
        .enqueue_response(Err("agent unavailable".to_owned()));
    api.register(DEVICE_KEY, None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );
    assert_eq!(api.device_count().await, 0);
    assert!(matches!(
        api.agent.requests().as_slice(),
        [AgentRequest::Status]
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn stopped_or_unconfigured_vpn_does_not_reserve_a_device(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let mut stopped = running_status();
    stopped.state = VpnState::Stopped;
    api.agent.set_status(stopped);
    api.register(DEVICE_KEY, None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );

    let mut missing_key = running_status();
    missing_key.public_key = None;
    api.agent.set_status(missing_key);
    api.register(DEVICE_KEY, None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );
    assert_eq!(api.device_count().await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn unusable_vpn_subnets_do_not_reserve_devices(pool: PgPool) {
    let api = TestApi::new(pool).await;
    for (address, prefix) in [
        ([10, 77, 0, 1], 33),
        ([10, 77, 0, 1], 31),
        ([10, 77, 0, 0], 24),
        ([10, 77, 0, 255], 24),
    ] {
        let mut status = running_status();
        status.address = address.into();
        status.prefix_len = prefix;
        api.agent.set_status(status);
        api.register(DEVICE_KEY, None).await.assert_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            "internal server error",
        );
    }
    assert_eq!(api.device_count().await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn exhausted_address_pool_does_not_insert_another_device(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let mut status = running_status();
    status.prefix_len = 30;
    api.agent.set_status(status);
    let first = api.register(DEVICE_KEY, None).await.registration();
    assert_eq!(first.address, "10.77.0.2/32");
    api.register([2; 32], None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );
    assert_eq!(api.device_count().await, 1);
    assert_eq!(api.agent.requests().len(), 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn provisioning_failure_keeps_the_reservation_out_of_the_address_pool(pool: PgPool) {
    let api = TestApi::new(pool).await;
    api.agent.enqueue_response(Ok(running_status()));
    api.agent
        .enqueue_response(Err("peer provisioning failed".to_owned()));
    api.register(DEVICE_KEY, None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );
    assert_eq!(api.device_count().await, 1);
    let reserved: String =
        sqlx::query_scalar("SELECT host(address) FROM devices WHERE wireguard_public_key = $1")
            .bind(DEVICE_KEY.as_slice())
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert_eq!(reserved, "10.77.0.2");
    let next = api.register([2; 32], None).await.registration();
    assert_eq!(next.address, "10.77.0.3/32");
    assert_eq!(api.device_count().await, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn detects_vpn_configuration_changes_during_provisioning(pool: PgPool) {
    let api = TestApi::new(pool).await;
    api.agent.enqueue_response(Ok(running_status()));
    let mut changed = running_status();
    changed.public_key = Some(STANDARD.encode([8; 32]));
    api.agent.enqueue_response(Ok(changed));
    api.register(DEVICE_KEY, None).await.assert_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_server_error",
        "internal server error",
    );
    assert_eq!(api.device_count().await, 1);
    assert_eq!(api.agent.requests().len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_a_reserved_address_outside_the_current_subnet(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let first = api.register(DEVICE_KEY, None).await.registration();
    let mut changed = running_status();
    changed.address = [10, 78, 0, 1].into();
    api.agent.set_status(changed);
    api.register(DEVICE_KEY, Some(&first.token))
        .await
        .assert_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            "internal server error",
        );
    assert_eq!(
        api.stored_device(first.device_id).await.address,
        "10.77.0.2"
    );
    assert_eq!(api.agent.requests().len(), 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_registration_of_the_same_key_creates_only_one_device(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let (first, second) = tokio::join!(
        api.register(DEVICE_KEY, None),
        api.register(DEVICE_KEY, None),
    );
    let (created, rejected) = if first.status == StatusCode::CREATED {
        (first, second)
    } else {
        (second, first)
    };
    let device = created.registration();
    rejected.assert_error(
        StatusCode::UNAUTHORIZED,
        "device_token_required",
        TOKEN_ERROR,
    );
    assert_eq!(api.device_count().await, 1);
    assert_eq!(
        api.stored_device(device.device_id).await.address,
        "10.77.0.2"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_devices_receive_distinct_addresses(pool: PgPool) {
    let api = TestApi::new(pool).await;
    let (first, second) =
        tokio::join!(api.register(DEVICE_KEY, None), api.register([2; 32], None),);
    let first = first.registration();
    let second = second.registration();
    assert_ne!(first.device_id, second.device_id);
    assert_ne!(first.token, second.token);
    let mut addresses = [first.address, second.address];
    addresses.sort();
    assert_eq!(addresses, ["10.77.0.2/32", "10.77.0.3/32"]);
    assert_eq!(api.device_count().await, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn registration_rate_limit_returns_retry_after_and_ignores_spoofed_forwarding(pool: PgPool) {
    let api = TestApi::new(pool).await;
    // Invalid bodies consume rate-limit capacity without touching the agent or DB.
    for index in 0..10 {
        let request = api
            .request()
            .header("cf-connecting-ip", format!("192.0.2.{}", index + 1))
            .body(Body::from("{}"))
            .unwrap();
        api.send(request).await.assert_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_request",
            INVALID_JSON_ERROR,
        );
    }
    let response = api.register(DEVICE_KEY, None).await;
    response.assert_error(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "too many requests",
    );
    assert!(
        response.headers["retry-after"]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(api.device_count().await, 0);
    assert!(api.agent.requests().is_empty());
}
