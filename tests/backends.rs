// bole-a7c
use bole::{DiskBackend, MemoryBackend, ObjectStore};
use bytes::Bytes;
use tempfile::TempDir;

async fn backend_contract(store: ObjectStore) {
    let id = store.put_blob(Bytes::from("contract test")).await.unwrap();
    assert!(store.exists(&id).await.unwrap());
    let obj = store.get(&id).await.unwrap();
    assert!(obj.is_some());

    // non-existent id
    use bole::ObjectId;
    let missing = ObjectId::new([0u8; 32]);
    assert!(!store.exists(&missing).await.unwrap());
    assert!(store.get(&missing).await.unwrap().is_none());
}

#[tokio::test]
async fn memory_backend_contract() {
    backend_contract(ObjectStore::new(MemoryBackend::new())).await;
}

#[tokio::test]
async fn disk_backend_contract() {
    let dir = TempDir::new().unwrap();
    let backend = DiskBackend::open(dir.path()).await.unwrap();
    backend_contract(ObjectStore::new(backend)).await;
}

#[tokio::test]
async fn disk_backend_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let id = {
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let store = ObjectStore::new(backend);
        store.put_blob(Bytes::from("persisted across reopen")).await.unwrap()
    };
    let backend = DiskBackend::open(dir.path()).await.unwrap();
    let store = ObjectStore::new(backend);
    assert!(store.get(&id).await.unwrap().is_some());
}
