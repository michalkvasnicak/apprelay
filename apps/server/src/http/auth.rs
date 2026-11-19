use axum::http::{HeaderMap, header::AUTHORIZATION};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest::{SHA256, digest},
    rand::{SecureRandom, SystemRandom},
};

use super::ApiError;

pub(super) struct Credential {
    pub token: String,
    pub hash: [u8; 32],
}

impl Credential {
    pub fn generate() -> Result<Self, ApiError> {
        let mut bytes = [0u8; 32];
        SystemRandom::new().fill(&mut bytes).map_err(|_| {
            ApiError::Internal(crate::error::OperationError::without_source(
                "generate_credential",
            ))
        })?;
        Ok(Self::from_bytes(bytes))
    }

    fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            token: URL_SAFE_NO_PAD.encode(bytes),
            hash: digest(&SHA256, &bytes).as_ref().try_into().unwrap(),
        }
    }

    pub fn optional(headers: &HeaderMap) -> Result<Option<Self>, ApiError> {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(ApiError::unauthorized());
        }
        let value = value.to_str().map_err(|_| ApiError::unauthorized())?;
        let (scheme, token) = value.split_once(' ').ok_or_else(ApiError::unauthorized)?;
        if !scheme.eq_ignore_ascii_case("Bearer") || token.len() != 43 {
            return Err(ApiError::unauthorized());
        }
        let bytes: [u8; 32] = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| ApiError::unauthorized())?
            .try_into()
            .map_err(|_| ApiError::unauthorized())?;
        Ok(Some(Self::from_bytes(bytes)))
    }

    pub fn required(headers: &HeaderMap) -> Result<Self, ApiError> {
        Self::optional(headers)?.ok_or_else(ApiError::unauthorized)
    }
}
