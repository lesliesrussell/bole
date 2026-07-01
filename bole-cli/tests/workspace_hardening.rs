// bole-3hj
//! Integration tests for linked-worktree hardening (prune / repair / list --check).

use std::path::{Path, PathBuf};
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

/// Sets up a primary repo bound to `main` with a `feature` timeline, and adds a
/// linked worktree `feat` (id "feat") bound to `feature`. Returns (primary, feat).
fn setup_with_linked(tmp: &Path) -> (PathBuf, PathBuf) {
    let primary = tmp.join("primary");
    let feat = tmp.join("feat");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::write(primary.join("a.txt"), "base\n").unwrap();
    ok(&primary, &["init", "."]);
    let snap = json(&primary, &["snapshot", "create", "--from-workspace", "-m", "init"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();
    ok(&primary, &["workspace", "open", "main", "--create", "--from", &id]);
    ok(&primary, &["timeline", "create", "feature", "--from", &id]);
    ok(&primary, &["workspace", "add", feat.to_str().unwrap(), "--timeline", "feature"]);
    (primary, feat)
}

fn linked_count(primary: &Path) -> usize {
    json(primary, &["workspace", "list"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["linked"] == true)
        .count()
}

#[test]
fn prune_missing_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());
    std::fs::remove_dir_all(&feat).unwrap();

    let out = json(&primary, &["workspace", "prune"]);
    assert_eq!(out.as_array().unwrap().len(), 1);
    assert_eq!(out[0]["status"], "missing-directory");
    assert_eq!(out[0]["pruned"], true);
    assert_eq!(linked_count(&primary), 0, "registry entry removed");
    // Metadata dir gone.
    assert!(!primary.join(".bole/worktrees/feat").exists());
}

#[test]
fn prune_missing_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());
    std::fs::remove_file(feat.join(".bole")).unwrap();

    let out = json(&primary, &["workspace", "prune"]);
    assert_eq!(out[0]["status"], "missing-pointer");
    assert_eq!(linked_count(&primary), 0);
}

#[test]
fn prune_bad_pointer_removes_only_pointer_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());
    std::fs::write(feat.join("user.txt"), "keep me\n").unwrap();
    std::fs::write(feat.join(".bole"), "}{ not json").unwrap();

    let out = json(&primary, &["workspace", "prune"]);
    assert_eq!(out[0]["status"], "bad-pointer");
    assert_eq!(linked_count(&primary), 0);
    // The user's other files are untouched; only the pointer file is removed.
    assert!(feat.join("user.txt").exists());
    assert!(!feat.join(".bole").exists());
}

#[test]
fn prune_dry_run_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());
    std::fs::remove_dir_all(&feat).unwrap();

    let out = json(&primary, &["workspace", "prune", "--dry-run"]);
    assert_eq!(out[0]["pruned"], false);
    assert_eq!(linked_count(&primary), 1, "registry unchanged in dry-run");
}

#[test]
fn prune_clean_repo_prunes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, _feat) = setup_with_linked(tmp.path());
    let out = json(&primary, &["workspace", "prune"]);
    assert!(out.as_array().unwrap().is_empty());
    assert_eq!(linked_count(&primary), 1);
}

#[test]
fn list_annotates_and_check_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());

    // Clean: list --check exits 0.
    let clean = run(&primary, &["workspace", "list", "--check"]);
    assert!(clean.status.success());

    std::fs::remove_dir_all(&feat).unwrap();

    // Human output annotates the stale entry.
    let human = String::from_utf8(ok(&primary, &["workspace", "list"]).stdout).unwrap();
    assert!(human.contains("[STALE: missing-directory]"), "got: {human}");

    // JSON carries the status field.
    let arr = json(&primary, &["workspace", "list"]);
    let linked = arr.as_array().unwrap().iter().find(|e| e["linked"] == true).unwrap();
    assert_eq!(linked["status"], "missing-directory");

    // list --check exits 1 when stale.
    let stale = run(&primary, &["workspace", "list", "--check"]);
    assert_eq!(stale.status.code(), Some(1));
}

#[test]
fn repair_directory_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());
    let newloc = tmp.path().join("moved-feat");
    std::fs::rename(&feat, &newloc).unwrap();

    // The old entry is now stale (missing directory).
    let arr = json(&primary, &["workspace", "list"]);
    let linked = arr.as_array().unwrap().iter().find(|e| e["linked"] == true).unwrap();
    assert_eq!(linked["status"], "missing-directory");

    // repair --moved-to updates the registered path.
    let res = json(&primary, &["workspace", "repair", "--moved-to", newloc.to_str().unwrap(), "feat"]);
    assert_eq!(res["repaired"], true);

    // Now clean again, pointing at the new location.
    let arr2 = json(&primary, &["workspace", "list"]);
    let linked2 = arr2.as_array().unwrap().iter().find(|e| e["linked"] == true).unwrap();
    assert_eq!(linked2["status"], "ok");
    assert!(linked2["path"].as_str().unwrap().contains("moved-feat"));
}

#[test]
fn repair_adopt_orphaned_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, feat) = setup_with_linked(tmp.path());

    // Manually drop the registry entry (simulate a lost registry).
    let reg_path = primary.join(".bole/worktrees.json");
    let mut reg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    reg["worktrees"].as_object_mut().unwrap().clear();
    std::fs::write(&reg_path, serde_json::to_vec_pretty(&reg).unwrap()).unwrap();
    assert_eq!(linked_count(&primary), 0);

    // Adopt the surviving pointer directory.
    let res = json(&primary, &["workspace", "repair", "--adopt", feat.to_str().unwrap()]);
    assert_eq!(res["adopted"], true);
    assert_eq!(res["id"], "feat");
    assert_eq!(linked_count(&primary), 1, "re-registered");

    // Adopting again is a no-op ("already consistent").
    let again = json(&primary, &["workspace", "repair", "--adopt", feat.to_str().unwrap()]);
    assert_eq!(again["adopted"], false);
}
