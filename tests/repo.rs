// bole-6w7
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::RefName;
use bole::{materialize, Repository};
use bytes::Bytes;
use std::collections::BTreeMap;
use tempfile::TempDir;

/// T5: 1000 snapshot round-trip. Create in-memory repo with 1000 sequential
/// snapshots (each pointing to the previous as its parent), tag every 100th
/// snapshot, copy to disk, reload, verify all IDs and tag targets survive.
#[tokio::test]
async fn t5_memory_to_disk_round_trip() {
    let mem_repo = Repository::memory();

    let mut snap_ids = Vec::with_capacity(1000);
    let mut prev: Option<bole::ObjectId> = None;

    for i in 0u32..1000 {
        let content = i.to_le_bytes().to_vec();
        let blob_id = mem_repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert(
            "data".to_string(),
            TreeEntry { id: blob_id, kind: EntryKind::Blob },
        );
        let tree_id = mem_repo.objects.put_tree(entries).await.unwrap();
        let parents = prev.map_or_else(Vec::new, |p| vec![p]);
        let snap_id = mem_repo
            .objects
            .put_snapshot(Snapshot {
                root: tree_id,
                parents,
                author: "test".to_string(),
                created_at: i as u64,
                message: format!("snap {}", i),
            })
            .await
            .unwrap();
        snap_ids.push(snap_id);
        prev = Some(snap_id);
    }

    // Tag every 100th snapshot
    let mut tagged: Vec<(RefName, bole::ObjectId)> = Vec::new();
    for j in 0..10usize {
        let name = RefName::new(format!("milestone/{}", j)).unwrap();
        let target = snap_ids[j * 100];
        mem_repo
            .refs
            .create_tag(name.clone(), target, None, j as u64)
            .unwrap();
        tagged.push((name, target));
    }

    // Copy to disk repo
    let dir = TempDir::new().unwrap();
    let disk_repo = Repository::disk(dir.path()).await.unwrap();
    mem_repo.copy_to(&disk_repo).await.unwrap();
    drop(disk_repo);

    // Reload from same directory
    let reloaded = Repository::disk(dir.path()).await.unwrap();

    // All 1000 snapshot IDs must be present
    for snap_id in &snap_ids {
        assert!(
            reloaded.objects.exists(snap_id).await.unwrap(),
            "snapshot {} missing after reload",
            snap_id
        );
    }

    // All 10 tags must have correct targets
    for (name, expected_target) in &tagged {
        let tag = reloaded
            .refs
            .get_tag(name)
            .unwrap()
            .unwrap_or_else(|| panic!("tag {} missing after reload", name.as_str()));
        assert_eq!(
            tag.target, *expected_target,
            "tag {} has wrong target after reload",
            name.as_str()
        );
    }
}

/// T5: materialize and re-materialize. Build a 3-file nested snapshot in
/// memory, materialize to a temp dir, drop the dir, then materialize again
/// to a second dir and verify all contents still match (objects stay in store).
#[tokio::test]
async fn t5_materialize_and_rematerialize() {
    let repo = Repository::memory();

    // Build a small nested tree:
    //   src/main.rs  -> "fn main() {}"
    //   README.md    -> "hello"
    //   nested/a.txt -> "a"
    let main_blob = repo
        .objects
        .put_blob(Bytes::from("fn main() {}"))
        .await
        .unwrap();
    let readme_blob = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
    let nested_blob = repo.objects.put_blob(Bytes::from("a")).await.unwrap();

    let mut nested_entries = BTreeMap::new();
    nested_entries.insert(
        "a.txt".to_string(),
        TreeEntry { id: nested_blob, kind: EntryKind::Blob },
    );
    let nested_tree_id = repo.objects.put_tree(nested_entries).await.unwrap();

    let mut src_entries = BTreeMap::new();
    src_entries.insert(
        "main.rs".to_string(),
        TreeEntry { id: main_blob, kind: EntryKind::Blob },
    );
    let src_tree_id = repo.objects.put_tree(src_entries).await.unwrap();

    let mut root_entries = BTreeMap::new();
    root_entries.insert(
        "src".to_string(),
        TreeEntry { id: src_tree_id, kind: EntryKind::Tree },
    );
    root_entries.insert(
        "README.md".to_string(),
        TreeEntry { id: readme_blob, kind: EntryKind::Blob },
    );
    root_entries.insert(
        "nested".to_string(),
        TreeEntry { id: nested_tree_id, kind: EntryKind::Tree },
    );
    let root_tree_id = repo.objects.put_tree(root_entries).await.unwrap();

    let snap_id = repo
        .objects
        .put_snapshot(Snapshot {
            root: root_tree_id,
            parents: vec![],
            author: "test".to_string(),
            created_at: 1,
            message: "init".to_string(),
        })
        .await
        .unwrap();

    // First materialization
    let dest1 = TempDir::new().unwrap();
    materialize(&repo.objects, snap_id, dest1.path()).await.unwrap();
    assert_eq!(
        std::fs::read(dest1.path().join("src/main.rs")).unwrap(),
        b"fn main() {}"
    );
    assert_eq!(
        std::fs::read(dest1.path().join("README.md")).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::read(dest1.path().join("nested/a.txt")).unwrap(),
        b"a"
    );

    // Drop dest1 — TempDir auto-deletes on drop
    drop(dest1);

    // Second materialization from same in-memory repo — objects are still in store
    let dest2 = TempDir::new().unwrap();
    materialize(&repo.objects, snap_id, dest2.path()).await.unwrap();
    assert_eq!(
        std::fs::read(dest2.path().join("src/main.rs")).unwrap(),
        b"fn main() {}"
    );
    assert_eq!(
        std::fs::read(dest2.path().join("README.md")).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::read(dest2.path().join("nested/a.txt")).unwrap(),
        b"a"
    );
}
