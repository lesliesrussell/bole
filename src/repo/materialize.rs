// bole-2l6
use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId};
use crate::store::ObjectStore;
use std::path::Path;

// bole-p8u
/// Recursively writes the tree reachable from `snapshot_id` to the filesystem
/// at `dest`, creating directories and files to mirror the stored path hierarchy.
///
/// Existing files are overwritten; directories are created as needed.  This is
/// a read-only export — changes to the materialized files are not tracked.
pub async fn materialize(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
    dest: impl AsRef<Path>,
) -> Result<()> {
    let dest = dest.as_ref();
    tokio::fs::create_dir_all(dest).await?;
    let snap = match objects.get(&snapshot_id).await? {
        Some(Object::Snapshot(s)) => s,
        Some(_) => return Err(Error::Storage(format!("{} is not a snapshot", snapshot_id))),
        None => return Err(Error::Storage(format!("snapshot not found: {}", snapshot_id))),
    };
    write_tree(objects, snap.root, dest).await
}

async fn write_tree(objects: &ObjectStore, tree_id: ObjectId, base: &Path) -> Result<()> {
    let tree = match objects.get(&tree_id).await? {
        Some(Object::Tree(t)) => t,
        Some(_) => return Err(Error::Storage(format!("{} is not a tree", tree_id))),
        None => return Err(Error::Storage(format!("tree not found: {}", tree_id))),
    };
    for (name, entry) in &tree.entries {
        let path = base.join(name);
        match entry.kind {
            EntryKind::Blob => match objects.get(&entry.id).await? {
                Some(Object::Blob(b)) => tokio::fs::write(&path, &b.data).await?,
                Some(_) => return Err(Error::Storage(format!("{} is not a blob", entry.id))),
                None => return Err(Error::Storage(format!("blob not found: {}", entry.id))),
            },
            EntryKind::Tree => {
                tokio::fs::create_dir_all(&path).await?;
                Box::pin(write_tree(objects, entry.id, &path)).await?;
            }
        }
    }
    Ok(())
}

// bole-2l6
#[cfg(test)]
mod tests {
    use super::materialize;
    use crate::object::{EntryKind, ObjectId, Snapshot, TreeEntry};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use bytes::Bytes;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn store() -> ObjectStore {
        ObjectStore::new(MemoryBackend::new())
    }

    #[tokio::test]
    async fn missing_snapshot_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let id = ObjectId::new([9u8; 32]);
        let err = materialize(&s, id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }

    #[tokio::test]
    async fn wrong_object_type_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let blob_id = s.put_blob(Bytes::from("not a snapshot")).await.unwrap();
        let err = materialize(&s, blob_id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }

    #[tokio::test]
    async fn simple_flat_tree() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let blob_id = s.put_blob(Bytes::from("hello world")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert(
            "hello.txt".into(),
            TreeEntry {
                id: blob_id,
                kind: EntryKind::Blob,
            },
        );
        let tree_id = s.put_tree(entries).await.unwrap();
        let snap_id = s
            .put_snapshot(Snapshot {
                root: tree_id,
                parents: vec![],
                author: "test".into(),
                created_at: 1,
                message: "m".into(),
            })
            .await
            .unwrap();
        materialize(&s, snap_id, dir.path()).await.unwrap();
        let content = std::fs::read(dir.path().join("hello.txt")).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn nested_directory_tree() {
        let s = store();
        let dir = TempDir::new().unwrap();

        let nested_blob = s.put_blob(Bytes::from("nested content")).await.unwrap();
        let mut nested_entries = BTreeMap::new();
        nested_entries.insert(
            "file.txt".into(),
            TreeEntry {
                id: nested_blob,
                kind: EntryKind::Blob,
            },
        );
        let nested_tree = s.put_tree(nested_entries).await.unwrap();

        let root_blob = s.put_blob(Bytes::from("root content")).await.unwrap();
        let mut root_entries = BTreeMap::new();
        root_entries.insert(
            "root.txt".into(),
            TreeEntry {
                id: root_blob,
                kind: EntryKind::Blob,
            },
        );
        root_entries.insert(
            "sub".into(),
            TreeEntry {
                id: nested_tree,
                kind: EntryKind::Tree,
            },
        );
        let root_tree = s.put_tree(root_entries).await.unwrap();

        let snap_id = s
            .put_snapshot(Snapshot {
                root: root_tree,
                parents: vec![],
                author: "test".into(),
                created_at: 1,
                message: "m".into(),
            })
            .await
            .unwrap();
        materialize(&s, snap_id, dir.path()).await.unwrap();

        assert_eq!(
            std::fs::read(dir.path().join("root.txt")).unwrap(),
            b"root content"
        );
        assert_eq!(
            std::fs::read(dir.path().join("sub/file.txt")).unwrap(),
            b"nested content"
        );
    }

    #[tokio::test]
    async fn missing_blob_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let missing_blob = ObjectId::new([7u8; 32]);
        let mut entries = BTreeMap::new();
        entries.insert(
            "gone.txt".into(),
            TreeEntry {
                id: missing_blob,
                kind: EntryKind::Blob,
            },
        );
        let tree_id = s.put_tree(entries).await.unwrap();
        let snap_id = s
            .put_snapshot(Snapshot {
                root: tree_id,
                parents: vec![],
                author: "test".into(),
                created_at: 1,
                message: "m".into(),
            })
            .await
            .unwrap();
        let err = materialize(&s, snap_id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }
}
