// bole-49r
// bole-a7c
pub mod error;
pub mod object;
pub mod store;

pub(crate) mod codec;

pub use error::{Error, Result};
pub use object::{Blob, EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
pub use store::{
    backend::StorageBackend,
    disk::DiskBackend,
    memory::MemoryBackend,
    ObjectStore,
};
