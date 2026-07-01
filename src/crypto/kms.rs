// bole-vw9
//! KMS integration slot (feature `kms`): a [`KmsClient`] trait an operator's KMS
//! / HSM implements, and a [`KmsKeyProvider`] that turns any `KmsClient` into a
//! [`KeyProvider`] by delegating the data-key wrap/unwrap to the KMS's
//! encrypt/decrypt of a small blob — the exact narrow interface a KMS exposes.
//!
//! bole ships the trait shape and a software reference adapter
//! ([`LocalKmsClient`]); real backends (AWS KMS, Vault Transit, PKCS#11) are
//! third-party `KmsClient` impls. The data key never leaves bole in plaintext
//! beyond the single `encrypt` call, and on read the KMS `decrypt` returns it.

use async_trait::async_trait;

use crate::crypto::key_provider::{KeyProvider, WrappedKey};
use crate::error::{Error, Result};

// bole-vw9
/// The narrow KMS boundary: encrypt/decrypt of a small blob (the data key),
/// bound to `aad`. `key_ref` identifies the KMS key (id/ARN + version).
#[async_trait]
pub trait KmsClient: Send + Sync {
    /// Stable identity of the active KMS key.
    fn key_ref(&self) -> &str;
    /// Encrypts `plaintext` (≤ a few KiB) under the active KMS key, binding `aad`.
    async fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>>;
    /// Decrypts a ciphertext produced by [`encrypt`](Self::encrypt), binding `aad`.
    async fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>>;
}

// bole-vw9
/// A [`KeyProvider`] backed by a [`KmsClient`]: data keys are wrapped by the KMS.
pub struct KmsKeyProvider {
    client: Box<dyn KmsClient>,
}

impl KmsKeyProvider {
    pub fn new(client: Box<dyn KmsClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl KeyProvider for KmsKeyProvider {
    fn active_key_ref(&self) -> &str {
        self.client.key_ref()
    }

    async fn wrap_dk(&self, dk: &[u8; 32], aad: &[u8]) -> Result<WrappedKey> {
        let bytes = self.client.encrypt(dk, aad).await?;
        Ok(WrappedKey { key_ref: self.client.key_ref().to_string(), bytes })
    }

    async fn unwrap_dk(&self, wrapped: &WrappedKey, aad: &[u8]) -> Result<[u8; 32]> {
        let dk = self.client.decrypt(&wrapped.bytes, aad).await?;
        dk.try_into().map_err(|_| Error::DecryptionFailed)
    }
}

// bole-vw9
/// A software reference `KmsClient` (dev/CI/tests): a held master key encrypts
/// blobs with ChaCha20-Poly1305. NOT a real KMS — a real deployment injects a
/// cloud/HSM-backed `KmsClient`. `key_ref` is a `local-kms:` fingerprint.
pub struct LocalKmsClient {
    key: [u8; 32],
    key_ref: String,
}

impl LocalKmsClient {
    pub fn new(key: [u8; 32]) -> Self {
        let fp = blake3::hash(&key);
        let mut hex = String::with_capacity(16);
        for b in &fp.as_bytes()[..8] {
            hex.push_str(&format!("{b:02x}"));
        }
        Self { key, key_ref: format!("local-kms:{hex}") }
    }
}

#[async_trait]
impl KmsClient for LocalKmsClient {
    fn key_ref(&self) -> &str {
        &self.key_ref
    }

    async fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        use chacha20poly1305::{
            aead::{Aead, KeyInit, Payload},
            ChaCha20Poly1305, Nonce,
        };
        use rand::random;
        let cipher = ChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|_| Error::Codec("invalid kms key".into()))?;
        let nonce_bytes: [u8; 12] = random();
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), Payload { msg: plaintext, aad })
            .map_err(|_| Error::Codec("kms encrypt failed".into()))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    async fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        use chacha20poly1305::{
            aead::{Aead, KeyInit, Payload},
            ChaCha20Poly1305, Nonce,
        };
        if ciphertext.len() < 12 {
            return Err(Error::DecryptionFailed);
        }
        let cipher = ChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|_| Error::Codec("invalid kms key".into()))?;
        let (nonce, ct) = ciphertext.split_at(12);
        cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad })
            .map_err(|_| Error::DecryptionFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::key_provider::ProviderChain;
    use crate::object::SecretAad;
    use crate::object::SecretV2;

    #[tokio::test]
    async fn kms_provider_wraps_and_unwraps_a_secret() {
        let kms = KmsKeyProvider::new(Box::new(LocalKmsClient::new([5u8; 32])));
        assert!(kms.active_key_ref().starts_with("local-kms:"));

        // A full envelope secret round-trips through the KMS-backed provider.
        let secret = SecretV2::encrypt_envelope(b"top secret", &kms, SecretAad::v2(None)).await.unwrap();
        let chain = ProviderChain::with_provider(Box::new(KmsKeyProvider::new(Box::new(LocalKmsClient::new([5u8; 32])))));
        assert_eq!(secret.decrypt(&chain).await.unwrap(), b"top secret");
    }

    #[tokio::test]
    async fn wrong_kms_key_fails() {
        let a = KmsKeyProvider::new(Box::new(LocalKmsClient::new([1u8; 32])));
        let dk = [7u8; 32];
        let wrapped = a.wrap_dk(&dk, b"aad").await.unwrap();
        let b = KmsKeyProvider::new(Box::new(LocalKmsClient::new([2u8; 32])));
        assert!(b.unwrap_dk(&wrapped, b"aad").await.is_err());
    }
}
