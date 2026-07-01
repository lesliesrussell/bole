// bole-58u
//! Integration test for `bole git import` (round-trip through `git export`).

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
fn git_export_then_import_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let git = tmp.path().join("mirror.git");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();

    // A bole repo with one snapshot on `main`.
    std::fs::write(src.join("a.txt"), "hello\n").unwrap();
    ok(&src, &["init", "."]);
    let snap = json(&src, &["snapshot", "create", "--from-workspace", "-m", "init"]);
    let id = snap["snapshot"].as_str().unwrap().to_string();
    ok(&src, &["workspace", "open", "main", "--create", "--from", &id]);

    // Export to a bare git repo.
    ok(&src, &["git", "export", "--to", git.to_str().unwrap()]);

    // Import into a fresh bole repo.
    ok(&dst, &["init", "."]);
    let summary = json(&dst, &["git", "import", git.to_str().unwrap()]);
    assert_eq!(summary["timelines_created"], 1);
    assert!(summary["snapshots"].as_u64().unwrap() >= 1);
    assert_eq!(summary["skipped"], 0);

    // The imported `main` timeline exists.
    let tl = json(&dst, &["timeline", "list"]);
    assert!(
        tl.as_array().unwrap().iter().any(|t| t["name"] == "main" || t == "main"),
        "imported timelines: {tl}"
    );
}
