// bole-tme
//! Integration tests for the merge and git command groups.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use bole::{EntryKind, ObjectId, Repository, Snapshot, TreeEntry};
use bytes::Bytes;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = args.to_vec();
    full.push("--json");
    let out = run(dir, &full);
    assert!(out.status.success(), "command {args:?} failed: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Stores a flat-file snapshot and returns its id.
async fn snap(repo: &Repository, files: &[(&str, &str)], parents: Vec<ObjectId>) -> ObjectId {
    let mut entries = BTreeMap::new();
    for (name, content) in files {
        let blob = repo.objects.put_blob(Bytes::from(content.to_string())).await.unwrap();
        entries.insert(name.to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
    }
    let root = repo.objects.put_tree(entries).await.unwrap();
    repo.objects
        .put_snapshot(Snapshot {
            root,
            parents,
            author: "test".into(),
            created_at: 0,
            message: "s".into(),
        })
        .await
        .unwrap()
}

/// Builds a repo with `main` and `feature` timelines diverging from a base.
async fn diverge(dir: &Path, main_files: &[(&str, &str)], feat_files: &[(&str, &str)]) {
    let repo = Repository::disk(dir.join(".bole")).await.unwrap();
    let base = snap(&repo, &[("a.txt", "base")], vec![]).await;
    let main_head = snap(&repo, main_files, vec![base]).await;
    let feat_head = snap(&repo, feat_files, vec![base]).await;
    let pol = bole::TimelinePolicy::Unrestricted;
    repo.refs.create_timeline(bole::RefName::new("main").unwrap(), main_head, pol.clone(), 0, "persistent".into(), None).unwrap();
    repo.refs.create_timeline(bole::RefName::new("feature").unwrap(), feat_head, pol, 0, "persistent".into(), None).unwrap();
}

#[tokio::test]
async fn merge_clean_advances_target() {
    let dir = tempfile::tempdir().unwrap();
    // main adds main.txt, feature adds feat.txt -> disjoint, clean merge.
    diverge(dir.path(), &[("a.txt", "base"), ("main.txt", "m")], &[("a.txt", "base"), ("feat.txt", "f")]).await;

    let before = json(dir.path(), &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();

    let check = json(dir.path(), &["merge", "check", "feature", "main"]);
    assert_eq!(check["verdict"], "allowed");

    let merged = json(dir.path(), &["merge", "run", "feature", "main", "-m", "merge feature"]);
    assert_eq!(merged["clean"], true);
    let merged_id = merged["snapshot"].as_str().unwrap().to_string();

    // main advanced to the merge snapshot.
    let after = json(dir.path(), &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();
    assert_ne!(before, after);
    assert_eq!(after, merged_id);

    // The merged snapshot contains files from both sides.
    let to = dir.path().join("out");
    assert!(run(dir.path(), &["workspace", "materialize", "--snapshot", &merged_id, "--to", to.to_str().unwrap()]).status.success());
    assert!(to.join("main.txt").exists());
    assert!(to.join("feat.txt").exists());
}

#[tokio::test]
async fn merge_conflict_does_not_advance() {
    let dir = tempfile::tempdir().unwrap();
    // Both edit a.txt differently -> conflict.
    diverge(dir.path(), &[("a.txt", "main-version")], &[("a.txt", "feature-version")]).await;

    let before = json(dir.path(), &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();
    let out = run(dir.path(), &["merge", "run", "feature", "main"]);
    assert!(!out.status.success(), "conflicting merge should fail");

    let after = json(dir.path(), &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();
    assert_eq!(before, after, "main must not advance on conflict");
}

#[tokio::test]
async fn git_export_creates_bare_repo() {
    let dir = tempfile::tempdir().unwrap();
    diverge(dir.path(), &[("a.txt", "base"), ("main.txt", "m")], &[("a.txt", "base"), ("feat.txt", "f")]).await;

    let out = dir.path().join("export.git");
    let res = run(dir.path(), &["git", "export", "--to", out.to_str().unwrap()]);
    assert!(res.status.success(), "export failed: {res:?}");
    assert!(out.join("HEAD").exists(), "exported repo missing HEAD");
}
