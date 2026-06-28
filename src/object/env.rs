// bole-hto
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// bole-p8u
/// The value of a single environment variable, either stored in the clear or as
/// an encrypted secret reference.
///
/// Using `Secret` keeps the actual value out of the snapshot tree while still
/// allowing the `EnvOverlay` to be content-addressed and audited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnvValue {
    // bole-p8u
    /// A plaintext string value for the variable.
    Plain(String),
    // bole-p8u
    /// A reference to a [`Secret`](crate::object::Secret) object that holds the
    /// encrypted value; callers must supply the decryption key to resolve it.
    Secret(ObjectId),
}

// bole-p8u
/// An immutable, typed bundle of environment variable name-value pairs.
///
/// `EnvOverlay` is stored as a content-addressed object so that identical
/// configurations are never duplicated, and so that historical overlay states
/// can be retrieved by `ObjectId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvOverlay {
    // bole-p8u
    /// Sorted map from variable name to its value or encrypted reference.
    pub entries: BTreeMap<String, EnvValue>,
}

// bole-hto
#[cfg(test)]
mod tests {
    use super::{EnvOverlay, EnvValue};
    use crate::object::ObjectId;
    use std::collections::BTreeMap;

    #[test]
    fn overlay_serializes_roundtrip() {
        let mut entries = BTreeMap::new();
        entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
        entries.insert("DB_URL".into(), EnvValue::Secret(ObjectId::new([1u8; 32])));
        let overlay = EnvOverlay { entries };
        let encoded = postcard::to_allocvec(&overlay).unwrap();
        let decoded: EnvOverlay = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(overlay, decoded);
    }
}
