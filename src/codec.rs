// bole-dq0
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};

pub(crate) fn serialize(obj: &Object) -> Result<Vec<u8>> {
    postcard::to_allocvec(obj).map_err(|e| Error::Codec(e.to_string()))
}

pub(crate) fn deserialize(data: &[u8]) -> Result<Object> {
    postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string()))
}

pub(crate) fn object_id(data: &[u8]) -> ObjectId {
    ObjectId::from_bytes(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Blob, Object};
    use bytes::Bytes;

    #[test]
    fn blob_round_trip() {
        let obj = Object::Blob(Blob { data: Bytes::from("hello world") });
        let data = serialize(&obj).unwrap();
        let decoded = deserialize(&data).unwrap();
        assert_eq!(obj, decoded);
    }

    #[test]
    fn same_object_same_id() {
        let obj = Object::Blob(Blob { data: Bytes::from("deterministic") });
        let d1 = serialize(&obj).unwrap();
        let d2 = serialize(&obj).unwrap();
        assert_eq!(object_id(&d1), object_id(&d2));
    }

    #[test]
    fn different_objects_different_ids() {
        let a = Object::Blob(Blob { data: Bytes::from("aaa") });
        let b = Object::Blob(Blob { data: Bytes::from("bbb") });
        let da = serialize(&a).unwrap();
        let db = serialize(&b).unwrap();
        assert_ne!(object_id(&da), object_id(&db));
    }
}
