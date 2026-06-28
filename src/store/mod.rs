// bole-b41
pub mod backend;
pub mod disk;
pub mod memory;

use bytes::Bytes;
use std::collections::BTreeMap;
use crate::codec;
use crate::error::Result;
use crate::object::{Blob, EnvOverlay, Object, ObjectId, Secret, Snapshot, Tree, TreeEntry};
use backend::StorageBackend;

// bole-p8u
/// The primary façade for reading and writing content-addressed objects.
///
/// `ObjectStore` wraps a [`StorageBackend`] and adds typed helpers so callers
/// never have to serialise objects manually.  Every `put_*` method returns the
/// `ObjectId` of the stored object; identical objects always produce the same
/// id and are deduplicated automatically.
pub struct ObjectStore {
    backend: Box<dyn StorageBackend>,
}

impl ObjectStore {
    // bole-p8u
    /// Creates an `ObjectStore` backed by the given [`StorageBackend`].
    pub fn new(backend: impl StorageBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    // bole-p8u
    /// Serialises `obj`, stores it if not already present, and returns its `ObjectId`.
    pub async fn put(&self, obj: &Object) -> Result<ObjectId> {
        let data = codec::serialize(obj)?;
        let id = codec::object_id(&data);
        if !self.backend.exists(&id).await? {
            self.backend.put(&id, &data).await?;
        }
        Ok(id)
    }

    // bole-p8u
    /// Retrieves and deserialises the object with the given `id`, or `None` if it does not exist.
    pub async fn get(&self, id: &ObjectId) -> Result<Option<Object>> {
        match self.backend.get(id).await? {
            Some(data) => Ok(Some(codec::deserialize(&data)?)),
            None => Ok(None),
        }
    }

    // bole-p8u
    /// Returns `true` if an object with the given `id` exists in the store.
    pub async fn exists(&self, id: &ObjectId) -> Result<bool> {
        self.backend.exists(id).await
    }

    // bole-p8u
    /// Stores `data` as a [`Blob`] and returns its `ObjectId`.
    pub async fn put_blob(&self, data: Bytes) -> Result<ObjectId> {
        self.put(&Object::Blob(Blob { data })).await
    }

    // bole-p8u
    /// Stores the given entries as a [`Tree`] and returns its `ObjectId`.
    pub async fn put_tree(&self, entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId> {
        self.put(&Object::Tree(Tree { entries })).await
    }

    // bole-p8u
    /// Stores `snap` as a [`Snapshot`] and returns its `ObjectId`.
    pub async fn put_snapshot(&self, snap: Snapshot) -> Result<ObjectId> {
        self.put(&Object::Snapshot(snap)).await
    }

    // bole-dq2
    // bole-p8u
    /// Returns the `ObjectId` of every object currently in the store.
    pub async fn list(&self) -> Result<Vec<ObjectId>> {
        self.backend.list().await
    }

    // bole-meg
    // bole-p8u
    /// Encrypts `plaintext` with `key`, stores the result as a [`Secret`], and returns its `ObjectId`.
    ///
    /// Because a fresh nonce is generated on each call, the same plaintext
    /// stored twice will produce two different `ObjectId`s.
    pub async fn put_secret(&self, plaintext: &[u8], key: &[u8; 32]) -> Result<ObjectId> {
        let secret = Secret::encrypt(plaintext, key)?;
        self.put(&Object::Secret(secret)).await
    }

    // bole-p8u
    /// Fetches the [`Secret`] at `id`, decrypts it with `key`, and returns the plaintext,
    /// or `None` if no object exists at that id.
    pub async fn get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        match self.get(id).await? {
            None => Ok(None),
            Some(Object::Secret(s)) => Ok(Some(s.decrypt(key)?)),
            Some(_) => Err(crate::error::Error::Codec("not a secret".into())),
        }
    }

    // bole-p8u
    /// Stores `overlay` as an [`EnvOverlay`] and returns its `ObjectId`.
    pub async fn put_overlay(&self, overlay: EnvOverlay) -> Result<ObjectId> {
        self.put(&Object::EnvOverlay(overlay)).await
    }

    // bole-p8u
    /// Fetches and returns the [`EnvOverlay`] at `id`, or `None` if it does not exist.
    pub async fn get_overlay(&self, id: &ObjectId) -> Result<Option<EnvOverlay>> {
        match self.get(id).await? {
            None => Ok(None),
            Some(Object::EnvOverlay(o)) => Ok(Some(o)),
            Some(_) => Err(crate::error::Error::Codec("not an env overlay".into())),
        }
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

    // bole-meg
    #[tokio::test]
    async fn put_secret_roundtrip() {
        let s = store();
        let key = [1u8; 32];
        let id = s.put_secret(b"value", &key).await.unwrap();
        let got = s.get_secret(&id, &key).await.unwrap().unwrap();
        assert_eq!(got, b"value");
    }

    #[tokio::test]
    async fn get_secret_wrong_key_returns_err() {
        let s = store();
        let key = [1u8; 32];
        let wrong_key = [2u8; 32];
        let id = s.put_secret(b"secret", &key).await.unwrap();
        let err = s.get_secret(&id, &wrong_key).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::DecryptionFailed));
    }

    #[tokio::test]
    async fn get_secret_missing_returns_none() {
        let s = store();
        let key = [1u8; 32];
        let id = crate::object::ObjectId::new([9u8; 32]);
        let got = s.get_secret(&id, &key).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn put_overlay_get_overlay_roundtrip() {
        use crate::object::{EnvOverlay, EnvValue, ObjectId};
        use std::collections::BTreeMap;
        let s = store();
        let mut entries = BTreeMap::new();
        entries.insert("LOG".into(), EnvValue::Plain("info".into()));
        entries.insert("KEY".into(), EnvValue::Secret(ObjectId::new([7u8; 32])));
        let overlay = EnvOverlay { entries };
        let id = s.put_overlay(overlay.clone()).await.unwrap();
        let got = s.get_overlay(&id).await.unwrap().unwrap();
        assert_eq!(got, overlay);
    }

    #[tokio::test]
    async fn get_secret_on_non_secret_object_returns_err() {
        let s = store();
        let blob_id = s.put_blob(Bytes::from("not a secret")).await.unwrap();
        let key = [1u8; 32];
        let err = s.get_secret(&blob_id, &key).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Codec(_)));
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

    // bole-7vf
    #[tokio::test]
    async fn put_secret_identical_plaintext_produces_distinct_ids() {
        let s = store();
        let key = [1u8; 32];
        let id1 = s.put_secret(b"same plaintext", &key).await.unwrap();
        let id2 = s.put_secret(b"same plaintext", &key).await.unwrap();
        assert_ne!(id1, id2, "identical plaintext must produce distinct ObjectIds (nonce prevents equality leakage)");
    }
}
