// bole-mbt
use async_trait::async_trait;
use bytes::Bytes;
use crate::error::Result;
use crate::object::ObjectId;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()>;
    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>>;
    async fn exists(&self, id: &ObjectId) -> Result<bool>;
    async fn delete(&self, id: &ObjectId) -> Result<()>;
    // bole-dq2
    async fn list(&self) -> Result<Vec<ObjectId>>;
}
