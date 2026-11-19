//! Version 1: AES-256-GCM over a JSON envelope, bound to the device UUID.
//! Neither plaintext nor the encryption key enters SQL/Temporal.

use std::{io, net::Ipv4Addr};

use chrono::{DateTime, Utc};
use idevice::remote_pairing::RpPairingFile;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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

impl InstallTargetError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Lookup(_) => "target_lookup",
            Self::Unavailable => "target_unavailable",
            Self::Decrypt(_) => "target_decrypt",
            Self::Decode(_) => "target_decode",
            Self::MissingPeerKey => "target_missing_peer_key",
            Self::InvalidRecord(_) => "target_invalid_record",
            Self::Address(_) => "target_invalid_address",
            Self::Port(_) => "target_invalid_port",
        }
    }
}

impl PairingStore {
    pub(crate) async fn install_target(
        &self,
        job_id: Uuid,
    ) -> Result<InstallTarget, InstallTargetError> {
        let row = sqlx::query_as::<_, InstallTargetRow>(
            "SELECT d.id AS device_id, host(d.address) AS address, d.remote_pairing_port,
                 p.nonce, p.ciphertext, LEAST(d.expires_at, j.expires_at) AS expires_at
             FROM install_jobs j JOIN devices d ON d.id = j.device_id AND d.expires_at = j.device_expires_at
             JOIN device_pairings p ON p.device_id = d.id AND p.attempt_started_at = j.pairing_started_at
             WHERE j.id = $1 AND j.queued_at IS NOT NULL AND j.finished_at IS NULL
               AND j.expires_at > clock_timestamp() AND d.expires_at > clock_timestamp()
               AND d.remote_pairing_port IS NOT NULL AND d.remote_pairing_port_expires_at > clock_timestamp()
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
    pub(crate) fn from_env(pool: PgPool) -> io::Result<Self> {
        Ok(Self {
            pool,
            key: SecretKey::from_env()?,
        })
    }

    pub(crate) async fn save(
        &self,
        device_id: Uuid,
        attempts: (Uuid, Option<Uuid>),
        expected_expiry: DateTime<Utc>,
        attempt_started_at: DateTime<Utc>,
        record: &PairingRecord,
    ) -> Result<(), OperationError> {
        let (attempt_id, public_attempt_id) = attempts;
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
        let plaintext = serde_json::to_vec(&stored)
            .map_err(OperationError::at("serialize_pairing_credentials"))?;
        // Domain separation + format version + device ID prevents moving a
        // ciphertext to a different device or interpreting it as another format.
        let (nonce, ciphertext) = self
            .key
            .seal(&associated_data(device_id), plaintext)
            .map_err(OperationError::at("encrypt_pairing_credentials"))?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(OperationError::at("begin_save_pairing"))?;
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
        .map_err(OperationError::at("lock_pairing_device"))?;
        if current.is_none() {
            return Err(OperationError::without_source("pairing_device_unavailable"));
        }
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
            .map_err(OperationError::at("complete_public_pairing_attempt"))?;
            if completed.rows_affected() != 1 {
                return Err(OperationError::without_source(
                    "public_pairing_attempt_unavailable",
                ));
            }
        }
        let saved = sqlx::query(
            "INSERT INTO device_pairings
                 (device_id, format_version, nonce, ciphertext, attempt_started_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (device_id) DO UPDATE SET
                 format_version = EXCLUDED.format_version,
                 nonce = EXCLUDED.nonce,
                 ciphertext = EXCLUDED.ciphertext,
                 attempt_started_at = EXCLUDED.attempt_started_at,
                 updated_at = clock_timestamp(),
                 verified_at = NULL
             WHERE device_pairings.attempt_started_at <= EXCLUDED.attempt_started_at",
        )
        .bind(device_id)
        .bind(FORMAT_VERSION)
        .bind(nonce.as_slice())
        .bind(ciphertext.as_slice())
        .bind(attempt_started_at)
        .execute(&mut *transaction)
        .await
        .map_err(OperationError::at("upsert_pairing_credentials"))?;
        if saved.rows_affected() != 1 {
            return Err(OperationError::without_source("pairing_superseded"));
        }
        // Commit the success notification together with the saved credentials.
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
        .map_err(OperationError::at("complete_pairing_attempt"))?;
        if completed.rows_affected() != 1 {
            return Err(OperationError::without_source(
                "pairing_attempt_unavailable",
            ));
        }
        transaction
            .commit()
            .await
            .map_err(OperationError::at("commit_save_pairing"))
    }
}
