// bole-gvy
//! Integration tests for the workspace and snapshot command groups.

use std::path::Path;
use std::process::Command;

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

#[test]
fn full_workspace_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    std::fs::create_dir_all(w.join("src")).unwrap();
    std::fs::write(w.join("README.md"), "hello\n").unwrap();
    std::fs::write(w.join("src/main.rs"), "fn main(){}\n").unwrap();

    assert!(run(w, &["init", "."]).status.success());

    // First snapshot from the work tree (no timeline bound yet).
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "initial"]);
    assert_eq!(snap["files"], 2);
    assert!(snap["advanced"].is_null());
    let first = snap["snapshot"].as_str().unwrap().to_string();

    // Create+open a timeline at that snapshot.
    assert!(run(w, &["workspace", "open", "main", "--create", "--from", &first]).status.success());
    let st = json(w, &["status"]);
    assert_eq!(st["timeline"], "main");

    // Edit and add a file -> diff should report them.
    std::fs::write(w.join("README.md"), "hello world\n").unwrap();
    std::fs::write(w.join("NOTES.md"), "notes\n").unwrap();
    let d = json(w, &["workspace", "diff"]);
    assert_eq!(d["added"], serde_json::json!(["NOTES.md"]));
    assert_eq!(d["modified"], serde_json::json!(["README.md"]));

    // Commit advances main.
    let snap2 = json(w, &["snapshot", "create", "--from-workspace", "-m", "edits"]);
    assert_eq!(snap2["files"], 3);
    assert_eq!(snap2["advanced"], "main");
    let second = snap2["snapshot"].as_str().unwrap().to_string();

    // Work tree now matches head.
    let d2 = json(w, &["workspace", "diff"]);
    assert_eq!(d2["added"], serde_json::json!([]));
    assert_eq!(d2["modified"], serde_json::json!([]));

    // History walks parents newest-first.
    let list = json(w, &["snapshot", "list"]);
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["snapshot"], second);
    assert_eq!(list[1]["snapshot"], first);

    // Snapshot diff between the two revisions.
    let sd = json(w, &["snapshot", "diff", &first, &second]);
    assert_eq!(sd["added"], serde_json::json!(["NOTES.md"]));
    assert_eq!(sd["modified"], serde_json::json!(["README.md"]));

    // Parents of the second snapshot is the first.
    let parents = json(w, &["snapshot", "parents", &second]);
    assert_eq!(parents, serde_json::json!([first]));
}

#[test]
fn materialize_writes_files() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    std::fs::write(w.join("a.txt"), "aaa\n").unwrap();
    run(w, &["init", "."]);
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "x"]);
    let id = snap["snapshot"].as_str().unwrap();

    let out = dir.path().join("export");
    assert!(run(w, &["workspace", "materialize", "--snapshot", id, "--to", out.to_str().unwrap()]).status.success());
    assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "aaa\n");
}

#[test]
fn no_advance_keeps_timeline_head() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    std::fs::write(w.join("a.txt"), "1\n").unwrap();
    run(w, &["init", "."]);
    let s1 = json(w, &["snapshot", "create", "--from-workspace", "-m", "one"]);
    let first = s1["snapshot"].as_str().unwrap().to_string();
    run(w, &["workspace", "open", "main", "--create", "--from", &first]);

    std::fs::write(w.join("a.txt"), "2\n").unwrap();
    let s2 = json(w, &["snapshot", "create", "--from-workspace", "-m", "two", "--no-advance"]);
    assert!(s2["advanced"].is_null());

    // Head still points at the first snapshot.
    let show = json(w, &["timeline", "show", "main"]);
    assert_eq!(show["head"], first);
}
