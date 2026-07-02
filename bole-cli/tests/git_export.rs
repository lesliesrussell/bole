// bole-1ff
//! End-to-end integration test for `bole git export`'s ACL filtering, driving the
//! real binary and inspecting the exported bare repo with `git`.
//!
//! Regression guard for two things that must hold together:
//!   - timelines the bound actor may read become branches (an unprotected
//!     timeline is public — bole-x8w), and
//!   - protected paths are excluded from the projected trees per the bound actor,
//!     while public paths are always included.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn ok(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = bin().args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "command {args:?} failed: {out:?}");
    out
}

fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = args.to_vec();
    full.push("--json");
    let out = ok(dir, &full);
    serde_json::from_slice(&out.stdout).unwrap()
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `git for-each-ref refs/heads` -> short branch names in the exported bare repo.
fn branches(git_dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .args([
            "--git-dir",
            git_dir.to_str().unwrap(),
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "git for-each-ref failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

/// `git ls-tree -r --name-only <branch>` -> file paths in that branch's tip tree.
fn tree_files(git_dir: &Path, branch: &str) -> Vec<String> {
    let out = Command::new("git")
        .args([
            "--git-dir",
            git_dir.to_str().unwrap(),
            "ls-tree",
            "-r",
            "--name-only",
            branch,
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "git ls-tree failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

fn commit_count(git_dir: &Path, branch: &str) -> usize {
    let out = Command::new("git")
        .args(["--git-dir", git_dir.to_str().unwrap(), "rev-list", "--count", branch])
        .output()
        .unwrap();
    assert!(out.status.success(), "git rev-list failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn git_export_filters_protected_paths_per_actor() {
    if !git_available() {
        eprintln!("skipping git_export_filters_protected_paths_per_actor: git not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."]);

    // Public files + a protected secret path.
    std::fs::write(w.join("bole-landing.html"), "<html>hi</html>").unwrap();
    std::fs::create_dir_all(w.join("src")).unwrap();
    std::fs::write(w.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::create_dir_all(w.join("secrets")).unwrap();
    std::fs::write(w.join("secrets/prod.key"), "TOPSECRET").unwrap();
    ok(w, &["acl", "path", "protect", "secrets/**"]);

    // Initial snapshot, bind main, then a second snapshot for real history.
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "initial"]);
    let snap_id = snap["snapshot"].as_str().unwrap().to_string();
    ok(w, &["workspace", "open", "main", "--create", "--from", &snap_id]);
    std::fs::write(w.join("src/main.rs"), "fn main() { /* v2 */ }").unwrap();
    ok(w, &["snapshot", "create", "--from-workspace", "-m", "edit"]);

    // A second public timeline off the current head.
    ok(w, &["timeline", "create", "feature/x", "--from", "@"]);

    // Actor cleared for a public path but NOT for secrets/**.
    ok(w, &["actor", "create", "dev"]);
    ok(w, &["actor", "grant-path", "dev", "src/**", "read"]);
    ok(w, &["actor", "use", "dev"]);

    // Export as dev.
    let dev_git = w.join("dev.git");
    ok(w, &["git", "export", "--to", dev_git.to_str().unwrap()]);

    // Both public timelines are visible to the bound actor (bole-x8w).
    let br = branches(&dev_git);
    assert!(br.iter().any(|b| b == "main"), "main branch missing: {br:?}");
    assert!(br.iter().any(|b| b == "feature/x"), "feature/x branch missing: {br:?}");
    // Real history, not an empty ref.
    assert_eq!(commit_count(&dev_git, "main"), 2, "expected 2 commits on main");

    // Public paths present; protected secrets/ absent for dev.
    let files = tree_files(&dev_git, "main");
    assert!(files.iter().any(|f| f == "src/main.rs"), "src/main.rs missing: {files:?}");
    assert!(files.iter().any(|f| f == "bole-landing.html"), "landing missing: {files:?}");
    assert!(
        !files.iter().any(|f| f.starts_with("secrets/")),
        "protected secrets/ leaked into the dev export: {files:?}"
    );

    // Contrast: an actor cleared for everything DOES see the protected path,
    // proving the filtering is per-actor, not a broken projection.
    ok(w, &["actor", "create", "admin"]);
    ok(w, &["actor", "grant-path", "admin", "**", "read"]);
    ok(w, &["actor", "use", "admin"]);
    let admin_git = w.join("admin.git");
    ok(w, &["git", "export", "--to", admin_git.to_str().unwrap()]);
    let admin_files = tree_files(&admin_git, "main");
    assert!(
        admin_files.iter().any(|f| f == "secrets/prod.key"),
        "admin (cleared for **) should see the protected path: {admin_files:?}"
    );
}
