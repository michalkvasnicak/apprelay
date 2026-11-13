use std::{io, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct SecretKey(Arc<LessSafeKey>);

impl SecretKey {
    pub(crate) fn from_env() -> io::Result<Self> {
        let encoded = Zeroizing::new(std::env::var("PAIRING_ENCRYPTION_KEY").map_err(|_| {
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
        Ok(Self(Arc::new(LessSafeKey::new(key))))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn seal(&self, aad: &[u8], plaintext: Vec<u8>) -> io::Result<([u8; 12], Vec<u8>)> {
        let mut buffer = Zeroizing::new(plaintext);
        let mut nonce = [0u8; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| io::Error::other("could not generate encryption nonce"))?;
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut *buffer,
            )
            .map_err(|_| io::Error::other("could not encrypt secret"))?;
        Ok((nonce, std::mem::take(&mut *buffer)))
    }

    pub(crate) fn open(
        &self,
        aad: &[u8],
        nonce: &[u8],
        ciphertext: Vec<u8>,
    ) -> io::Result<Zeroizing<Vec<u8>>> {
        let nonce: [u8; 12] = nonce
            .try_into()
            .map_err(|_| io::Error::other("invalid secret nonce"))?;
        let mut buffer = Zeroizing::new(ciphertext);
        let length = self
            .0
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut buffer,
            )
            .map_err(|_| io::Error::other("could not decrypt secret"))?
            .len();
        buffer.truncate(length);
        Ok(buffer)
    }
}
