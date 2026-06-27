// bole-b41
pub mod backend;
pub mod disk;
pub mod memory;

use bytes::Bytes;
use std::collections::BTreeMap;
use crate::codec;
use crate::error::Result;
use crate::object::{Blob, Object, ObjectId, Snapshot, Tree, TreeEntry};
use backend::StorageBackend;

pub struct ObjectStore {
    backend: Box<dyn StorageBackend>,
}

impl ObjectStore {
    pub fn new(backend: impl StorageBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    pub async fn put(&self, obj: &Object) -> Result<ObjectId> {
        let data = codec::serialize(obj)?;
        let id = codec::object_id(&data);
        if !self.backend.exists(&id).await? {
            self.backend.put(&id, &data).await?;
        }
        Ok(id)
    }

    pub async fn get(&self, id: &ObjectId) -> Result<Option<Object>> {
        match self.backend.get(id).await? {
            Some(data) => Ok(Some(codec::deserialize(&data)?)),
            None => Ok(None),
        }
    }

    pub async fn exists(&self, id: &ObjectId) -> Result<bool> {
        self.backend.exists(id).await
    }

    pub async fn put_blob(&self, data: Bytes) -> Result<ObjectId> {
        self.put(&Object::Blob(Blob { data })).await
    }

    pub async fn put_tree(&self, entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId> {
        self.put(&Object::Tree(Tree { entries })).await
    }

    pub async fn put_snapshot(&self, snap: Snapshot) -> Result<ObjectId> {
        self.put(&Object::Snapshot(snap)).await
    }

    // bole-dq2
    pub async fn list(&self) -> Result<Vec<ObjectId>> {
        self.backend.list().await
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectStore;
    use crate::object::{Blob, EntryKind, Object, Snapshot, TreeEntry};
    use crate::store::memory::MemoryBackend;
    use bytes::Bytes;
    use std::collections::BTreeMap;

    fn store() -> ObjectStore {
        ObjectStore::new(MemoryBackend::new())
    }

    #[tokio::test]
    async fn put_blob_returns_stable_id() {
        let s = store();
        let id1 = s.put_blob(Bytes::from("hello")).await.unwrap();
        let id2 = s.put_blob(Bytes::from("hello")).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn put_is_idempotent() {
        let s = store();
        let obj = Object::Blob(Blob { data: Bytes::from("same") });
        let id1 = s.put(&obj).await.unwrap();
        let id2 = s.put(&obj).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn get_returns_original_object() {
        let s = store();
        let id = s.put_blob(Bytes::from("retrieve me")).await.unwrap();
        let obj = s.get(&id).await.unwrap().unwrap();
        match obj {
            Object::Blob(b) => assert_eq!(b.data, Bytes::from("retrieve me")),
            _ => panic!("expected blob"),
        }
    }

    #[tokio::test]
    async fn snapshot_immutability() {
        let s = store();
        let r1 = s.put_blob(Bytes::from("v1")).await.unwrap();
        let snap1 = Snapshot {
            root: r1, parents: vec![], author: "alice".into(),
            created_at: 1, message: "first".into(),
        };
        let s1 = s.put_snapshot(snap1).await.unwrap();

        let r2 = s.put_blob(Bytes::from("v2")).await.unwrap();
        let snap2 = Snapshot {
            root: r2, parents: vec![s1], author: "alice".into(),
            created_at: 2, message: "second".into(),
        };
        let s2 = s.put_snapshot(snap2).await.unwrap();

        assert_ne!(s1, s2);
        let original = s.get(&s1).await.unwrap().unwrap();
        match original {
            Object::Snapshot(snap) => assert_eq!(snap.message, "first"),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn snapshot_parents_preserved() {
        let s = store();
        let root = s.put_blob(Bytes::from("root")).await.unwrap();
        let s1 = s.put_snapshot(Snapshot {
            root, parents: vec![], author: "a".into(), created_at: 1, message: "s1".into(),
        }).await.unwrap();
        let s2 = s.put_snapshot(Snapshot {
            root, parents: vec![s1], author: "a".into(), created_at: 2, message: "s2".into(),
        }).await.unwrap();
        match s.get(&s2).await.unwrap().unwrap() {
            Object::Snapshot(snap) => assert_eq!(snap.parents, vec![s1]),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn tree_round_trip() {
        let s = store();
        let blob_id = s.put_blob(Bytes::from("content")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("file.txt".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = s.put_tree(entries.clone()).await.unwrap();
        match s.get(&tree_id).await.unwrap().unwrap() {
            Object::Tree(t) => assert_eq!(t.entries, entries),
            _ => panic!("expected tree"),
        }
    }

    // bole-dq2
    #[tokio::test]
    async fn object_store_list() {
        let s = store();
        let id1 = s.put_blob(Bytes::from("foo")).await.unwrap();
        let id2 = s.put_blob(Bytes::from("bar")).await.unwrap();
        let ids = s.list().await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }
}
