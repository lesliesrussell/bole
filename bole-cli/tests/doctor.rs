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

// A clean repo whose .boleignore covers secrets is all-clear and exits 0.
#[test]
fn doctor_clean_repo_all_clear() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("README.md"), "# hi").unwrap();
    // Cover the secret globs so the boleignore check is happy.
    assert!(run(dir.path(), &["ignore", "add", "*.key", "*.pem", "*.seed", "id_rsa", ".env"]).status.success());
    let out = run(dir.path(), &["doctor", "--json"]);
    assert!(out.status.success(), "clean repo should exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["errors"], 0);
    assert_eq!(v["warnings"], 0, "expected no warnings: {v}");
}

// A repo with no .boleignore warns about it (but doesn't fail exit).
#[test]
fn doctor_warns_when_boleignore_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("README.md"), "# hi").unwrap();
    let out = run(dir.path(), &["doctor", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let has = v["checks"].as_array().unwrap().iter().any(|c|
        c["check"] == "boleignore" && c["severity"] == "warn");
    assert!(has, "missing .boleignore is warned: {v}");
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
    // Cover the seed file AND the secret globs so doctor is fully quiet.
    assert!(run(dir.path(), &["ignore", "add", "id.key", "*.key", "*.pem", "*.seed", "id_rsa", ".env"]).status.success());

    let out = run(dir.path(), &["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let seed_warn = v["checks"].as_array().unwrap().iter().any(|c| c["check"] == "worktree-seed" && c["severity"] == "warn");
    assert!(!seed_warn, "ignored seed is not warned about: {v}");
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

fn check<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"].as_array().unwrap().iter().find(|c| c["check"] == name).expect("check present")
}

// A fresh repo reports the new checks as clean (ok) and stays exit 0.
#[test]
fn doctor_new_checks_clean_on_fresh_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    let home = tempfile::tempdir().unwrap();
    let out = bin().args(["doctor", "--json"]).current_dir(dir.path()).env("HOME", home.path()).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for c in ["orphan-repo", "policy-pin", "bound-state", "key-perms", "gc-opportunity"] {
        assert_eq!(check(&v, c)["severity"], "ok", "{c} should be ok on a fresh repo: {v}");
    }
}

// A seed file with loose perms in ~/.bole/keys is a key-perms warning.
#[cfg(unix)]
#[test]
fn doctor_flags_world_readable_keyring_seed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    let home = tempfile::tempdir().unwrap();
    let keys = home.path().join(".bole").join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let kf = keys.join("acct.key");
    std::fs::write(&kf, "a1".repeat(32)).unwrap();
    std::fs::set_permissions(&kf, std::fs::Permissions::from_mode(0o644)).unwrap();

    let out = bin().args(["doctor", "--json"]).current_dir(dir.path()).env("HOME", home.path()).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let has = v["checks"].as_array().unwrap().iter().any(|c|
        c["check"] == "key-perms" && c["severity"] == "warn" && c["message"].as_str().unwrap().contains("acct.key"));
    assert!(has, "loose keyring seed is warned: {v}");
}

// gc-opportunity reports reclaimable objects after orphaning one.
#[test]
fn doctor_reports_gc_opportunity() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    let home = tempfile::tempdir().unwrap();
    // Store a blob that no ref points at → reclaimable garbage.
    std::fs::write(dir.path().join("junk.txt"), "loose garbage").unwrap();
    assert!(run(dir.path(), &["object", "put-blob", "junk.txt"]).status.success());

    let out = bin().args(["doctor", "--json"]).current_dir(dir.path()).env("HOME", home.path()).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let msg = check(&v, "gc-opportunity")["message"].as_str().unwrap();
    assert!(msg.contains("reclaimable"), "gc-opportunity reports reclaimable count: {msg}");
}
