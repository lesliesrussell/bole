// bole-su8
//! A trusted-relay pin: the relay's pinned Ed25519 public key and its transport
//! endpoint. Local, unsigned config (NOT a self-signed `CollabObject`): stored as
//! a plain `Object::Blob` under `refs/collab/relays/`. The key is the identity;
//! the endpoint is merely where to reach it.
use crate::collab::Key;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPin {
    pub key: Key,
    pub endpoint: String,
}

impl RelayPin {
    /// Postcard bytes for blob storage. Infallible for owned data.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("postcard serialization is infallible for owned data")
    }

    /// Parses a pin from blob bytes; `None` if the bytes are not a valid pin.
    pub fn from_bytes(bytes: &[u8]) -> Option<RelayPin> {
        postcard::from_bytes(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::RelayPin;

    #[test]
    fn relay_pin_round_trips() {
        let pin = RelayPin { key: [7u8; 32], endpoint: "relay.example:9418".into() };
        let bytes = pin.to_bytes();
        assert_eq!(RelayPin::from_bytes(&bytes), Some(pin));
        assert_eq!(RelayPin::from_bytes(b"not-valid-postcard-\xff\xff"), None);
    }
}
