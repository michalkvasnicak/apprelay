//! Capture the device public key from authenticated pair-setup, and check it
//! against the pair-verify transcript before requesting a tunnel listener.
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::Aead};
use hkdf::Hkdf;
use idevice::{IdeviceError, remote_pairing::RpPairingSocketProvider};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use zeroize::Zeroizing;

fn invalid() -> IdeviceError {
    IdeviceError::UnexpectedResponse("invalid RemotePairing identity exchange".into())
}

fn tlv(mut bytes: &[u8]) -> Result<BTreeMap<u8, Vec<u8>>, IdeviceError> {
    let mut fields = BTreeMap::<u8, Vec<u8>>::new();
    while let Some((&kind, rest)) = bytes.split_first() {
        let (&len, rest) = rest.split_first().ok_or_else(invalid)?;
        let (value, rest) = rest
            .split_at_checked(usize::from(len))
            .ok_or_else(invalid)?;
        fields.entry(kind).or_default().extend_from_slice(value);
        bytes = rest;
    }
    Ok(fields)
}

fn field(fields: &BTreeMap<u8, Vec<u8>>, kind: u8) -> Result<&[u8], IdeviceError> {
    fields.get(&kind).map(Vec::as_slice).ok_or_else(invalid)
}

fn decrypt(
    secret: &[u8],
    salt: &[u8],
    info: &[u8],
    nonce: &[u8; 12],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, IdeviceError> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<sha2::Sha512>::new(Some(salt), secret)
        .expand(info, key.as_mut())
        .map_err(|_| invalid())?;
    let cipher = ChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| invalid())?;
    cipher
        .decrypt(&Nonce::from(*nonce), ciphertext)
        .map(Zeroizing::new)
        .map_err(|_| invalid())
}

#[derive(Clone, Default)]
pub(crate) struct Transcript(Arc<Mutex<Messages>>);

#[derive(Default)]
struct Messages {
    host_ephemeral: Vec<u8>,
    peer_reply: Vec<u8>,
}

impl Transcript {
    pub(crate) fn setup_public_key(&self, srp_key: &[u8]) -> Result<[u8; 32], IdeviceError> {
        let messages = self.0.lock().map_err(|_| invalid())?;
        let reply = tlv(&messages.peer_reply)?;
        // PairableHost has already authenticated the phone via PIN/SRP and
        // accepted this AEAD-protected M5 before returning the SRP key.
        let plaintext = decrypt(
            srp_key,
            b"Pair-Setup-Encrypt-Salt",
            b"Pair-Setup-Encrypt-Info",
            b"\0\0\0\0PS-Msg05",
            field(&reply, 5)?,
        )?;
        field(&tlv(&plaintext)?, 3)?
            .try_into()
            .map_err(|_| invalid())
    }

    pub(crate) fn verify(&self, secret: &[u8], peer_key: &[u8; 32]) -> Result<(), IdeviceError> {
        let messages = self.0.lock().map_err(|_| invalid())?;
        if messages.host_ephemeral.len() != 32
            || secret.len() != 32
            || secret.iter().all(|b| *b == 0)
        {
            return Err(invalid());
        }
        let reply = tlv(&messages.peer_reply)?;
        let peer_ephemeral: &[u8; 32] = field(&reply, 3)?.try_into().map_err(|_| invalid())?;
        let plaintext = decrypt(
            secret,
            b"Pair-Verify-Encrypt-Salt",
            b"Pair-Verify-Encrypt-Info",
            b"\0\0\0\0PV-Msg02",
            field(&reply, 5)?,
        )?;
        let identity = tlv(&plaintext)?;
        let mut signed = peer_ephemeral.to_vec();
        signed.extend_from_slice(field(&identity, 1)?);
        signed.extend_from_slice(&messages.host_ephemeral);
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, peer_key)
            .verify(&signed, field(&identity, 10)?)
            .map_err(|_| invalid())
    }
}

pub(crate) struct IdentitySocket {
    stream: TcpStream,
    next_sequence: u64,
    transcript: Transcript,
    setup: bool,
}

impl std::fmt::Debug for IdentitySocket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdentitySocket")
            .finish_non_exhaustive()
    }
}

impl IdentitySocket {
    pub(crate) fn new(stream: TcpStream, transcript: Transcript, setup: bool) -> Self {
        Self {
            stream,
            next_sequence: 0,
            transcript,
            setup,
        }
    }

    async fn send(&mut self, message: Value, seq: usize) -> Result<(), IdeviceError> {
        let origin = if self.setup { "device" } else { "host" };
        let value = json!({"message": message, "originatedBy": origin, "sequenceNumber": seq});
        let bytes = serde_json::to_vec(&value)?;
        let length = u16::try_from(bytes.len()).map_err(|_| invalid())?;
        self.stream.write_all(b"RPPairing").await?;
        self.stream.write_all(&length.to_be_bytes()).await?;
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

fn pairing_data(value: &Value) -> Result<Option<Vec<u8>>, IdeviceError> {
    value
        .pointer("/event/_0/pairingData/_0/data")
        .and_then(Value::as_str)
        .map(|data| STANDARD.decode(data).map_err(|_| invalid()))
        .transpose()
}

impl RpPairingSocketProvider for IdentitySocket {
    fn send_plain(
        &mut self,
        value: impl Serialize,
        seq: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdeviceError>> + Send + '_>> {
        let value = serde_json::to_value(value);
        Box::pin(async move {
            let value = value?;
            if !self.setup
                && let Some(data) = pairing_data(&value)?
            {
                let fields = tlv(&data)?;
                if fields.get(&6).map(Vec::as_slice) == Some(&[1]) {
                    self.transcript
                        .0
                        .lock()
                        .map_err(|_| invalid())?
                        .host_ephemeral = field(&fields, 3)?.to_vec();
                }
            }
            self.send(json!({"plain": {"_0": value}}), seq).await
        })
    }

    fn send_encrypted(
        &mut self,
        ciphertext: Vec<u8>,
        seq: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdeviceError>> + Send + '_>> {
        Box::pin(async move {
            self.send(
                json!({"streamEncrypted": {"_0": STANDARD.encode(ciphertext)}}),
                seq,
            )
            .await
        })
    }

    fn recv_plain(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<plist::Value, IdeviceError>> + Send + '_>> {
        Box::pin(async move {
            let mut magic = [0; 9];
            self.stream.read_exact(&mut magic).await?;
            if &magic != b"RPPairing" {
                return Err(invalid());
            }
            let length = self.stream.read_u16().await?;
            let mut bytes = vec![0; usize::from(length)];
            self.stream.read_exact(&mut bytes).await?;
            let value: Value = serde_json::from_slice(&bytes)?;
            if !self.setup {
                if value["originatedBy"] != "device"
                    || value["sequenceNumber"].as_u64() != Some(self.next_sequence)
                {
                    return Err(invalid());
                }
                self.next_sequence += 1;
            }
            let plain = value.pointer("/message/plain/_0");
            if let Some(plain) = plain
                && let Some(data) = pairing_data(plain)?
            {
                let fields = tlv(&data)?;
                let state = if self.setup { 5 } else { 2 };
                if fields.get(&6).map(Vec::as_slice) == Some(&[state]) && !fields.contains_key(&7) {
                    self.transcript.0.lock().map_err(|_| invalid())?.peer_reply = data;
                }
            }
            Ok(plist::to_value(plain.unwrap_or(&value))?)
        })
    }

    fn serialize_bytes(bytes: &[u8]) -> plist::Value {
        plist::Value::String(STANDARD.encode(bytes))
    }

    fn deserialize_bytes(value: plist::Value) -> Option<Vec<u8>> {
        STANDARD.decode(value.as_string()?).ok()
    }
}
