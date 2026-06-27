// bole-dlw
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(*hash.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectId;

    #[test]
    fn same_content_same_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_different_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_64_hex_chars() {
        let id = ObjectId::from_bytes(b"test");
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn roundtrip_via_bytes() {
        let id = ObjectId::from_bytes(b"roundtrip");
        let id2 = ObjectId::new(*id.as_bytes());
        assert_eq!(id, id2);
    }
}
