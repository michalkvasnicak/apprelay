//! Version 1: AES-256-GCM over a JSON envelope, bound to the device UUID.
//! Neither plaintext nor the encryption key enters SQL/Temporal.

use std::{io, net::Ipv4Addr};

use chrono::{DateTime, Utc};
use idevice::remote_pairing::RpPairingFile;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{error::OperationError, pairing::PairingRecord, secrets::SecretKey};

const FORMAT_VERSION: i16 = 1;

pub(crate) struct PairingStore {
    pool: PgPool,
    key: SecretKey,
}

// Borrow the source data to avoid unnecessary copies of secrets. The plist and
// final serialized envelope are zeroized after encryption, including on errors.
#[derive(Serialize)]
struct StoredPairing<'a> {
    pairing_file: &'a [u8],
    host: StoredHost<'a>,
    peer: StoredPeer<'a>,
    peer_public_key: Option<[u8; 32]>,
}

#[derive(Serialize)]
struct StoredHost<'a> {
    name: &'a str,
    model: &'a str,
    udid: &'a str,
    identifier: &'a str,
    wire_protocol_version: u8,
    alt_irk: &'a [u8; 16],
    allows_pinless_pairing: bool,
}

#[derive(Serialize)]
struct StoredPeer<'a> {
    account_id: &'a str,
    alt_irk: &'a [u8],
    model: &'a str,
    name: &'a str,
    remotepairing_udid: &'a str,
}

fn associated_data(device_id: Uuid) -> Vec<u8> {
    let mut data = b"apprelay/device-pairing/".to_vec();
    data.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    data.extend_from_slice(device_id.as_bytes());
    data
}

#[derive(Deserialize)]
struct InstallCredentials {
    pairing_file: Vec<u8>,
    peer_public_key: Option<[u8; 32]>,
}

#[derive(sqlx::FromRow)]
struct InstallTargetRow {
    device_id: Uuid,
    address: String,
    remote_pairing_port: i32,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    expires_at: DateTime<Utc>,
}

pub(crate) struct InstallTarget {
    pub address: Ipv4Addr,
    pub port: u16,
    pub record: RpPairingFile,
    pub peer_key: [u8; 32],
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InstallTargetError {
    #[error("install target lookup failed")]
    Lookup(#[source] sqlx::Error),
    #[error("paired phone unavailable; connect its VPN")]
    Unavailable,
    #[error("could not decrypt stored pairing")]
    Decrypt(#[source] io::Error),
    #[error("invalid stored pairing")]
    Decode(#[source] serde_json::Error),
    #[error("pair the phone again before installing")]
    MissingPeerKey,
    #[error("invalid stored pairing")]
    InvalidRecord(#[source] idevice::IdeviceError),
    #[error("invalid device address")]
    Address(#[source] std::net::AddrParseError),
    #[error("invalid device port")]
    Port(#[source] std::num::TryFromIntError),
}

impl From<InstallTargetError> for OperationError {
    fn from(error: InstallTargetError) -> Self {
        let operation = match &error {
            InstallTargetError::Lookup(_) => "target_lookup",
            InstallTargetError::Unavailable => "target_unavailable",
            InstallTargetError::Decrypt(_) => "target_decrypt",
            InstallTargetError::Decode(_) => "target_decode",
            InstallTargetError::MissingPeerKey => "target_missing_peer_key",
            InstallTargetError::InvalidRecord(_) => "target_invalid_record",
            InstallTargetError::Address(_) => "target_invalid_address",
            InstallTargetError::Port(_) => "target_invalid_port",
        };
        Self::at(operation)(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not configure pairing encryption key")]
pub(crate) struct PairingStoreConfigError(#[source] io::Error);

impl From<PairingStoreConfigError> for OperationError {
    fn from(error: PairingStoreConfigError) -> Self {
        Self::at("configure_pairing_store")(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PairingWriteError {
    #[error("could not prepare pairing attempt for verification")]
    PrepareVerification(#[source] sqlx::Error),
    #[error("could not verify pairing port")]
    VerifyPort(#[source] sqlx::Error),
    #[error("could not serialize pairing credentials")]
    SerializeCredentials(#[source] serde_json::Error),
    #[error("could not encrypt pairing credentials")]
    EncryptCredentials(#[source] io::Error),
    #[error("could not start pairing transaction")]
    Begin(#[source] sqlx::Error),
    #[error("could not lock pairing device")]
    LockDevice(#[source] sqlx::Error),
    #[error("pairing device is unavailable")]
    DeviceUnavailable,
    #[error("could not complete public pairing attempt")]
    CompletePublicAttempt(#[source] sqlx::Error),
    #[error("public pairing attempt is unavailable")]
    PublicAttemptUnavailable,
    #[error("could not save pairing credentials")]
    UpsertCredentials(#[source] sqlx::Error),
    #[error("a newer pairing superseded this pairing")]
    Superseded,
    #[error("could not complete pairing attempt")]
    CompleteAttempt(#[source] sqlx::Error),
    #[error("pairing attempt is unavailable")]
    AttemptUnavailable,
    #[error("could not commit pairing transaction")]
    Commit(#[source] sqlx::Error),
}

impl From<PairingWriteError> for OperationError {
    fn from(error: PairingWriteError) -> Self {
        let operation = match &error {
            PairingWriteError::PrepareVerification(_) => "prepare_pairing_verification",
            PairingWriteError::VerifyPort(_) => "verify_pairing_port",
            PairingWriteError::SerializeCredentials(_) => "serialize_pairing_credentials",
            PairingWriteError::EncryptCredentials(_) => "encrypt_pairing_credentials",
            PairingWriteError::Begin(_) => "begin_save_pairing",
            PairingWriteError::LockDevice(_) => "lock_pairing_device",
            PairingWriteError::DeviceUnavailable => "pairing_device_unavailable",
            PairingWriteError::CompletePublicAttempt(_) => "complete_public_pairing_attempt",
            PairingWriteError::PublicAttemptUnavailable => "public_pairing_attempt_unavailable",
            PairingWriteError::UpsertCredentials(_) => "upsert_pairing_credentials",
            PairingWriteError::Superseded => "pairing_superseded",
            PairingWriteError::CompleteAttempt(_) => "complete_pairing_attempt",
            PairingWriteError::AttemptUnavailable => "pairing_attempt_unavailable",
            PairingWriteError::Commit(_) => "commit_save_pairing",
        };
        Self::at(operation)(error)
    }
}

impl PairingStore {
    pub(crate) async fn install_target(
        &self,
        job_id: Uuid,
    ) -> Result<InstallTarget, InstallTargetError> {
        let row = sqlx::query_as::<_, InstallTargetRow>(
            "SELECT d.id AS device_id, host(d.address) AS address, p.remote_pairing_port,
                 p.nonce, p.ciphertext, LEAST(d.expires_at, j.expires_at) AS expires_at
             FROM install_jobs j JOIN devices d ON d.id = j.device_id AND d.expires_at = j.device_expires_at
             JOIN device_pairings p ON p.device_id = d.id AND p.attempt_started_at = j.pairing_started_at
             WHERE j.id = $1 AND j.queued_at IS NOT NULL AND j.finished_at IS NULL
               AND j.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND p.remote_pairing_port IS NOT NULL AND p.remote_pairing_port_expires_at > clock_timestamp()
               AND p.format_version = $2",
        )
        .bind(job_id)
        .bind(FORMAT_VERSION)
        .fetch_optional(&self.pool)
        .await
        .map_err(InstallTargetError::Lookup)?
        .ok_or(InstallTargetError::Unavailable)?;
        let plaintext = self
            .key
            .open(&associated_data(row.device_id), &row.nonce, row.ciphertext)
            .map_err(InstallTargetError::Decrypt)?;
        let stored: InstallCredentials =
            serde_json::from_slice(&plaintext).map_err(InstallTargetError::Decode)?;
        let pairing_file = Zeroizing::new(stored.pairing_file);
        let peer_key = stored
            .peer_public_key
            .ok_or(InstallTargetError::MissingPeerKey)?;
        let record =
            RpPairingFile::from_bytes(&pairing_file).map_err(InstallTargetError::InvalidRecord)?;
        Ok(InstallTarget {
            address: row.address.parse().map_err(InstallTargetError::Address)?,
            port: u16::try_from(row.remote_pairing_port).map_err(InstallTargetError::Port)?,
            record,
            peer_key,
            expires_at: row.expires_at,
        })
    }

    /// Fail startup on missing/malformed configuration; never print the value.
    pub(crate) fn from_env(pool: PgPool) -> Result<Self, PairingStoreConfigError> {
        Ok(Self {
            pool,
            key: SecretKey::from_env().map_err(PairingStoreConfigError)?,
        })
    }

    pub(crate) async fn save_pending(
        &self,
        device_id: Uuid,
        attempt_id: Uuid,
        expected_expiry: DateTime<Utc>,
        attempt_started_at: DateTime<Utc>,
        verification_expires_at: DateTime<Utc>,
        record: &PairingRecord,
    ) -> Result<(), PairingWriteError> {
        let pairing_file = Zeroizing::new(record.pairing_file.to_bytes());
        let host = &record.host_info;
        let peer = &record.peer;
        let stored = StoredPairing {
            pairing_file: &pairing_file,
            peer_public_key: record.peer_public_key,
            host: StoredHost {
                name: &host.name,
                model: &host.model,
                udid: &host.udid,
                identifier: &host.identifier,
                wire_protocol_version: host.wire_protocol_version,
                // This is distinct from the device's altIRK in RpPairingFile.
                alt_irk: &host.alt_irk,
                allows_pinless_pairing: host.allows_pinless_pairing,
            },
            peer: StoredPeer {
                account_id: &peer.account_id,
                alt_irk: &peer.alt_irk,
                model: &peer.model,
                name: &peer.name,
                remotepairing_udid: &peer.remotepairing_udid,
            },
        };
        let plaintext =
            serde_json::to_vec(&stored).map_err(PairingWriteError::SerializeCredentials)?;
        // Domain separation + format version + device ID prevents moving a
        // ciphertext to a different device or interpreting it as another format.
        let (nonce, ciphertext) = self
            .key
            .seal(&associated_data(device_id), plaintext)
            .map_err(PairingWriteError::EncryptCredentials)?;

        let mut transaction = self.lock_device(device_id, expected_expiry).await?;
        // Keep the attempt active during verification, but never extend the PIN window.
        let pending = sqlx::query(
            "UPDATE pairing_attempts SET expires_at = LEAST(device_expires_at, $4),
                 code_id = NULL, nonce = NULL, ciphertext = NULL
             WHERE id = $1 AND device_id = $2 AND device_expires_at = $3
               AND ended_at IS NULL AND expires_at > clock_timestamp()
               AND $4 > clock_timestamp()",
        )
        .bind(attempt_id)
        .bind(device_id)
        .bind(expected_expiry)
        .bind(verification_expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(PairingWriteError::PrepareVerification)?;
        if pending.rows_affected() != 1 {
            return Err(PairingWriteError::AttemptUnavailable);
        }
        let saved = sqlx::query(
            "INSERT INTO device_pairings
                 (device_id, format_version, nonce, ciphertext, attempt_started_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, clock_timestamp())
             ON CONFLICT (device_id) DO UPDATE SET
                 format_version = EXCLUDED.format_version,
                 nonce = EXCLUDED.nonce,
                 ciphertext = EXCLUDED.ciphertext,
                 attempt_started_at = EXCLUDED.attempt_started_at,
                 updated_at = clock_timestamp(),
                 verified_at = NULL,
                 remote_pairing_port = NULL,
                 remote_pairing_port_expires_at = NULL
             WHERE device_pairings.attempt_started_at < EXCLUDED.attempt_started_at",
        )
        .bind(device_id)
        .bind(FORMAT_VERSION)
        .bind(nonce.as_slice())
        .bind(ciphertext.as_slice())
        .bind(attempt_started_at)
        .execute(&mut *transaction)
        .await
        .map_err(PairingWriteError::UpsertCredentials)?;
        if saved.rows_affected() != 1 {
            return Err(PairingWriteError::Superseded);
        }
        transaction
            .commit()
            .await
            .map_err(PairingWriteError::Commit)
    }

    pub(crate) async fn complete_if_reported(
        &self,
        device_id: Uuid,
        attempts: (Uuid, Option<Uuid>),
        expected_expiry: DateTime<Utc>,
        attempt_started_at: DateTime<Utc>,
        record: &PairingRecord,
    ) -> Result<bool, PairingWriteError> {
        let (attempt_id, public_attempt_id) = attempts;
        let mut transaction = self.lock_device(device_id, expected_expiry).await?;
        // The authenticated report confirms discovery, not a pair-verify handshake.
        let verified = sqlx::query(
            "UPDATE device_pairings SET verified_at = clock_timestamp()
             WHERE device_id = $1 AND attempt_started_at = $2
               AND remote_pairing_port IS NOT NULL
               AND remote_pairing_port_expires_at > clock_timestamp()",
        )
        .bind(device_id)
        .bind(attempt_started_at)
        .execute(&mut *transaction)
        .await
        .map_err(PairingWriteError::VerifyPort)?;
        if verified.rows_affected() != 1 {
            return Ok(false);
        }

        let peer = &record.peer;
        if let Some(public_attempt_id) = public_attempt_id {
            let completed = sqlx::query(
                "UPDATE public_pairing_attempts SET status = 'success',
                     paired_udid = $4, paired_name = $5, paired_model = $6,
                     pairing_started_at = $7
                 WHERE id = $1 AND device_id = $2 AND device_expires_at = $3
                   AND status = 'pairing' AND expires_at > clock_timestamp()",
            )
            .bind(public_attempt_id)
            .bind(device_id)
            .bind(expected_expiry)
            // Provisional: remote-pairing identity is not yet verified as hardware UDID.
            .bind(&peer.remotepairing_udid)
            .bind(&peer.name)
            .bind(&peer.model)
            .bind(attempt_started_at)
            .execute(&mut *transaction)
            .await
            .map_err(PairingWriteError::CompletePublicAttempt)?;
            if completed.rows_affected() != 1 {
                return Err(PairingWriteError::PublicAttemptUnavailable);
            }
        }
        // Publish success atomically with verification.
        let completed = sqlx::query(
            "UPDATE pairing_attempts SET
                 ended_at = clock_timestamp(), code_id = NULL, nonce = NULL, ciphertext = NULL,
                 success_notification_id = $2,
                 success_notification_expires_at = LEAST(
                     device_expires_at, clock_timestamp() + INTERVAL '60 seconds'),
                 success_notification_queued_at = NULL, success_notification_sent_at = NULL
             WHERE id = $1 AND device_id = $3 AND device_expires_at = $4
               AND ended_at IS NULL AND expires_at > clock_timestamp()",
        )
        .bind(attempt_id)
        .bind(Uuid::new_v4())
        .bind(device_id)
        .bind(expected_expiry)
        .execute(&mut *transaction)
        .await
        .map_err(PairingWriteError::CompleteAttempt)?;
        if completed.rows_affected() != 1 {
            return Err(PairingWriteError::AttemptUnavailable);
        }
        transaction
            .commit()
            .await
            .map_err(PairingWriteError::Commit)?;
        Ok(true)
    }

    async fn lock_device(
        &self,
        device_id: Uuid,
        expected_expiry: DateTime<Utc>,
    ) -> Result<Transaction<'_, Postgres>, PairingWriteError> {
        let mut transaction = self.pool.begin().await.map_err(PairingWriteError::Begin)?;
        // Coordinate with renewal/cleanup. A completed pairing must never be
        // attached to an expired, deleted, or subsequently renewed registration.
        let current = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM devices
             WHERE id = $1 AND expires_at = $2 AND expires_at > clock_timestamp()
             FOR UPDATE",
        )
        .bind(device_id)
        .bind(expected_expiry)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(PairingWriteError::LockDevice)?;
        if current.is_none() {
            return Err(PairingWriteError::DeviceUnavailable);
        }
        Ok(transaction)
    }
}
