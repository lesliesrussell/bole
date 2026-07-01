// bole-9mz
//! Master-key authority and the wrap/unwrap boundary for envelope encryption.
//!
//! A [`KeyProvider`] is the authority for the *master key* only. It never sees
//! secret values or data keys outside the wrap/unwrap boundary — the same narrow
//! interface a KMS/HSM exposes (`Encrypt`/`Decrypt` of a small blob). Envelope
//! encryption generates a random per-secret **data key** (DK) that encrypts the
//! value; the DK is **wrapped** under the master key (MK) via the provider.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::random;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

// bole-9mz
/// An opaque wrapped data key: the bytes a provider returns from
/// [`KeyProvider::wrap_dk`] and consumes in [`KeyProvider::unwrap_dk`]. For local
/// providers this is `nonce || AEAD(dk)` under the MK; for a KMS it is the
/// provider's ciphertext blob. bole stores it verbatim in a `SecretV2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    /// Identifies which master key produced this wrap, so a reader can route to
    /// the right provider and `rekey` can detect staleness.
    pub key_ref: String,
    /// Provider-opaque wrapped-key bytes.
    pub bytes: Vec<u8>,
}

// bole-9mz
/// Source of the master key and the wrap/unwrap boundary. Implementations wrap a
/// local MK (env/file) or delegate to an external KMS/HSM (deferred, bole-vw9).
#[async_trait::async_trait]
pub trait KeyProvider: Send + Sync {
    /// Stable identity of the active master key (used for `WrappedKey.key_ref`
    /// and rekey's "is this wrap current?" check).
    fn active_key_ref(&self) -> &str;

    /// Wrap a freshly generated 32-byte data key under the active master key.
    /// `aad` binds the wrap to the secret it protects.
    async fn wrap_dk(&self, dk: &[u8; 32], aad: &[u8]) -> Result<WrappedKey>;

    /// Unwrap a previously wrapped data key. `aad` must match the wrap.
    async fn unwrap_dk(&self, wrapped: &WrappedKey, aad: &[u8]) -> Result<[u8; 32]>;
}

// bole-9mz
/// A local (in-process) [`KeyProvider`]: the master key is held in memory and
/// wraps DKs with ChaCha20-Poly1305. The CLI builds these from `$BOLE_KEY`
/// (`env:` ref) or `--key-file` (`file:` ref); reading the source is the CLI's
/// job, so the library takes the raw MK plus a `key_ref` prefix.
pub struct LocalKeyProvider {
    mk: [u8; 32],
    key_ref: String,
}

impl LocalKeyProvider {
    /// Builds a provider from a master key and a `ref_prefix` (e.g. `"env"` or
    /// `"file"`). `active_key_ref` becomes `"{prefix}:{fingerprint}"`, where the
    /// fingerprint is `blake3(mk)[..8]` hex — rotating the MK changes the ref
    /// without ever exposing the key.
    pub fn new(mk: [u8; 32], ref_prefix: &str) -> Self {
        let fp = blake3::hash(&mk);
        let key_ref = format!("{}:{}", ref_prefix, hex8(fp.as_bytes()));
        Self { mk, key_ref }
    }
}

#[async_trait::async_trait]
impl KeyProvider for LocalKeyProvider {
    fn active_key_ref(&self) -> &str {
        &self.key_ref
    }

    async fn wrap_dk(&self, dk: &[u8; 32], aad: &[u8]) -> Result<WrappedKey> {
        let cipher = ChaCha20Poly1305::new_from_slice(&self.mk)
            .map_err(|_| Error::Codec("invalid master key length".into()))?;
        let nonce_bytes: [u8; 12] = random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, Payload { msg: dk, aad })
            .map_err(|_| Error::Codec("dk wrap failed".into()))?;
        let mut bytes = Vec::with_capacity(12 + ct.len());
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ct);
        Ok(WrappedKey { key_ref: self.key_ref.clone(), bytes })
    }

    async fn unwrap_dk(&self, wrapped: &WrappedKey, aad: &[u8]) -> Result<[u8; 32]> {
        if wrapped.bytes.len() < 12 {
            return Err(Error::DecryptionFailed);
        }
        let cipher = ChaCha20Poly1305::new_from_slice(&self.mk)
            .map_err(|_| Error::Codec("invalid master key length".into()))?;
        let (nonce_bytes, ct) = wrapped.bytes.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let dk = cipher
            .decrypt(nonce, Payload { msg: ct, aad })
            .map_err(|_| Error::DecryptionFailed)?;
        let arr: [u8; 32] = dk.try_into().map_err(|_| Error::DecryptionFailed)?;
        Ok(arr)
    }
}

// bole-9mz
/// The read-side resolver: an ordered set of [`KeyProvider`]s (active first,
/// then fallbacks for reads across master-key rotations) plus any legacy raw
/// v1 keys. `unwrap` finds the first provider that can recover a `WrappedKey`;
/// `legacy_keys` feed the v1 decrypt path.
#[derive(Default)]
pub struct ProviderChain {
    providers: Vec<Box<dyn KeyProvider>>,
    legacy_keys: Vec<[u8; 32]>,
}

impl ProviderChain {
    /// An empty chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a chain with a single active provider.
    pub fn with_provider(provider: Box<dyn KeyProvider>) -> Self {
        Self { providers: vec![provider], legacy_keys: Vec::new() }
    }

    /// Appends a fallback provider (tried after earlier ones on reads).
    pub fn push_provider(&mut self, provider: Box<dyn KeyProvider>) {
        self.providers.push(provider);
    }

    /// Appends a legacy raw v1 key for the v1 decrypt path.
    pub fn push_legacy_key(&mut self, key: [u8; 32]) {
        self.legacy_keys.push(key);
    }

    /// The active (first) provider — the one new wraps are written under.
    pub fn active(&self) -> Result<&dyn KeyProvider> {
        self.providers
            .first()
            .map(|p| p.as_ref())
            .ok_or_else(|| Error::Codec("no key provider configured".into()))
    }

    /// The legacy v1 keys, in order.
    pub fn legacy_keys(&self) -> &[[u8; 32]] {
        &self.legacy_keys
    }

    /// Unwraps a `WrappedKey` by trying the provider whose `active_key_ref`
    /// matches first, then every other provider. Fails closed if none succeed.
    pub async fn unwrap(&self, wrapped: &WrappedKey, aad: &[u8]) -> Result<[u8; 32]> {
        // Prefer the provider that advertises this exact key_ref.
        for p in &self.providers {
            if p.active_key_ref() == wrapped.key_ref {
                if let Ok(dk) = p.unwrap_dk(wrapped, aad).await {
                    return Ok(dk);
                }
            }
        }
        // Fall through to any provider (prior MK versions, etc.).
        for p in &self.providers {
            if let Ok(dk) = p.unwrap_dk(wrapped, aad).await {
                return Ok(dk);
            }
        }
        Err(Error::DecryptionFailed)
    }
}

// bole-9mz
/// Hex-encodes the first 8 bytes of `bytes` (a 16-char fingerprint).
fn hex8(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    for b in &bytes[..8] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{KeyProvider, LocalKeyProvider, ProviderChain, WrappedKey};

    #[tokio::test]
    async fn wrap_unwrap_roundtrip() {
        let p = LocalKeyProvider::new([7u8; 32], "env");
        let dk = [3u8; 32];
        let aad = b"ctx";
        let w = p.wrap_dk(&dk, aad).await.unwrap();
        assert!(w.key_ref.starts_with("env:"));
        assert_eq!(p.unwrap_dk(&w, aad).await.unwrap(), dk);
    }

    #[tokio::test]
    async fn wrong_master_key_fails() {
        let a = LocalKeyProvider::new([1u8; 32], "env");
        let b = LocalKeyProvider::new([2u8; 32], "env");
        let w = a.wrap_dk(&[9u8; 32], b"x").await.unwrap();
        assert!(b.unwrap_dk(&w, b"x").await.is_err());
    }

    #[tokio::test]
    async fn aad_mismatch_fails() {
        let p = LocalKeyProvider::new([5u8; 32], "file");
        let w = p.wrap_dk(&[4u8; 32], b"aad-A").await.unwrap();
        assert!(p.unwrap_dk(&w, b"aad-B").await.is_err());
    }

    #[tokio::test]
    async fn key_ref_changes_with_master_key() {
        let a = LocalKeyProvider::new([1u8; 32], "env");
        let b = LocalKeyProvider::new([2u8; 32], "env");
        assert_ne!(a.active_key_ref(), b.active_key_ref());
    }

    #[tokio::test]
    async fn chain_unwraps_across_two_master_keys() {
        let mk_a = LocalKeyProvider::new([1u8; 32], "env");
        let mk_b = LocalKeyProvider::new([2u8; 32], "env");
        let w_a = mk_a.wrap_dk(&[8u8; 32], b"c").await.unwrap();
        let w_b = mk_b.wrap_dk(&[8u8; 32], b"c").await.unwrap();

        // Chain holds both; either wrap resolves.
        let mut chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "env")));
        chain.push_provider(Box::new(LocalKeyProvider::new([2u8; 32], "env")));
        assert_eq!(chain.unwrap(&w_a, b"c").await.unwrap(), [8u8; 32]);
        assert_eq!(chain.unwrap(&w_b, b"c").await.unwrap(), [8u8; 32]);

        // A wrap under an unknown MK fails closed.
        let mk_c = LocalKeyProvider::new([3u8; 32], "env");
        let w_c = mk_c.wrap_dk(&[8u8; 32], b"c").await.unwrap();
        assert!(chain.unwrap(&w_c, b"c").await.is_err());
        let _ = WrappedKey { key_ref: "x".into(), bytes: vec![] };
    }
}
