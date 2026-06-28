// bole-dlw
// bole-qj8
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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

// bole-qj8
#[derive(Debug)]
pub struct ParseObjectIdError;

impl fmt::Display for ParseObjectIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected 64 lowercase hex characters")
    }
}

impl std::error::Error for ParseObjectIdError {}

impl FromStr for ObjectId {
    type Err = ParseObjectIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 { return Err(ParseObjectIdError); }
        let mut bytes = [0u8; 32];
        let s = s.as_bytes();
        for (i, byte) in bytes.iter_mut().enumerate() {
            let hi = hex_nibble(s[i * 2]).ok_or(ParseObjectIdError)?;
            let lo = hex_nibble(s[i * 2 + 1]).ok_or(ParseObjectIdError)?;
            *byte = (hi << 4) | lo;
        }
        Ok(ObjectId(bytes))
    }
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
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

    // bole-qj8
    #[test]
    fn from_str_roundtrip() {
        let id = ObjectId::from_bytes(b"hello");
        let hex = id.to_string();
        let parsed: ObjectId = hex.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn from_str_wrong_length_errors() {
        assert!("abc".parse::<ObjectId>().is_err());
        assert!("".parse::<ObjectId>().is_err());
    }

    #[test]
    fn from_str_invalid_hex_errors() {
        let bad = "g".repeat(64);
        assert!(bad.parse::<ObjectId>().is_err());
    }
}
