use std::io;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use ring::hmac;
use zeroize::Zeroizing;

use super::ApiError;

const HEADER: &str = "x-app-key";
const VERIFICATION_MESSAGE: &[u8] = b"AppRelay app key verification";

#[derive(Clone)]
pub struct AppKey(hmac::Key);

impl AppKey {
    pub fn from_env() -> io::Result<Self> {
        let value = Zeroizing::new(std::env::var("APPRELAY_API_KEY").map_err(|_| {
            io::Error::other("APPRELAY_API_KEY is required and must be valid Unicode")
        })?);
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::other(
                "APPRELAY_API_KEY must contain exactly 64 hexadecimal characters (openssl rand -hex 32)",
            ));
        }
        Ok(Self(hmac::Key::new(hmac::HMAC_SHA256, value.as_bytes())))
    }
}

pub(super) async fn require_app_key(
    State(key): State<AppKey>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let unauthorized = || ApiError {
        status: StatusCode::UNAUTHORIZED,
        message: "a valid app key is required",
    };
    let mut values = request.headers().get_all(HEADER).iter();
    let value = values.next().ok_or_else(unauthorized)?;
    if values.next().is_some()
        || value.as_bytes().len() != 64
        || !value.as_bytes().iter().all(u8::is_ascii_hexdigit)
    {
        return Err(unauthorized());
    }
    // HMAC verification compares fixed-length tags in constant time, avoiding
    // direct string comparisons of the configured and supplied keys.
    let candidate = hmac::Key::new(hmac::HMAC_SHA256, value.as_bytes());
    let tag = hmac::sign(&candidate, VERIFICATION_MESSAGE);
    if hmac::verify(&key.0, VERIFICATION_MESSAGE, tag.as_ref()).is_err() {
        return Err(unauthorized());
    }
    Ok(next.run(request).await)
}
