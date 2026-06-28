// bole-0hg
//! Integration tests for the object/ref/store plumbing and repo info.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
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

#[test]
fn object_put_type_cat_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("f.txt"), b"plumbing payload").unwrap();

    let id = json(w, &["object", "put-blob", "f.txt"])["id"].as_str().unwrap().to_string();

    // type is blob
    assert_eq!(json(w, &["object", "type", &id])["kind"], "blob");

    // cat returns raw bytes
    let cat = ok(w, &["object", "cat", &id]);
    assert_eq!(cat.stdout, b"plumbing payload");

    // list contains the id
    let list = json(w, &["object", "list"]);
    assert!(list.as_array().unwrap().iter().any(|v| v == &serde_json::json!(id)));
}

#[test]
fn store_stats_and_fsck() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("a.txt"), b"a").unwrap();
    ok(w, &["object", "put-blob", "a.txt"]);

    let stats = json(w, &["store", "stats"]);
    assert!(stats["objects"].as_u64().unwrap() >= 1);

    let fsck = json(w, &["store", "fsck"]);
    assert_eq!(fsck["bad"], serde_json::json!([]));
}

#[test]
fn ref_list_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("a.txt"), b"a").unwrap();
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "x"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();
    ok(w, &["timeline", "create", "main", "--from", &id]);

    let list = json(w, &["ref", "list"]);
    assert_eq!(list, serde_json::json!(["main"]));

    assert_eq!(json(w, &["ref", "get", "main"])["kind"], "timeline");

    ok(w, &["ref", "delete", "main"]);
    assert_eq!(json(w, &["ref", "list"]), serde_json::json!([]));
}

#[test]
fn repo_info_reports_paths() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    let info = json(w, &["repo", "info"]);
    assert_eq!(info["backend"], "disk");
    assert!(info["repo_dir"].as_str().unwrap().ends_with(".bole"));
}
