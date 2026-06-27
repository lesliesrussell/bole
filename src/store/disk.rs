// bole-2zd
// bole-4q6
use async_trait::async_trait;
use bytes::Bytes;
use std::path::{Path, PathBuf};
use tokio::fs;
use crate::error::{Error, Result};
use crate::object::ObjectId;
use super::backend::StorageBackend;

pub struct DiskBackend {
    root: PathBuf,
}

impl DiskBackend {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        let hex = id.to_string();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }
}

#[async_trait]
impl StorageBackend for DiskBackend {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()> {
        let path = self.object_path(id);
        if tokio::fs::try_exists(&path).await? {
            return Ok(());
        }
        let parent = path.parent().expect("object path always has a parent directory");
        tokio::fs::create_dir_all(parent).await?;
        let data = data.to_vec();
        let compressed = tokio::task::spawn_blocking(move || zstd::encode_all(data.as_slice(), 3))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
            .map_err(|e| Error::Storage(e.to_string()))?;
        // Write to a temp file then rename for atomicity
        let tmp_path = path.with_extension("tmp");
        tokio::fs::write(&tmp_path, &compressed).await?;
        tokio::fs::rename(&tmp_path, &path).await?;
        Ok(())
    }

    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>> {
        let path = self.object_path(id);
        match fs::read(&path).await {
            Ok(compressed) => {
                let data = tokio::task::spawn_blocking(move || {
                    zstd::decode_all(compressed.as_slice())
                })
                .await
                .map_err(|e| Error::Storage(e.to_string()))?
                .map_err(|e| Error::Storage(e.to_string()))?;
                Ok(Some(Bytes::from(data)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    async fn exists(&self, id: &ObjectId) -> Result<bool> {
        Ok(tokio::fs::try_exists(self.object_path(id)).await?)
    }

    async fn delete(&self, id: &ObjectId) -> Result<()> {
        match fs::remove_file(self.object_path(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DiskBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;
    use tempfile::TempDir;

    #[tokio::test]
    async fn put_then_get() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = {
            let backend = DiskBackend::open(dir.path()).await.unwrap();
            let id = ObjectId::from_bytes(b"persistent");
            backend.put(&id, b"data").await.unwrap();
            id
        };
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"data".as_slice()));
    }
}
