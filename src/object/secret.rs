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

    /// Master-key rotation: unwrap the DK with `old`, re-wrap it with `new`, and
    /// return a new `SecretV2` with the SAME `nonce`/`ciphertext`/`aad` — the
    /// value AEAD is never recomputed (O(1) wrap, not O(bytes) re-encryption).
    pub async fn rewrap(&self, old: &ProviderChain, new: &dyn KeyProvider) -> Result<SecretV2> {
        let aad_bytes = self.aad.to_bytes()?;
        let dk = old.unwrap(&self.wrapped_dk, &aad_bytes).await?;
        let wrapped_dk = new.wrap_dk(&dk, &aad_bytes).await?;
        Ok(SecretV2 {
            wrapped_dk,
            nonce: self.nonce,
            ciphertext: self.ciphertext.clone(),
            aad: self.aad.clone(),
        })
    }
}

// bole-21g
/// A **multi-recipient** envelope secret: one random data key (DK) encrypts the
/// value exactly once, and that DK is wrapped independently for each *recipient*
/// — typically one per actor, under that actor's own master key.
///
/// This moves secrets from *access-gated* (any cleared reader shares one master
/// key, and the repo decides who is cleared) to *cryptographically per-actor*
/// (each actor unwraps with a key only they hold; no shared master key exists).
///
/// - [`encrypt_for`](Self::encrypt_for) wraps the DK for an initial recipient set.
/// - [`grant`](Self::grant) adds a recipient (unwrap the DK via a chain that can
///   already read, then re-wrap for the newcomer) — the value ciphertext is
///   never recomputed.
/// - [`revoke`](Self::revoke) drops a recipient's wrap. This is *forward*
///   revocation: a reader who already extracted the DK is not un-taught it, so
///   revoking a compromised recipient should be paired with a value rotation
///   (fresh DK via a new `encrypt_for`).
///
/// The [`SecretAad`] (version + label) is bound into both the value AEAD and
/// every DK wrap, exactly as in [`SecretV2`], so a wrap cannot be lifted between
/// secrets and a silent relabel is detected on decrypt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiRecipientSecret {
    /// The DK wrapped once per recipient; any one wrap recovers the same DK.
    pub recipients: Vec<WrappedKey>,
    /// AEAD nonce for the value encryption.
    pub nonce: [u8; 12],
    /// AEAD ciphertext of the value (plaintext length + 16-byte tag).
    pub ciphertext: Vec<u8>,
    /// AAD bound into the value AEAD and every recipient wrap.
    pub aad: SecretAad,
}

impl MultiRecipientSecret {
    /// Envelope-encrypt `plaintext` for one or more recipients: generate a random
    /// DK, AEAD the value once under `(DK, nonce, aad)`, then wrap the DK for each
    /// provider with the SAME aad. Rejects an empty recipient set — a secret no
    /// one can read is a bug, not a valid state.
    pub async fn encrypt_for(
        plaintext: &[u8],
        recipients: &[&dyn KeyProvider],
        aad: SecretAad,
    ) -> Result<Self> {
        if recipients.is_empty() {
            return Err(Error::Codec("a multi-recipient secret needs at least one recipient".into()));
        }
        let aad_bytes = aad.to_bytes()?;
        let dk: [u8; 32] = random();
        let cipher = ChaCha20Poly1305::new_from_slice(&dk)
            .map_err(|_| Error::Codec("invalid data key length".into()))?;
        let nonce_bytes: [u8; 12] = random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: &aad_bytes })
            .map_err(|_| Error::Codec("value encryption failed".into()))?;
        let mut wraps = Vec::with_capacity(recipients.len());
        for provider in recipients {
            wraps.push(provider.wrap_dk(&dk, &aad_bytes).await?);
        }
        Ok(Self { recipients: wraps, nonce: nonce_bytes, ciphertext, aad })
    }

    /// Decrypt: find the first recipient wrap this `chain` can unwrap, recover the
    /// DK, and AEAD-open the value. Fail-closed if no recipient matches, the DK is
    /// wrong, or the aad/label was tampered with.
    pub async fn decrypt(&self, chain: &ProviderChain) -> Result<Vec<u8>> {
        let aad_bytes = self.aad.to_bytes()?;
        for wrap in &self.recipients {
            if let Ok(dk) = chain.unwrap(wrap, &aad_bytes).await {
                let cipher = ChaCha20Poly1305::new_from_slice(&dk)
                    .map_err(|_| Error::Codec("invalid data key length".into()))?;
                let nonce = Nonce::from_slice(&self.nonce);
                return cipher
                    .decrypt(nonce, Payload { msg: self.ciphertext.as_slice(), aad: &aad_bytes })
                    .map_err(|_| Error::DecryptionFailed);
            }
        }
        Err(Error::DecryptionFailed)
    }

    /// Grant `recipient` access: recover the DK via `chain` (which must already be
    /// able to read this secret), then wrap it for the newcomer. The value AEAD is
    /// untouched. A no-op (Ok) if `recipient`'s key_ref is already present.
    pub async fn grant(&mut self, chain: &ProviderChain, recipient: &dyn KeyProvider) -> Result<()> {
        let aad_bytes = self.aad.to_bytes()?;
        // Recover the DK from any wrap the chain can open.
        let mut dk_opt = None;
        for wrap in &self.recipients {
            if let Ok(dk) = chain.unwrap(wrap, &aad_bytes).await {
                dk_opt = Some(dk);
                break;
            }
        }
        let dk = dk_opt.ok_or(Error::DecryptionFailed)?;
        let new_wrap = recipient.wrap_dk(&dk, &aad_bytes).await?;
        if self.recipients.iter().any(|w| w.key_ref == new_wrap.key_ref) {
            return Ok(());
        }
        self.recipients.push(new_wrap);
        Ok(())
    }

    /// Revoke every recipient wrap whose `key_ref` equals `key_ref`. Returns
    /// whether any wrap was removed. Refuses to remove the *last* recipient, which
    /// would leave the secret unreadable by anyone.
    pub fn revoke(&mut self, key_ref: &str) -> Result<bool> {
        let matching = self.recipients.iter().filter(|w| w.key_ref == key_ref).count();
        if matching == 0 {
            return Ok(false);
        }
        if matching >= self.recipients.len() {
            return Err(Error::Codec("cannot revoke the last recipient of a secret".into()));
        }
        self.recipients.retain(|w| w.key_ref != key_ref);
        Ok(true)
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

    // bole-21g
    use super::MultiRecipientSecret;
    use crate::crypto::key_provider::KeyProvider;

    /// Two actors, each holding their own master key, can both read the same
    /// secret; an actor with neither key cannot.
    #[tokio::test]
    async fn multi_recipient_each_actor_reads_with_own_key() {
        let alice = LocalKeyProvider::new([1u8; 32], "actor:alice");
        let bob = LocalKeyProvider::new([2u8; 32], "actor:bob");
        let recipients: [&dyn KeyProvider; 2] = [&alice, &bob];

        let s = MultiRecipientSecret::encrypt_for(b"shared secret", &recipients, SecretAad::v2(None))
            .await
            .unwrap();
        assert_eq!(s.recipients.len(), 2);

        // Alice decrypts with only her key.
        let a_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "actor:alice")));
        assert_eq!(s.decrypt(&a_chain).await.unwrap(), b"shared secret");

        // Bob decrypts with only his key.
        let b_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "actor:bob")));
        assert_eq!(s.decrypt(&b_chain).await.unwrap(), b"shared secret");

        // Carol, cleared for neither, cannot.
        let c_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([9u8; 32], "actor:carol")));
        assert!(matches!(s.decrypt(&c_chain).await.unwrap_err(), Error::DecryptionFailed));
    }

    /// Granting a new actor adds a wrap without re-encrypting the value; the new
    /// actor can then read, and the value bytes are byte-identical.
    #[tokio::test]
    async fn grant_adds_recipient_without_touching_value() {
        let alice = LocalKeyProvider::new([1u8; 32], "actor:alice");
        let one: [&dyn KeyProvider; 1] = [&alice];
        let mut s = MultiRecipientSecret::encrypt_for(b"v", &one, SecretAad::v2(None)).await.unwrap();
        let ct_before = s.ciphertext.clone();
        let nonce_before = s.nonce;

        // Bob is not yet a recipient.
        let b_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "actor:bob")));
        assert!(s.decrypt(&b_chain).await.is_err());

        // Alice grants Bob: unwrap via Alice's chain, wrap for Bob.
        let a_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "actor:alice")));
        let bob = LocalKeyProvider::new([2u8; 32], "actor:bob");
        s.grant(&a_chain, &bob).await.unwrap();

        // Value AEAD untouched; Bob can now read.
        assert_eq!(s.ciphertext, ct_before);
        assert_eq!(s.nonce, nonce_before);
        assert_eq!(s.recipients.len(), 2);
        assert_eq!(s.decrypt(&b_chain).await.unwrap(), b"v");
    }

    /// Revoking an actor removes their wrap; they can no longer decrypt, but
    /// remaining recipients still can. Revoking the last recipient is refused.
    #[tokio::test]
    async fn revoke_removes_recipient_and_guards_last() {
        let alice = LocalKeyProvider::new([1u8; 32], "actor:alice");
        let bob = LocalKeyProvider::new([2u8; 32], "actor:bob");
        let recipients: [&dyn KeyProvider; 2] = [&alice, &bob];
        let mut s = MultiRecipientSecret::encrypt_for(b"v", &recipients, SecretAad::v2(None)).await.unwrap();

        let bob_ref = LocalKeyProvider::new([2u8; 32], "actor:bob").active_key_ref().to_string();
        assert!(s.revoke(&bob_ref).unwrap());

        // Bob can no longer read; Alice still can.
        let b_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "actor:bob")));
        assert!(s.decrypt(&b_chain).await.is_err());
        let a_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "actor:alice")));
        assert_eq!(s.decrypt(&a_chain).await.unwrap(), b"v");

        // Revoking the last recipient would orphan the secret — refused.
        let alice_ref = LocalKeyProvider::new([1u8; 32], "actor:alice").active_key_ref().to_string();
        assert!(s.revoke(&alice_ref).is_err());
    }

    /// A recipient wrap lifted from another secret cannot open this value.
    #[tokio::test]
    async fn multi_recipient_lifted_wrap_fails() {
        let alice = LocalKeyProvider::new([1u8; 32], "actor:alice");
        let one: [&dyn KeyProvider; 1] = [&alice];
        let x = MultiRecipientSecret::encrypt_for(b"x", &one, SecretAad::v2(None)).await.unwrap();
        let mut y = MultiRecipientSecret::encrypt_for(b"y", &one, SecretAad::v2(None)).await.unwrap();
        // Lift X's recipient wrap into Y: unwraps to X's DK, which cannot open Y.
        y.recipients = x.recipients.clone();
        let a_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "actor:alice")));
        assert!(matches!(y.decrypt(&a_chain).await.unwrap_err(), Error::DecryptionFailed));
    }

    /// Encrypting with no recipients is refused — an unreadable secret is a bug.
    #[tokio::test]
    async fn encrypt_for_requires_a_recipient() {
        let none: [&dyn KeyProvider; 0] = [];
        assert!(MultiRecipientSecret::encrypt_for(b"v", &none, SecretAad::v2(None)).await.is_err());
    }

    #[tokio::test]
    async fn rewrap_preserves_value_bytes_and_plaintext() {
        let a = LocalKeyProvider::new([1u8; 32], "env");
        let s = SecretV2::encrypt_envelope(b"val", &a, SecretAad::v2(None)).await.unwrap();
        let chain_a = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "env")));
        let b = LocalKeyProvider::new([2u8; 32], "env");

        let s2 = s.rewrap(&chain_a, &b).await.unwrap();
        // Only the wrap changes; the value AEAD is untouched.
        assert_eq!(s2.nonce, s.nonce);
        assert_eq!(s2.ciphertext, s.ciphertext);
        assert_ne!(s2.wrapped_dk, s.wrapped_dk);

        // Decrypts under the new MK; the old MK can no longer unwrap it.
        let chain_b = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "env")));
        assert_eq!(s2.decrypt(&chain_b).await.unwrap(), b"val");
        assert!(s2.decrypt(&chain_a).await.is_err());
    }
}
