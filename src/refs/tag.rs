// bole-prn
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

// bole-p8u
/// A named, immutable pointer to a specific snapshot.
///
/// Unlike a [`Timeline`](crate::refs::Timeline), a `Tag`'s `target` is fixed
/// after creation; it can only be moved by an explicit `move_tag` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    // bole-p8u
    /// The `ObjectId` of the snapshot this tag points to.
    pub target: ObjectId,
    // bole-p8u
    /// Unix timestamp (seconds) when this tag was created.
    pub created_at: u64,
    // bole-p8u
    /// Optional human-readable annotation attached to the tag (e.g. a release note).
    pub message: Option<String>,
}
