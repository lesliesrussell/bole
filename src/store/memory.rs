// bole-mbt
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::error::Result;
use crate::object::ObjectId;
use super::backend::StorageBackend;

#[derive(Debug, Clone, Default)]
pub struct MemoryBackend {
    store: Arc<RwLock<HashMap<[u8; 32], Bytes>>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StorageBackend for MemoryBackend {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()> {
        self.store
            .write()
            .await
            .insert(*id.as_bytes(), Bytes::copy_from_slice(data));
        Ok(())
    }

    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>> {
        Ok(self.store.read().await.get(id.as_bytes()).cloned())
    }

    async fn exists(&self, id: &ObjectId) -> Result<bool> {
        Ok(self.store.read().await.contains_key(id.as_bytes()))
    }

    async fn delete(&self, id: &ObjectId) -> Result<()> {
        self.store.write().await.remove(id.as_bytes());
        Ok(())
    }

    // bole-dq2
    async fn list(&self) -> Result<Vec<ObjectId>> {
        Ok(self.store.read().await
            .keys()
            .map(|k| ObjectId::new(*k))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;

    #[tokio::test]
    async fn put_then_get() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let backend = MemoryBackend::new();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }

    // bole-dq2
    #[tokio::test]
    async fn list_returns_all_ids() {
        let backend = MemoryBackend::new();
        let id1 = ObjectId::from_bytes(b"a");
        let id2 = ObjectId::from_bytes(b"b");
        let id3 = ObjectId::from_bytes(b"c");
        backend.put(&id1, b"data1").await.unwrap();
        backend.put(&id2, b"data2").await.unwrap();
        backend.put(&id3, b"data3").await.unwrap();
        let ids = backend.list().await.unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
    }

    #[tokio::test]
    async fn list_empty_store_returns_empty() {
        let backend = MemoryBackend::new();
        assert!(backend.list().await.unwrap().is_empty());
    }
}
