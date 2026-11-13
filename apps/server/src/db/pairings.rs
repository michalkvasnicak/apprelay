//! Version 1: AES-256-GCM over a JSON envelope, bound to the device UUID.
//! Only this worker receives the key; neither plaintext nor key enters SQL/Temporal.

use std::io;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::pairing::PairingRecord;

const FORMAT_VERSION: i16 = 1;
const KEY_ENV: &str = "PAIRING_ENCRYPTION_KEY";

pub(crate) struct PairingStore {
    pool: PgPool,
    key: LessSafeKey,
}

// Borrow the source data to avoid unnecessary copies of secrets. The plist and
// final serialized envelope are zeroized after encryption, including on errors.
#[derive(Serialize)]
struct StoredPairing<'a> {
    pairing_file: &'a [u8],
    host: StoredHost<'a>,
    peer: StoredPeer<'a>,
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

impl PairingStore {
    /// Fail startup on missing/malformed configuration; never print the value.
    pub(crate) fn from_env(pool: PgPool) -> io::Result<Self> {
        let encoded = Zeroizing::new(std::env::var(KEY_ENV).map_err(|_| {
            io::Error::other("PAIRING_ENCRYPTION_KEY is required and must be valid Unicode")
        })?);
        let bytes = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(|_| io::Error::other("PAIRING_ENCRYPTION_KEY must be Base64-encoded"))?,
        );
        let key = UnboundKey::new(&AES_256_GCM, &bytes).map_err(|_| {
            io::Error::other("PAIRING_ENCRYPTION_KEY must decode to exactly 32 bytes")
        })?;
        Ok(Self {
            pool,
            key: LessSafeKey::new(key),
        })
    }

    pub(crate) async fn save(
        &self,
        device_id: Uuid,
        expected_expiry: DateTime<Utc>,
        attempt_started_at: DateTime<Utc>,
        record: &PairingRecord,
    ) -> io::Result<()> {
        let pairing_file = Zeroizing::new(record.pairing_file.to_bytes());
        let host = &record.host_info;
        let peer = &record.peer;
        let stored = StoredPairing {
            pairing_file: &pairing_file,
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
        let mut ciphertext = Zeroizing::new(
            serde_json::to_vec(&stored)
                .map_err(|_| io::Error::other("could not serialize pairing credentials"))?,
        );
        let mut nonce = [0u8; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| io::Error::other("could not generate pairing encryption nonce"))?;
        // Domain separation + format version + device ID prevents moving a
        // ciphertext to a different device or interpreting it as another format.
        let mut associated_data = b"apprelay/device-pairing/".to_vec();
        associated_data.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
        associated_data.extend_from_slice(device_id.as_bytes());
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(associated_data),
                &mut *ciphertext,
            )
            .map_err(|_| io::Error::other("could not encrypt pairing credentials"))?;

        let mut transaction = self.pool.begin().await.map_err(io::Error::other)?;
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
        .map_err(io::Error::other)?;
        if current.is_none() {
            return Err(io::Error::other(
                "device registration changed or expired before pairing was saved",
            ));
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
        .map_err(io::Error::other)?;
        if saved.rows_affected() != 1 {
            return Err(io::Error::other(
                "a newer pairing attempt has already been saved",
            ));
        }
        transaction.commit().await.map_err(io::Error::other)
    }
}
