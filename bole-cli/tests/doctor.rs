// bole-wphx
//! `bole doctor` — health-check reporting: seed leaks, worktree hygiene,
//! store integrity, and a CI-gating exit code.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

const SEED: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

// A clean repo is all-clear and exits 0.
#[test]
fn doctor_clean_repo_all_clear() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("README.md"), "# hi").unwrap();
    let out = run(dir.path(), &["doctor", "--json"]);
    assert!(out.status.success(), "clean repo should exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"], 0);
    assert_eq!(v["warnings"], 0);
}

// A seed sitting in the working tree (unignored) is a WARN, exit still 0 —
// but --strict turns it into a failure.
#[test]
fn doctor_flags_worktree_seed_as_warning() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("id.key"), format!("{SEED}\n")).unwrap();

    let out = run(dir.path(), &["doctor", "--json"]);
    assert!(out.status.success(), "warning alone should not fail exit code");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["warnings"], 1, "worktree seed is a warning: {v}");
    assert_eq!(v["errors"], 0);
    let has = v["checks"].as_array().unwrap().iter().any(|c|
        c["check"] == "worktree-seed" && c["severity"] == "warn" && c["message"].as_str().unwrap().contains("id.key"));
    assert!(has, "worktree-seed warning names the file: {v}");

    // --strict promotes the warning to a failing exit.
    let strict = run(dir.path(), &["doctor", "--strict"]);
    assert!(!strict.status.success(), "--strict must fail on a warning");
}

// Once the seed file is ignored, doctor is quiet about it.
#[test]
fn doctor_quiet_when_seed_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("id.key"), format!("{SEED}\n")).unwrap();
    assert!(run(dir.path(), &["ignore", "add", "id.key"]).status.success());

    let out = run(dir.path(), &["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["warnings"], 0, "ignored seed is not warned about: {v}");
    assert_eq!(v["errors"], 0);
}

// A seed committed into a timeline is an ERROR and exits non-zero.
#[test]
fn doctor_committed_seed_is_error_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("leaked.key"), format!("{SEED}\n")).unwrap();
    // Commit it to a `main` timeline.
    let snap = run(dir.path(), &["snapshot", "create", "--from-workspace", "--no-advance", "-m", "oops", "--json"]);
    let sv: serde_json::Value = serde_json::from_slice(&snap.stdout).unwrap();
    let head = sv["snapshot"].as_str().unwrap();
    assert!(run(dir.path(), &["timeline", "create", "main", "--from", head]).status.success());

    let out = run(dir.path(), &["doctor", "--json"]);
    assert!(!out.status.success(), "committed seed must fail exit code");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["errors"].as_i64().unwrap() >= 1, "committed seed is an error: {v}");
    let has = v["checks"].as_array().unwrap().iter().any(|c|
        c["check"] == "committed-seed" && c["severity"] == "error");
    assert!(has, "committed-seed error present: {v}");
}
