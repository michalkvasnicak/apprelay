use std::{collections::BTreeMap, io, time::Duration};

use aws_sdk_s3::{
    Client,
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation, retry::RetryConfig, timeout::TimeoutConfig,
    },
    presigning::PresigningConfig,
};

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

    pub async fn upload(&self, key: &str, size: i64, sha256: &str) -> Result<Upload, StorageError> {
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
                PresigningConfig::expires_in(Duration::from_secs(900)).map_err(|_| StorageError)?,
            )
            .await
            .map_err(|_| StorageError)?;
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
    ) -> Result<Verification, StorageError> {
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
            Err(_) => return Err(StorageError),
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

    pub async fn download(&self, key: &str, seconds: u64) -> Result<String, StorageError> {
        self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(
                PresigningConfig::expires_in(Duration::from_secs(seconds))
                    .map_err(|_| StorageError)?,
            )
            .await
            .map(|request| request.uri().to_owned())
            .map_err(|_| StorageError)
    }
}
