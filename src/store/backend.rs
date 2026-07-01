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

    // bole-81z
    /// Consolidates loose objects into packs where the backend supports it.
    /// Returns the number of objects packed. Default: no-op (0) for backends
    /// without packs (memory, loose-only disk).
    async fn compact(&self) -> Result<u64> {
        Ok(0)
    }

    // bole-81z
    /// Removes every object NOT in `reachable`. The default deletes unreachable
    /// objects individually (used by memory/loose-only backends and ignores the
    /// grace window). Packed backends override to rewrite packs keeping only the
    /// reachable set and to honour `grace_secs` for recently-written loose
    /// objects (`now` is a unix-seconds clock supplied by the caller). Returns
    /// the number of objects removed.
    async fn sweep(
        &self,
        reachable: &std::collections::HashSet<ObjectId>,
        _grace_secs: u64,
        _now: u64,
    ) -> Result<u64> {
        let mut removed = 0u64;
        for id in self.list().await? {
            if !reachable.contains(&id) {
                self.delete(&id).await?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}
