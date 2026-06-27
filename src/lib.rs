// bole-49r
// bole-a7c
// bole-s5y
pub mod error;
pub mod object;
pub mod refs;
pub mod store;
// bole-1vi
pub mod repo;

pub(crate) mod codec;

pub use error::{Error, Result};
pub use object::{Blob, EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
// bole-wmu
pub use refs::{
    backend::RefBackend,
    disk::DiskRefBackend,
    memory::MemoryRefBackend,
    Ref, RefName, RefStore, Tag, Timeline, TimelinePolicy,
};
pub use store::{
    backend::StorageBackend,
    disk::DiskBackend,
    memory::MemoryBackend,
    ObjectStore,
};
// bole-1vi
pub use repo::{copy_objects, materialize::materialize, Repository};
