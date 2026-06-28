// bole-dq0
use bytes::Bytes;
use serde::{Deserialize, Serialize};

// bole-p8u
/// Raw byte payload stored as an immutable, content-addressed object.
///
/// A `Blob` holds uninterpreted bytes — file contents, configuration values,
/// or any other opaque data. Its identity in the store is derived entirely
/// from the content of `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    // bole-p8u
    /// The raw content of this blob.
    pub data: Bytes,
}
