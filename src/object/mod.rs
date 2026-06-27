// bole-dq0
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;

pub use blob::Blob;
pub use id::ObjectId;
pub use snapshot::Snapshot;
pub use tree::{EntryKind, Tree, TreeEntry};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
}
