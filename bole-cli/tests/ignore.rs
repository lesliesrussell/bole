// bole-phxz
//! Integration tests for the `bole ignore` command group.

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

fn setup(w: &Path) {
    ok(w, &["init", "."]);
}

#[test]
fn add_bare_then_list_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    // Bare form is sugar for `add`.
    ok(w, &["ignore", "*.log", "target/"]);
    let listed = json(w, &["ignore", "list"]);
    let patterns: Vec<String> = listed["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(patterns, vec!["*.log".to_string(), "target/".to_string()]);

    // The file lives at the work-tree root.
    let body = std::fs::read_to_string(w.join(".boleignore")).unwrap();
    assert_eq!(body, "*.log\ntarget/\n");
}

#[test]
fn add_dedups_existing_patterns() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    ok(w, &["ignore", "*.log"]);
    let res = json(w, &["ignore", "add", "*.log", "*.tmp"]);
    assert_eq!(res["added"].as_array().unwrap().len(), 1);
    assert_eq!(res["skipped"].as_array().unwrap().len(), 1);

    let body = std::fs::read_to_string(w.join(".boleignore")).unwrap();
    assert_eq!(body, "*.log\n*.tmp\n");
}

#[test]
fn remove_deletes_line() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    ok(w, &["ignore", "*.log", "target/", "*.tmp"]);
    let res = json(w, &["ignore", "remove", "target/", "nope"]);
    assert_eq!(res["removed"], serde_json::json!(["target/"]));
    assert_eq!(res["not_found"], serde_json::json!(["nope"]));

    let listed = json(w, &["ignore", "list"]);
    let patterns: Vec<String> = listed["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(patterns, vec!["*.log".to_string(), "*.tmp".to_string()]);
}

#[test]
fn check_reports_ignored_and_matching_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    ok(w, &["ignore", "*.log", "target/", "!keep.log"]);

    let res = json(w, &["ignore", "check", "app.log", "keep.log", "src/main.rs"]);
    let results = res["results"].as_array().unwrap();

    let by_path = |p: &str| results.iter().find(|r| r["path"] == p).unwrap().clone();
    assert_eq!(by_path("app.log")["ignored"], serde_json::json!(true));
    assert_eq!(by_path("app.log")["pattern"], serde_json::json!("*.log"));
    // Negation wins for keep.log.
    assert_eq!(by_path("keep.log")["ignored"], serde_json::json!(false));
    assert_eq!(by_path("src/main.rs")["ignored"], serde_json::json!(false));
}

// bole-0tou
#[test]
fn check_models_parent_dir_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);
    std::fs::create_dir_all(w.join("target/sub")).unwrap();
    std::fs::write(w.join("target/out.bin"), b"x").unwrap();
    std::fs::write(w.join("target/sub/deep.bin"), b"x").unwrap();
    ok(w, &["ignore", "target/"]);

    let res = json(
        w,
        &[
            "ignore", "check", "target", "target/", "target/out.bin", "target/sub/deep.bin",
            "src/main.rs",
        ],
    );
    let results = res["results"].as_array().unwrap();
    let by = |p: &str| results.iter().find(|r| r["path"] == p).unwrap().clone();

    // The dir itself, a trailing-slash query, and everything beneath it are all
    // ignored — the walk prunes `target/` whole, so `check` must agree.
    assert_eq!(by("target")["ignored"], serde_json::json!(true));
    assert_eq!(by("target/")["ignored"], serde_json::json!(true));
    assert_eq!(by("target/out.bin")["ignored"], serde_json::json!(true));
    assert_eq!(by("target/sub/deep.bin")["ignored"], serde_json::json!(true));
    assert_eq!(by("src/main.rs")["ignored"], serde_json::json!(false));
}

#[test]
fn check_directory_pattern_matches_dir() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);
    std::fs::create_dir(w.join("target")).unwrap();

    ok(w, &["ignore", "target/"]);
    let res = json(w, &["ignore", "check", "target"]);
    assert_eq!(res["results"][0]["ignored"], serde_json::json!(true));
}

#[test]
fn malformed_pattern_is_rejected_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    // An unclosed alternate group `{` is an invalid glob.
    let out = run(w, &["ignore", "add", "a{b"]);
    assert!(!out.status.success(), "expected rejection, got {out:?}");
    // File must not have been created.
    assert!(!w.join(".boleignore").exists());
}

#[test]
fn list_empty_when_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    let res = json(w, &["ignore", "list"]);
    assert_eq!(res["patterns"], serde_json::json!([]));
}
