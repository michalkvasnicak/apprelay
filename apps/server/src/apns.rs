use std::{fmt::Write as _, io, sync::Mutex, time::Duration};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use reqwest::{Client, StatusCode, Version, redirect::Policy};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::db::pairing_codes::{Notification, NotificationContent};

pub(crate) struct ApnsClient {
    http: Client,
    signing_key: EcdsaKeyPair,
    key_id: String,
    team_id: String,
    topic: String,
    jwt: Mutex<Option<(i64, String)>>,
}

pub(crate) enum SendOutcome {
    Accepted,
    Expired,
    InvalidToken(Option<i64>),
    Rejected,
}

#[derive(Deserialize)]
struct ApnsError {
    reason: String,
    timestamp: Option<i64>,
}

fn required(name: &str) -> io::Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| io::Error::other(format!("{name} is required")))
}

impl ApnsClient {
    pub(crate) fn from_env() -> io::Result<Self> {
        let path = required("APPRELAY_APNS_SIGNING_KEY_FILE")?;
        let pem = Zeroizing::new(std::fs::read_to_string(path)?);
        let body = pem
            .trim()
            .strip_prefix("-----BEGIN PRIVATE KEY-----")
            .and_then(|value| value.strip_suffix("-----END PRIVATE KEY-----"))
            .ok_or_else(|| io::Error::other("APNs signing key must be a PKCS#8 PEM .p8 file"))?;
        let encoded = Zeroizing::new(body.split_whitespace().collect::<String>());
        let der = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(|_| io::Error::other("invalid APNs signing key encoding"))?,
        );
        let signing_key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new())
                .map_err(|_| {
                    io::Error::other("APNs signing key must be a valid P-256 PKCS#8 key")
                })?;
        let http = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| io::Error::other("could not initialize APNs HTTP client"))?;
        Ok(Self {
            http,
            signing_key,
            key_id: required("APPRELAY_APNS_KEY_ID")?,
            team_id: required("APPRELAY_APNS_TEAM_ID")?,
            topic: required("APPRELAY_APNS_TOPIC")?,
            jwt: Mutex::new(None),
        })
    }

    fn authorization(&self) -> io::Result<String> {
        let mut cached = self
            .jwt
            .lock()
            .map_err(|_| io::Error::other("APNs signing cache unavailable"))?;
        let now = Utc::now().timestamp();
        if let Some((issued, jwt)) = cached.as_ref()
            && now >= *issued
            && now - issued < 3000
        {
            return Ok(jwt.clone());
        }
        let header =
            URL_SAFE_NO_PAD.encode(json!({"alg": "ES256", "kid": self.key_id}).to_string());
        let claims = URL_SAFE_NO_PAD.encode(json!({"iss": self.team_id, "iat": now}).to_string());
        let message = format!("{header}.{claims}");
        let signature = self
            .signing_key
            .sign(&SystemRandom::new(), message.as_bytes())
            .map_err(|_| io::Error::other("could not sign APNs provider token"))?;
        let jwt = format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()));
        *cached = Some((now, jwt.clone()));
        Ok(jwt)
    }

    pub(crate) async fn send(
        &self,
        notification_id: Uuid,
        notification: &Notification,
    ) -> io::Result<SendOutcome> {
        let host = match notification.environment.as_str() {
            "sandbox" => "api.sandbox.push.apple.com",
            "production" => "api.push.apple.com",
            _ => return Err(io::Error::other("invalid stored APNs environment")),
        };
        if notification.expires_at.timestamp() <= Utc::now().timestamp() {
            return Ok(SendOutcome::Expired);
        }
        let remaining = (notification.expires_at - Utc::now()).to_std().ok();
        let Some(remaining) = remaining.filter(|duration| !duration.is_zero()) else {
            return Ok(SendOutcome::Expired);
        };
        let mut token = String::with_capacity(notification.token.len() * 2);
        for byte in &notification.token {
            write!(&mut token, "{byte:02x}")
                .map_err(|_| io::Error::other("could not encode APNs token"))?;
        }
        let (kind, title, body) = match &notification.content {
            NotificationContent::Code(code) => (
                "code",
                "Pair with AppRelay",
                format!("Enter {} to pair your device.", code.code),
            ),
            NotificationContent::Started => (
                "started",
                "Pairing started",
                "You have 60 seconds to finish pairing your device.".to_owned(),
            ),
            NotificationContent::Success => (
                "success",
                "Pair setup complete",
                "Pair setup is complete.".to_owned(),
            ),
            NotificationContent::Timeout => (
                "timeout",
                "Pairing failed",
                "Pairing wasn't completed within one minute.".to_owned(),
            ),
            NotificationContent::Install { outcome, .. } => match outcome {
                None => (
                    "started",
                    "Installation started",
                    "AppRelay is sending the app to this iPhone.".to_owned(),
                ),
                Some(true) => (
                    "success",
                    "App installed",
                    "AppRelay finished installing the app.".to_owned(),
                ),
                Some(false) => (
                    "failed",
                    "Installation failed",
                    "AppRelay couldn't confirm installation. Check the VPN and try again."
                        .to_owned(),
                ),
            },
        };
        let mut payload = json!({
            "aps": {"alert": {"title": title, "body": body}, "sound": "default"},
            "expires_at": notification.expires_at,
        });
        let collapse = if let NotificationContent::Install { job_id, .. } = &notification.content {
            payload["install_job_id"] = json!(job_id);
            format!("install-{kind}-{job_id}")
        } else {
            payload["pairing_attempt_id"] = json!(notification_id);
            format!("pairing-{kind}-{}", notification.device_id)
        };
        let response = self
            .http
            .post(format!("https://{host}/3/device/{token}"))
            .version(Version::HTTP_2)
            .bearer_auth(self.authorization()?)
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header(
                "apns-expiration",
                notification.expires_at.timestamp().to_string(),
            )
            .header("apns-id", notification_id.to_string())
            .header("apns-collapse-id", collapse)
            .timeout(remaining.min(Duration::from_secs(5)))
            .json(&payload)
            .send()
            .await
            // reqwest errors can include the URL, which contains the APNs token.
            .map_err(|_| io::Error::other("APNs request failed"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(SendOutcome::Accepted);
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(io::Error::other("APNs temporarily unavailable"));
        }
        let error = response.json::<ApnsError>().await.ok();
        if status == StatusCode::GONE
            || (status == StatusCode::BAD_REQUEST
                && error.as_ref().is_some_and(|e| e.reason == "BadDeviceToken"))
        {
            return Ok(SendOutcome::InvalidToken(error.and_then(|e| e.timestamp)));
        }
        if status == StatusCode::FORBIDDEN
            && error.is_some_and(|e| e.reason == "ExpiredProviderToken")
        {
            if let Ok(mut cached) = self.jwt.lock() {
                *cached = None;
            }
            return Err(io::Error::other("APNs provider token expired"));
        }
        Ok(SendOutcome::Rejected)
    }
}
