// bole-ef8
//! Integration tests for the actor registry and acl command groups.

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

/// Sets up a repo bound to `main` with one snapshot.
fn setup(w: &Path) {
    std::fs::create_dir_all(w.join("src")).unwrap();
    std::fs::write(w.join("src/main.rs"), "fn main(){}\n").unwrap();
    ok(w, &["init", "."]);
    let snap = json(w, &["snapshot", "create", "--from-workspace", "-m", "init"]);
    let first = snap["snapshot"].as_str().unwrap().to_string();
    ok(w, &["workspace", "open", "main", "--create", "--from", &first]);
}

#[test]
fn actor_grants_and_can_checks() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    ok(w, &["actor", "create", "bot"]);
    ok(w, &["actor", "grant-path", "bot", "src/**", "write"]);
    ok(w, &["actor", "grant-timeline", "bot", "agent/**", "write"]);

    let show = json(w, &["actor", "show", "bot"]);
    assert_eq!(show["path_roles"][0]["glob"], "src/**");
    assert_eq!(show["timeline_roles"][0]["pattern"], "agent/**");

    // bot can write src paths and agent timelines, but not main or docs.
    assert_eq!(json(w, &["acl", "can-write-path", "--actor", "bot", "src/lib.rs"])["allowed"], true);
    assert_eq!(json(w, &["acl", "can-write-path", "--actor", "bot", "docs/x.md"])["allowed"], false);
    assert_eq!(json(w, &["acl", "can-write-timeline", "--actor", "bot", "agent/x"])["allowed"], true);
    assert_eq!(json(w, &["acl", "can-write-timeline", "--actor", "bot", "main"])["allowed"], false);
}

#[test]
fn bound_actor_is_enforced_on_advance() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    // bot may not write the main timeline.
    ok(w, &["actor", "create", "bot"]);
    ok(w, &["actor", "grant-path", "bot", "**", "write"]);
    ok(w, &["actor", "grant-timeline", "bot", "agent/**", "write"]);

    // alice may write everything.
    ok(w, &["actor", "create", "alice"]);
    ok(w, &["actor", "grant-path", "alice", "**", "write"]);
    ok(w, &["actor", "grant-timeline", "alice", "**", "write"]);

    // Acting as bot, advancing main (via snapshot create) is denied.
    ok(w, &["actor", "use", "bot"]);
    std::fs::write(w.join("src/main.rs"), "fn main(){ }\n").unwrap();
    let denied = run(w, &["snapshot", "create", "--from-workspace", "-m", "bot-edit"]);
    assert!(!denied.status.success(), "bot should be denied on main");
    assert!(String::from_utf8_lossy(&denied.stderr).contains("denied"));

    // Acting as alice, the same advance succeeds.
    ok(w, &["actor", "use", "alice"]);
    let allowed = run(w, &["snapshot", "create", "--from-workspace", "-m", "alice-edit"]);
    assert!(allowed.status.success(), "alice should succeed: {allowed:?}");
}

#[test]
fn acl_path_protect_list_unprotect() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    setup(w);

    ok(w, &["acl", "path", "protect", "secrets/**"]);
    let list = json(w, &["acl", "path", "list"]);
    assert_eq!(list, serde_json::json!(["secrets/**"]));

    ok(w, &["acl", "path", "unprotect", "secrets/**"]);
    let list2 = json(w, &["acl", "path", "list"]);
    assert_eq!(list2, serde_json::json!([]));
}
