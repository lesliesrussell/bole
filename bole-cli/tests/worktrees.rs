// bole-hrk
//! Integration tests for linked worktrees (workspace add/list/remove).

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

/// Primary repo bound to `main`, plus a `feature` timeline at the same snapshot.
fn setup(primary: &Path) {
    std::fs::write(primary.join("a.txt"), "base\n").unwrap();
    ok(primary, &["init", "."]);
    let snap = json(primary, &["snapshot", "create", "--from-workspace", "-m", "init"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();
    ok(primary, &["workspace", "open", "main", "--create", "--from", &id]);
    ok(primary, &["timeline", "create", "feature", "--from", &id]);
}

#[test]
fn add_list_status_isolation_remove() {
    let tmp = tempfile::tempdir().unwrap();
    let primary = tmp.path().join("primary");
    let feat = tmp.path().join("feat");
    std::fs::create_dir_all(&primary).unwrap();
    setup(&primary);

    // add a linked worktree bound to feature
    ok(&primary, &["workspace", "add", feat.to_str().unwrap(), "--timeline", "feature"]);
    assert!(feat.join(".bole").is_file(), "linked worktree should have a .bole pointer file");
    assert_eq!(std::fs::read_to_string(feat.join("a.txt")).unwrap(), "base\n", "head should be materialized");

    // list shows both; the linked one is on feature
    let list = json(&primary, &["workspace", "list"]);
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let linked = arr.iter().find(|e| e["linked"] == true).unwrap();
    assert_eq!(linked["timeline"], "feature");
    let primary_row = arr.iter().find(|e| e["linked"] == false).unwrap();
    assert_eq!(primary_row["timeline"], "main");

    // status from inside the linked worktree resolves the shared store + its own binding
    let st = json(&feat, &["status"]);
    assert_eq!(st["timeline"], "feature");
    assert!(st["repo_dir"].as_str().unwrap().contains("primary"), "linked worktree uses the primary store");

    // committing in the linked worktree advances feature, not main
    let main_before = json(&primary, &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();
    std::fs::write(feat.join("a.txt"), "feature change\n").unwrap();
    let snap2 = json(&feat, &["snapshot", "create", "--from-workspace", "-m", "feat edit"]);
    assert_eq!(snap2["advanced"], "feature");
    let main_after = json(&primary, &["timeline", "show", "main"])["head"].as_str().unwrap().to_string();
    assert_eq!(main_before, main_after, "primary main must be untouched");
    let feat_head = json(&primary, &["timeline", "show", "feature"])["head"].as_str().unwrap().to_string();
    assert_eq!(feat_head, snap2["snapshot"].as_str().unwrap());

    // remove unregisters but leaves the user's files
    ok(&primary, &["workspace", "remove", feat.to_str().unwrap()]);
    let list2 = json(&primary, &["workspace", "list"]);
    assert_eq!(list2.as_array().unwrap().len(), 1);
    assert!(!feat.join(".bole").exists(), "pointer file should be gone");
    assert!(feat.join("a.txt").exists(), "user files must be preserved");
}

#[test]
fn add_rejects_existing_repo_and_unknown_timeline() {
    let tmp = tempfile::tempdir().unwrap();
    let primary = tmp.path().join("primary");
    std::fs::create_dir_all(&primary).unwrap();
    setup(&primary);

    // adding onto the primary repo dir itself (already has .bole) fails
    let onto_primary = run(&primary, &["workspace", "add", primary.to_str().unwrap(), "--timeline", "feature"]);
    assert!(!onto_primary.status.success(), "must not clobber an existing .bole");

    // adding bound to a non-existent timeline fails
    let bad_tl = run(&primary, &["workspace", "add", tmp.path().join("x").to_str().unwrap(), "--timeline", "ghost"]);
    assert!(!bad_tl.status.success(), "unknown timeline should fail");
}

#[test]
fn remove_unknown_worktree_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let primary = tmp.path().join("primary");
    std::fs::create_dir_all(&primary).unwrap();
    setup(&primary);
    let out = run(&primary, &["workspace", "remove", tmp.path().join("nope").to_str().unwrap()]);
    assert!(!out.status.success(), "removing an unregistered worktree should fail");
}
