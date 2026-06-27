// bole-a7c
use bole::{EntryKind, MemoryBackend, Object, ObjectStore, Snapshot, TreeEntry};
use bytes::Bytes;
use std::collections::BTreeMap;

fn store() -> ObjectStore {
    ObjectStore::new(MemoryBackend::new())
}

#[tokio::test]
async fn t1_snapshots_are_immutable() {
    let s = store();
    let r1 = s.put_blob(Bytes::from("state-v1")).await.unwrap();
    let snap = Snapshot {
        root: r1, parents: vec![], author: "alice".into(),
        created_at: 1000, message: "initial".into(),
    };
    let id = s.put_snapshot(snap).await.unwrap();

    // "edit" produces new id
    let r2 = s.put_blob(Bytes::from("state-v2")).await.unwrap();
    let snap2 = Snapshot {
        root: r2, parents: vec![id], author: "alice".into(),
        created_at: 2000, message: "modified".into(),
    };
    let id2 = s.put_snapshot(snap2).await.unwrap();

    assert_ne!(id, id2);
    // original unchanged
    match s.get(&id).await.unwrap().unwrap() {
        Object::Snapshot(snap) => {
            assert_eq!(snap.message, "initial");
            assert_eq!(snap.parents, vec![]);
        }
        _ => panic!("expected snapshot"),
    }
}

#[tokio::test]
async fn t1_content_deduplication() {
    let s = store();
    let id1 = s.put_blob(Bytes::from("dedup test")).await.unwrap();
    let id2 = s.put_blob(Bytes::from("dedup test")).await.unwrap();
    assert_eq!(id1, id2);
}

#[tokio::test]
async fn t1_snapshot_parents_form_history() {
    let s = store();
    let root = s.put_blob(Bytes::from("root content")).await.unwrap();
    let s1 = s.put_snapshot(Snapshot {
        root, parents: vec![], author: "a".into(), created_at: 1, message: "s1".into(),
    }).await.unwrap();
    let s2 = s.put_snapshot(Snapshot {
        root, parents: vec![s1], author: "a".into(), created_at: 2, message: "s2".into(),
    }).await.unwrap();
    let s3 = s.put_snapshot(Snapshot {
        root, parents: vec![s2], author: "a".into(), created_at: 3, message: "s3".into(),
    }).await.unwrap();

    // all three independently retrievable
    assert!(s.get(&s1).await.unwrap().is_some());
    assert!(s.get(&s2).await.unwrap().is_some());
    assert!(s.get(&s3).await.unwrap().is_some());

    // parents link correctly
    match s.get(&s3).await.unwrap().unwrap() {
        Object::Snapshot(snap) => assert_eq!(snap.parents, vec![s2]),
        _ => panic!(),
    }
}

#[tokio::test]
async fn t1_tree_references_blobs() {
    let s = store();
    let file_id = s.put_blob(Bytes::from("file content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("README.md".into(), TreeEntry { id: file_id, kind: EntryKind::Blob });
    let tree_id = s.put_tree(entries).await.unwrap();

    match s.get(&tree_id).await.unwrap().unwrap() {
        Object::Tree(t) => {
            let entry = t.entries.get("README.md").unwrap();
            assert_eq!(entry.id, file_id);
        }
        _ => panic!("expected tree"),
    }
}
