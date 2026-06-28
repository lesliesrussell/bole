// bole-dq0
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

// bole-p8u
/// An immutable point-in-time record linking a root tree to its ancestry.
///
/// Snapshots are the nodes of the snapshot DAG that backs each timeline.
/// They record who made a change, when, and what the full tree looked like,
/// without storing diffs — the store's content-addressing handles deduplication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    // bole-p8u
    /// The root `Tree` object describing the full path hierarchy at this point in time.
    pub root: ObjectId,
    // bole-p8u
    /// Identifiers of the snapshots this snapshot was derived from; empty for the initial snapshot.
    pub parents: Vec<ObjectId>,
    // bole-p8u
    /// Human-readable identity of whoever created this snapshot.
    pub author: String,
    // bole-p8u
    /// Unix timestamp (seconds) when this snapshot was recorded.
    pub created_at: u64,
    // bole-p8u
    /// Human-readable description of what changed in this snapshot.
    pub message: String,
}
