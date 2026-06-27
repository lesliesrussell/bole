// bole-dq0
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;
// bole-hto
pub mod env;
pub mod secret;

pub use blob::Blob;
pub use id::ObjectId;
pub use snapshot::Snapshot;
pub use tree::{EntryKind, Tree, TreeEntry};
// bole-hto
pub use env::{EnvOverlay, EnvValue};
pub use secret::Secret;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
    // bole-hto
    Secret(Secret),
    EnvOverlay(EnvOverlay),
}
