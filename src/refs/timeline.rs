// bole-prn
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

// bole-p8u
/// The advancement rule that governs how a timeline's head may be moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimelinePolicy {
    // bole-p8u
    /// The new head must be a direct descendant of the current head (no rewriting history).
    FastForwardOnly,
    // bole-p8u
    /// Snapshots may only be appended; arbitrary rewrites are not permitted.
    Append,
    // bole-p8u
    /// The head may be set to any snapshot regardless of its ancestry.
    Unrestricted,
}

// bole-p8u
/// The live state of a named timeline: its current head snapshot, its
/// advancement policy, and optional lifecycle metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    // bole-p8u
    /// The `ObjectId` of the most recent snapshot on this timeline.
    pub head: ObjectId,
    // bole-p8u
    /// The rule that constrains how `head` may be advanced.
    pub policy: TimelinePolicy,
    // bole-p8u
    /// Unix timestamp (seconds) when this timeline was created.
    pub created_at: u64,
    // bole-qv5
    // bole-p8u
    /// Lifecycle category for the timeline (e.g. `"persistent"` or `"ephemeral"`).
    #[serde(default = "default_kind")]
    pub kind: String,
    // bole-p8u
    /// Optional Unix timestamp after which this timeline may be pruned.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

// bole-qv5
fn default_kind() -> String {
    "persistent".into()
}
