use std::{collections::BTreeMap, io, time::Duration};

use aws_sdk_s3::{
    Client,
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation, retry::RetryConfig, timeout::TimeoutConfig,
    },
    presigning::PresigningConfig,
};

use crate::error::OperationError;

#[derive(Clone)]
pub(crate) struct R2Storage {
    client: Client,
    bucket: String,
}

#[derive(Debug)]
pub(crate) struct StorageError;

pub(crate) struct Upload {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

pub(crate) enum Verification {
    Valid,
    Missing,
    Mismatch,
}

impl R2Storage {
    pub fn new(
        account_id: &str,
        bucket: String,
        access_key: String,
        secret_key: String,
    ) -> io::Result<Self> {
        if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::other(
                "APPRELAY_R2_ACCOUNT_ID must be a 32-character hexadecimal account ID",
            ));
        }
        if !(3..=63).contains(&bucket.len())
            || !bucket
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || bucket.starts_with('-')
            || bucket.ends_with('-')
        {
            return Err(io::Error::other(
                "APPRELAY_R2_BUCKET must be a valid R2 bucket name",
            ));
        }
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(format!("https://{account_id}.r2.cloudflarestorage.com"))
            .force_path_style(true)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .credentials_provider(Credentials::new(access_key, secret_key, None, None, "r2"))
            .retry_config(RetryConfig::standard().with_max_attempts(2))
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(5))
                    .operation_timeout(Duration::from_secs(15))
                    .build(),
            )
            .build();
        Ok(Self {
            client: Client::from_conf(config),
            bucket,
        })
    }

    pub async fn upload(
        &self,
        key: &str,
        size: i64,
        sha256: &str,
        expires_in: Duration,
    ) -> Result<Upload, OperationError> {
        let request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("application/octet-stream")
            .content_length(size)
            .checksum_sha256(sha256)
            // R2 validates the checksum on PUT. Signed metadata lets HEAD verify
            // identity without relying on S3 HeadObject checksum-mode support.
            .metadata("sha256", sha256)
            // A still-valid upload URL must not overwrite a completed artifact.
            .if_none_match("*")
            .presigned(
                PresigningConfig::expires_in(expires_in)
                    .map_err(OperationError::at("configure_r2_upload_presign"))?,
            )
            .await
            .map_err(OperationError::at("presign_r2_upload"))?;
        let headers = request
            .headers()
            .filter(|(name, _)| !name.eq_ignore_ascii_case("host"))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        Ok(Upload {
            url: request.uri().to_owned(),
            headers,
        })
    }

    pub async fn verify(
        &self,
        key: &str,
        size: i64,
        sha256: &str,
    ) -> Result<Verification, OperationError> {
        let object = match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(object) => object,
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_not_found()) =>
            {
                return Ok(Verification::Missing);
            }
            Err(error) => return Err(OperationError::at("verify_r2_upload")(error)),
        };
        let matching_checksum = object
            .metadata()
            .and_then(|metadata| metadata.get("sha256"))
            .is_some_and(|value| value == sha256);
        if object.content_length() == Some(size) && matching_checksum {
            Ok(Verification::Valid)
        } else {
            Ok(Verification::Mismatch)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub async fn read_signed(
        &self,
        key: &str,
        size: i64,
        sha256: &str,
    ) -> Result<Vec<u8>, OperationError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use ring::digest::{Context, SHA256};

        let size = usize::try_from(size).map_err(OperationError::at("signed_ipa_size"))?;
        if !(1..=100 * 1024 * 1024).contains(&size) {
            return Err(OperationError::without_source("signed_ipa_size"));
        }
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(OperationError::at("signed_ipa_get_object"))?;
        if response.content_length() != Some(size as i64) {
            return Err(OperationError::without_source("signed_ipa_content_length"));
        }
        let mut stream = response.body;
        let mut bytes = Vec::with_capacity(size);
        let mut digest = Context::new(&SHA256);
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(OperationError::at("signed_ipa_read_body"))?
        {
            if chunk.len() > size - bytes.len() {
                return Err(OperationError::without_source("signed_ipa_content_length"));
            }
            digest.update(&chunk);
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != size || STANDARD.encode(digest.finish().as_ref()) != sha256 {
            return Err(OperationError::without_source("signed_ipa_integrity"));
        }
        Ok(bytes)
    }

    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map(|_| ())
            .map_err(|_| StorageError)
    }

    pub async fn download(&self, key: &str, seconds: u64) -> Result<String, OperationError> {
        self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(
                PresigningConfig::expires_in(Duration::from_secs(seconds))
                    .map_err(OperationError::at("configure_r2_download_presign"))?,
            )
            .await
            .map(|request| request.uri().to_owned())
            .map_err(OperationError::at("presign_r2_download"))
    }
}
