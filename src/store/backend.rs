// bole-mbt
use async_trait::async_trait;
use bytes::Bytes;
use crate::error::Result;
use crate::object::ObjectId;

// bole-p8u
/// Low-level async key-value interface that object stores are built on top of.
///
/// Implementors are responsible only for raw byte storage; serialisation,
/// content-addressing, and type safety are handled by [`crate::store::ObjectStore`].
/// The two built-in implementations are [`crate::store::memory::MemoryBackend`]
/// and [`crate::store::disk::DiskBackend`].
#[async_trait]
pub trait StorageBackend: Send + Sync {
    // bole-p8u
    /// Stores `data` under `id`, creating the entry if it does not already exist.
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()>;
    // bole-p8u
    /// Returns the raw bytes stored under `id`, or `None` if no entry exists.
    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>>;
    // bole-p8u
    /// Returns `true` if an entry exists for `id`.
    async fn exists(&self, id: &ObjectId) -> Result<bool>;
    // bole-p8u
    /// Removes the entry for `id`; succeeds silently if no entry exists.
    async fn delete(&self, id: &ObjectId) -> Result<()>;
    // bole-dq2
    // bole-p8u
    /// Returns the ids of all objects currently in the store.
    async fn list(&self) -> Result<Vec<ObjectId>>;

    // bole-81z
    /// Returns the number of distinct objects in the store. The default counts
    /// `list()`; backends with a cheaper path (e.g. pack index headers) override.
    async fn count(&self) -> Result<u64> {
        Ok(self.list().await?.len() as u64)
    }
}
