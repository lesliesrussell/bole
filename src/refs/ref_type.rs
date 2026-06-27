// bole-prn
use crate::refs::{Tag, Timeline};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ref {
    Tag(Tag),
    Timeline(Timeline),
}

#[cfg(test)]
mod tests {
    use super::Ref;
    use crate::refs::{Tag, Timeline, TimelinePolicy};
    use crate::object::ObjectId;

    #[test]
    fn tag_round_trip() {
        let id = ObjectId::new([1u8; 32]);
        let r = Ref::Tag(Tag { target: id, created_at: 1000, message: Some("v1".into()) });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    #[test]
    fn timeline_round_trip() {
        let id = ObjectId::new([2u8; 32]);
        let r = Ref::Timeline(Timeline {
            head: id,
            policy: TimelinePolicy::Append,
            created_at: 2000,
            // bole-qv5
            kind: "persistent".into(),
            expires_at: None,
        });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }
}
