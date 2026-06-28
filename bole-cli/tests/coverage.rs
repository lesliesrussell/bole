// bole-0sd
//! Coverage tests for every CLI subcommand and flag not exercised by the
//! domain-specific test files (cli/refs/workspace/acl/merge/secret/plumbing).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use bole::{EntryKind, ObjectId, Repository, Snapshot, TreeEntry};
use bytes::Bytes;

const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bole"));
    c.env("BOLE_KEY", KEY);
    c
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = run(dir, args);
    assert!(out.status.success(), "command {args:?} failed: {out:?}");
    out
}

fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = args.to_vec();
    full.push("--json");
    let out = ok(dir, &full);
    serde_json::from_slice(&out.stdout).unwrap()
}

/// init + first snapshot bound to `main`, returns the snapshot id.
fn init_with_main(w: &Path) -> String {
    std::fs::write(w.join("a.txt"), "1\n").unwrap();
    ok(w, &["init", "."]);
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "init"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();
    ok(w, &["workspace", "open", "main", "--create", "--from", &id]);
    id
}

// ---------------------------------------------------------------- tags

#[test]
fn tag_list_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    let id = init_with_main(w);
    ok(w, &["tag", "create", "v1", "--target", &id]);
    ok(w, &["tag", "create", "v2", "--target", "@main"]);

    let list = json(w, &["tag", "list"]);
    assert_eq!(list.as_array().unwrap().len(), 2);

    ok(w, &["tag", "delete", "v1"]);
    let list2 = json(w, &["tag", "list"]);
    assert_eq!(list2.as_array().unwrap().len(), 1);

    // deleting a missing tag fails
    assert!(!run(w, &["tag", "delete", "nope"]).status.success());
}

// ---------------------------------------------------------------- snapshot

#[test]
fn snapshot_show_and_author() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);
    std::fs::write(w.join("b.txt"), "2\n").unwrap();
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "second", "--author", "alice"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();

    let show = json(w, &["snapshot", "show", &id]);
    assert_eq!(show["author"], "alice");
    assert_eq!(show["message"], "second");
    assert_eq!(show["files"], 2);
    assert_eq!(show["parents"].as_array().unwrap().len(), 1);

    // @ resolves to the bound timeline head
    let show_at = json(w, &["snapshot", "show", "@"]);
    assert_eq!(show_at["snapshot"], id);

    // bad reference fails
    assert!(!run(w, &["snapshot", "show", "nope"]).status.success());
}

// ---------------------------------------------------------------- workspace

#[test]
fn workspace_show_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);

    std::fs::write(w.join("c.txt"), "3\n").unwrap();
    let show = json(w, &["workspace", "show"]);
    assert_eq!(show["timeline"], "main");
    assert_eq!(show["pending"]["added"], serde_json::json!(["c.txt"]));

    ok(w, &["workspace", "clear"]);
    let show2 = json(w, &["workspace", "show"]);
    assert!(show2["timeline"].is_null());
}

#[test]
fn workspace_open_as_binds_actor() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);
    ok(w, &["actor", "create", "alice"]);
    ok(w, &["actor", "grant-path", "alice", "**", "write"]);
    ok(w, &["actor", "grant-timeline", "alice", "**", "write"]);

    ok(w, &["workspace", "open", "main", "--as", "alice"]);
    let st = json(w, &["status"]);
    assert_eq!(st["actor"], "alice");

    // opening with an unknown actor fails
    assert!(!run(w, &["workspace", "open", "main", "--as", "ghost"]).status.success());
}

// ---------------------------------------------------------------- actor

#[test]
fn actor_list_current_and_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);

    assert_eq!(json(w, &["actor", "current"])["actor"], serde_json::Value::Null);

    ok(w, &["actor", "create", "a"]);
    ok(w, &["actor", "create", "b"]);
    assert!(!run(w, &["actor", "create", "a"]).status.success(), "duplicate should fail");

    let list = json(w, &["actor", "list"]);
    assert_eq!(list, serde_json::json!(["a", "b"]));

    ok(w, &["actor", "use", "b"]);
    assert_eq!(json(w, &["actor", "current"])["actor"], "b");
    // using an unknown actor fails
    assert!(!run(w, &["actor", "use", "ghost"]).status.success());
}

// ---------------------------------------------------------------- acl

#[test]
fn acl_timeline_rules_and_can_read() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);

    ok(w, &["acl", "timeline", "protect", "release/**"]);
    assert_eq!(json(w, &["acl", "timeline", "list"]), serde_json::json!(["release/**"]));
    ok(w, &["acl", "timeline", "unprotect", "release/**"]);
    assert_eq!(json(w, &["acl", "timeline", "list"]), serde_json::json!([]));

    ok(w, &["actor", "create", "reader"]);
    ok(w, &["actor", "grant-path", "reader", "docs/**", "read"]);
    ok(w, &["actor", "grant-timeline", "reader", "main", "read"]);

    assert_eq!(json(w, &["acl", "can-read-path", "--actor", "reader", "docs/x.md"])["allowed"], true);
    assert_eq!(json(w, &["acl", "can-read-path", "--actor", "reader", "src/x.rs"])["allowed"], false);
    assert_eq!(json(w, &["acl", "can-read-timeline", "--actor", "reader", "main"])["allowed"], true);
    assert_eq!(json(w, &["acl", "can-read-timeline", "--actor", "reader", "other"])["allowed"], false);
}

// ---------------------------------------------------------------- secret

#[test]
fn secret_rotate_changes_value() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("v1"), b"one").unwrap();
    ok(w, &["secret", "put", "k", "--from-file", "v1"]);
    assert_eq!(json(w, &["secret", "reveal", "k"])["value"], "one");

    std::fs::write(w.join("v2"), b"two").unwrap();
    ok(w, &["secret", "rotate", "k", "--from-file", "v2"]);
    assert_eq!(json(w, &["secret", "reveal", "k"])["value"], "two");

    // rotating a missing secret fails; revealing a missing secret fails
    assert!(!run(w, &["secret", "rotate", "missing", "--from-file", "v2"]).status.success());
    assert!(!run(w, &["secret", "reveal", "missing"]).status.success());
    // putting an existing secret fails (must rotate)
    assert!(!run(w, &["secret", "put", "k", "--from-file", "v2"]).status.success());
}

// ---------------------------------------------------------------- env errors

#[test]
fn env_set_on_missing_overlay_errors() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    assert!(!run(w, &["env", "set", "nope", "X", "1"]).status.success());
    ok(w, &["env", "create", "dev"]);
    assert!(!run(w, &["env", "create", "dev"]).status.success(), "duplicate overlay should fail");
}

// ---------------------------------------------------------------- object get

#[test]
fn object_get_shows_kind() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("f.txt"), b"hi").unwrap();
    let id = json(w, &["object", "put-blob", "f.txt"])["id"].as_str().unwrap().to_string();
    let got = json(w, &["object", "get", &id]);
    assert_eq!(got["kind"], "blob");
    assert_eq!(got["id"], id);
    // unknown id fails
    let bad = "0".repeat(64);
    assert!(!run(w, &["object", "get", &bad]).status.success());
}

// ---------------------------------------------------------------- timeline policy + branch alias

#[test]
fn timeline_policy_is_stored_and_branch_alias_works() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    let id = init_with_main(w);

    // branch alias creates a timeline with a chosen policy
    ok(w, &["branch", "create", "ff-line", "--from", &id, "--policy", "ff"]);
    assert_eq!(json(w, &["timeline", "show", "ff-line"])["policy"], "ff");

    ok(w, &["branch", "create", "app-line", "--from", &id, "--policy", "append"]);
    assert_eq!(json(w, &["timeline", "show", "app-line"])["policy"], "append");

    // unknown timeline show fails
    assert!(!run(w, &["timeline", "show", "ghost"]).status.success());
}

// ---------------------------------------------------------------- quiet flag

#[test]
fn quiet_suppresses_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    init_with_main(w);
    let out = ok(w, &["status", "--quiet"]);
    assert!(out.stdout.is_empty(), "quiet should produce no stdout, got {:?}", out.stdout);
}

// ---------------------------------------------------------------- merge verdicts

/// Stores a flat-file snapshot and returns its id.
async fn snap(repo: &Repository, files: &[(&str, &str)], parents: Vec<ObjectId>) -> ObjectId {
    let mut entries = BTreeMap::new();
    for (name, content) in files {
        let blob = repo.objects.put_blob(Bytes::from(content.to_string())).await.unwrap();
        entries.insert(name.to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
    }
    let root = repo.objects.put_tree(entries).await.unwrap();
    repo.objects
        .put_snapshot(Snapshot { root, parents, author: "t".into(), created_at: 0, message: "s".into() })
        .await
        .unwrap()
}

/// Builds main + feature where feature carries `secret/key.txt`.
async fn diverge_with_secret_path(dir: &Path) {
    let repo = Repository::disk(dir.join(".bole")).await.unwrap();
    let base = snap(&repo, &[("a.txt", "base")], vec![]).await;
    let main_head = snap(&repo, &[("a.txt", "base"), ("main.txt", "m")], vec![base]).await;
    let feat_head = snap(&repo, &[("a.txt", "base"), ("secret/key.txt", "k")], vec![base]).await;
    let pol = bole::TimelinePolicy::Unrestricted;
    repo.refs.create_timeline(bole::RefName::new("main").unwrap(), main_head, pol.clone(), 0, "persistent".into(), None).unwrap();
    repo.refs.create_timeline(bole::RefName::new("feature").unwrap(), feat_head, pol, 0, "persistent".into(), None).unwrap();
}

#[tokio::test]
async fn merge_check_requires_approval_and_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    diverge_with_secret_path(w).await;

    // Protect the path that feature would leak into the unprotected main.
    ok(w, &["acl", "path", "protect", "secret/**"]);

    // No actor bound -> full access can write main -> requires-approval.
    assert_eq!(json(w, &["merge", "check", "feature", "main"])["verdict"], "requires-approval");

    // An actor without write on main -> rejected.
    ok(w, &["actor", "create", "ro"]);
    ok(w, &["actor", "grant-timeline", "ro", "other/**", "write"]);
    ok(w, &["actor", "use", "ro"]);
    assert_eq!(json(w, &["merge", "check", "feature", "main"])["verdict"], "rejected");
}
