// bole-49r
// bole-a7c
// bole-s5y
pub mod error;
pub mod object;
pub mod refs;
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
