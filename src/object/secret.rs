// bole-hto
use crate::error::{Error, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::random;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

impl Secret {
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

    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Codec("invalid key length".into()))?;
        let nonce = Nonce::from_slice(&self.nonce);
        cipher
            .decrypt(nonce, self.ciphertext.as_slice())
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
}
