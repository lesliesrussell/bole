// bole-w3a
//! Integration tests for the timeline and tag command groups.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use bole::{EntryKind, Repository, Snapshot, TreeEntry};
use bytes::Bytes;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

/// Initialises a repo at `dir` and seeds `n` linear snapshots, returning their
/// hex ids oldest-first.
async fn seed(dir: &Path, n: usize) -> Vec<String> {
    let repo = Repository::disk(dir.join(".bole")).await.unwrap();
    let mut ids = Vec::new();
    let mut parents = Vec::new();
    for i in 0..n {
        let blob = repo
            .objects
            .put_blob(Bytes::from(format!("content-{i}")))
            .await
            .unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("file.txt".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot {
                root: tree,
                parents: parents.clone(),
                author: "test".into(),
                created_at: i as u64,
                message: format!("snap {i}"),
            })
            .await
            .unwrap();
        parents = vec![snap];
        ids.push(snap.to_string());
    }
    ids
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

#[tokio::test]
async fn timeline_create_list_show_advance_delete() {
    let dir = tempfile::tempdir().unwrap();
    let snaps = seed(dir.path(), 2).await;

    // create
    let c = run(dir.path(), &["timeline", "create", "main", "--from", &snaps[0]]);
    assert!(c.status.success(), "create failed: {c:?}");

    // list shows it
    let l = run(dir.path(), &["timeline", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&l.stdout).unwrap();
    assert_eq!(v[0]["name"], "main");
    assert_eq!(v[0]["head"], snaps[0]);

    // show
    let s = run(dir.path(), &["timeline", "show", "main", "--json"]);
    let sv: serde_json::Value = serde_json::from_slice(&s.stdout).unwrap();
    assert_eq!(sv["policy"], "unrestricted");

    // advance to the second snapshot
    let a = run(dir.path(), &["timeline", "advance", "main", "--to", &snaps[1]]);
    assert!(a.status.success(), "advance failed: {a:?}");
    let s2 = run(dir.path(), &["timeline", "show", "main", "--json"]);
    let sv2: serde_json::Value = serde_json::from_slice(&s2.stdout).unwrap();
    assert_eq!(sv2["head"], snaps[1]);

    // delete
    let d = run(dir.path(), &["timeline", "delete", "main"]);
    assert!(d.status.success(), "delete failed: {d:?}");
    let l2 = run(dir.path(), &["timeline", "list", "--json"]);
    let v2: serde_json::Value = serde_json::from_slice(&l2.stdout).unwrap();
    assert_eq!(v2.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn tag_create_target_by_timeline_ref() {
    let dir = tempfile::tempdir().unwrap();
    let snaps = seed(dir.path(), 1).await;
    run(dir.path(), &["timeline", "create", "main", "--from", &snaps[0]]);

    // tag targets a timeline by name -> resolves to its head
    let c = run(dir.path(), &["tag", "create", "v1", "--target", "main", "--message", "release"]);
    assert!(c.status.success(), "tag create failed: {c:?}");

    let s = run(dir.path(), &["tag", "show", "v1", "--json"]);
    let sv: serde_json::Value = serde_json::from_slice(&s.stdout).unwrap();
    assert_eq!(sv["target"], snaps[0]);
    assert_eq!(sv["message"], "release");

    // @main shortcut resolves identically
    let c2 = run(dir.path(), &["tag", "create", "v2", "--target", "@main"]);
    assert!(c2.status.success(), "tag create @main failed: {c2:?}");
    let s2 = run(dir.path(), &["tag", "show", "v2", "--json"]);
    let sv2: serde_json::Value = serde_json::from_slice(&s2.stdout).unwrap();
    assert_eq!(sv2["target"], snaps[0]);
}

#[tokio::test]
async fn branch_alias_lists_timelines() {
    let dir = tempfile::tempdir().unwrap();
    let snaps = seed(dir.path(), 1).await;
    run(dir.path(), &["timeline", "create", "main", "--from", &snaps[0]]);
    let l = run(dir.path(), &["branches", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&l.stdout).unwrap();
    assert_eq!(v[0]["name"], "main");
}
