// bole-hto
use crate::error::{Error, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::random;
use serde::{Deserialize, Serialize};
// bole-9mz
use crate::acl::lattice::Label;
use crate::crypto::key_provider::{KeyProvider, ProviderChain, WrappedKey};

// bole-p8u
/// A ChaCha20-Poly1305 encrypted object stored alongside its random nonce.
///
/// `Secret` is the at-rest representation of an encrypted value.  The
/// encryption key is never stored — callers must supply the same 32-byte key
/// that was used during [`Secret::encrypt`] to recover the plaintext.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    // bole-p8u
    /// The 12-byte random nonce generated at encryption time; required for decryption.
    pub nonce: [u8; 12],
    // bole-p8u
    /// The AEAD ciphertext (plaintext length + 16-byte authentication tag).
    pub ciphertext: Vec<u8>,
}

impl Secret {
    // bole-p8u
    /// Encrypts `plaintext` with the given 32-byte `key` and returns a `Secret`.
    ///
    /// A fresh random nonce is generated for every call, so encrypting the same
    /// plaintext twice produces two distinct `Secret` values with different
    /// `ObjectId`s in the store.
    pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Self> {
        let nonce_bytes: [u8; 12] = random();
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Codec("invalid key length".into()))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| Error::Codec("encryption failed".into()))?;
        Ok(Self { nonce: nonce_bytes, ciphertext })
    }

    // bole-p8u
    /// Decrypts this secret with the given 32-byte `key` and returns the plaintext.
    ///
    /// Returns [`crate::error::Error::DecryptionFailed`] if the key is wrong
    /// or the ciphertext has been tampered with.
    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Codec("invalid key length".into()))?;
        let nonce = Nonce::from_slice(&self.nonce);
        cipher
            .decrypt(nonce, self.ciphertext.as_slice())
            .map_err(|_| Error::DecryptionFailed)
    }
}

// bole-9mz
/// Additional authenticated data bound into BOTH the value AEAD and the data-key
/// wrap. Serialized deterministically (postcard) and passed as AEAD `aad`, so a
/// wrapped DK cannot be lifted between secrets and a silent label downgrade is
/// detected on decrypt.
///
/// A stable random `secret_uid: [u8; 16]` is reserved for future per-identity
/// binding (O3); it is intentionally not a field yet, to keep the current AAD
/// bytes `{version, label}` minimal and stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretAad {
    /// Format/algorithm binding (= 2 for envelope secrets).
    pub version: u8,
    /// The secret's WS1 confidentiality label, if any.
    pub label: Option<Label>,
}

impl SecretAad {
    /// The AAD for a v2 envelope secret at `label`.
    pub fn v2(label: Option<Label>) -> Self {
        Self { version: 2, label }
    }

    /// Deterministic AAD bytes for AEAD.
    fn to_bytes(&self) -> Result<Vec<u8>> {
        postcard::to_allocvec(self).map_err(|e| Error::Codec(e.to_string()))
    }
}

// bole-9mz
/// Envelope-encrypted secret: a random per-secret data key (DK) encrypts the
/// value; the DK is wrapped under a master key via a [`KeyProvider`]. Rotating
/// the master key re-wraps `wrapped_dk` only — the value AEAD is never touched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretV2 {
    /// The data key wrapped under a master key (provider-opaque).
    pub wrapped_dk: WrappedKey,
    /// AEAD nonce for the value encryption.
    pub nonce: [u8; 12],
    /// AEAD ciphertext of the value (plaintext length + 16-byte tag).
    pub ciphertext: Vec<u8>,
    /// AAD bound into both the value AEAD and the DK wrap.
    pub aad: SecretAad,
}

impl SecretV2 {
    /// Envelope-encrypt `plaintext`: generate a random DK, AEAD the value under
    /// `(DK, nonce, aad)`, then wrap the DK via `provider` with the SAME aad.
    pub async fn encrypt_envelope(
        plaintext: &[u8],
        provider: &dyn KeyProvider,
        aad: SecretAad,
    ) -> Result<Self> {
        let aad_bytes = aad.to_bytes()?;
        let dk: [u8; 32] = random();
        let cipher = ChaCha20Poly1305::new_from_slice(&dk)
            .map_err(|_| Error::Codec("invalid data key length".into()))?;
        let nonce_bytes: [u8; 12] = random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: &aad_bytes })
            .map_err(|_| Error::Codec("value encryption failed".into()))?;
        let wrapped_dk = provider.wrap_dk(&dk, &aad_bytes).await?;
        Ok(Self { wrapped_dk, nonce: nonce_bytes, ciphertext, aad })
    }

    /// Decrypt: unwrap the DK via the chain (using this secret's aad), then
    /// AEAD-open the value. Wrong master key, lifted `wrapped_dk`, or a tampered
    /// `aad`/label → [`Error::DecryptionFailed`].
    pub async fn decrypt(&self, chain: &ProviderChain) -> Result<Vec<u8>> {
        let aad_bytes = self.aad.to_bytes()?;
        let dk = chain.unwrap(&self.wrapped_dk, &aad_bytes).await?;
        let cipher = ChaCha20Poly1305::new_from_slice(&dk)
            .map_err(|_| Error::Codec("invalid data key length".into()))?;
        let nonce = Nonce::from_slice(&self.nonce);
        cipher
            .decrypt(nonce, Payload { msg: self.ciphertext.as_slice(), aad: &aad_bytes })
            .map_err(|_| Error::DecryptionFailed)
    }
}

// bole-hto
#[cfg(test)]
mod tests {
    use super::Secret;
    use crate::error::Error;

    fn key() -> [u8; 32] { [42u8; 32] }
    fn wrong_key() -> [u8; 32] { [99u8; 32] }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = b"super secret value";
        let s = Secret::encrypt(plaintext, &key()).unwrap();
        let got = s.decrypt(&key()).unwrap();
        assert_eq!(got, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let s = Secret::encrypt(b"value", &key()).unwrap();
        let err = s.decrypt(&wrong_key()).unwrap_err();
        assert!(matches!(err, Error::DecryptionFailed));
    }

    #[test]
    fn two_encryptions_have_different_nonces() {
        let s1 = Secret::encrypt(b"val", &key()).unwrap();
        let s2 = Secret::encrypt(b"val", &key()).unwrap();
        // Same plaintext, different nonces → different ciphertext
        assert_ne!(s1.nonce, s2.nonce);
    }

    // bole-9mz
    use super::{SecretAad, SecretV2};
    use crate::acl::lattice::Label;
    use crate::crypto::key_provider::{LocalKeyProvider, ProviderChain};

    #[tokio::test]
    async fn envelope_roundtrip() {
        let provider = LocalKeyProvider::new([7u8; 32], "env");
        let s = SecretV2::encrypt_envelope(b"top secret", &provider, SecretAad::v2(None))
            .await
            .unwrap();
        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([7u8; 32], "env")));
        assert_eq!(s.decrypt(&chain).await.unwrap(), b"top secret");
    }

    #[tokio::test]
    async fn envelope_wrong_master_key_fails() {
        let provider = LocalKeyProvider::new([1u8; 32], "env");
        let s = SecretV2::encrypt_envelope(b"v", &provider, SecretAad::v2(None)).await.unwrap();
        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "env")));
        assert!(matches!(s.decrypt(&chain).await.unwrap_err(), Error::DecryptionFailed));
    }

    #[tokio::test]
    async fn relabel_tamper_fails() {
        let provider = LocalKeyProvider::new([5u8; 32], "env");
        let label = Some(Label::protected());
        let mut s = SecretV2::encrypt_envelope(b"v", &provider, SecretAad::v2(label)).await.unwrap();
        // Silently downgrade the stored label; AAD no longer matches → fails.
        s.aad.label = Some(Label::public());
        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([5u8; 32], "env")));
        assert!(matches!(s.decrypt(&chain).await.unwrap_err(), Error::DecryptionFailed));
    }

    #[tokio::test]
    async fn lifted_wrapped_dk_fails() {
        let provider = LocalKeyProvider::new([9u8; 32], "env");
        let x = SecretV2::encrypt_envelope(b"x-value", &provider, SecretAad::v2(None)).await.unwrap();
        let mut y = SecretV2::encrypt_envelope(b"y-value", &provider, SecretAad::v2(None)).await.unwrap();
        // Lift X's wrapped DK into Y: unwrap yields X's DK, which cannot open Y.
        y.wrapped_dk = x.wrapped_dk.clone();
        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([9u8; 32], "env")));
        assert!(matches!(y.decrypt(&chain).await.unwrap_err(), Error::DecryptionFailed));
    }
}
