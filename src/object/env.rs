// bole-hto
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnvValue {
    Plain(String),
    Secret(ObjectId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvOverlay {
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
