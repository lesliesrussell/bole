// bole-6i1
//! End-to-end integration tests for the collab CLI authoring workflow:
//! `bole profile set/show` and `bole trust follow/list`.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn run(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let mut c = bin();
    c.args(args).current_dir(dir);
    if let Some(s) = seed {
        c.env("BOLE_COLLAB_KEY", s);
    }
    c.output().unwrap()
}

fn ok(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let out = run(dir, args, seed);
    assert!(out.status.success(), "cmd {args:?} failed: {out:?}");
    out
}

#[test]
fn cli_profile_set_and_show() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "aa".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Alice", "--bio", "hi"], Some(&seed));
    let show = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(v["display_name"], "Alice");
    // Re-setting bumps seq monotonically.
    ok(w, &["profile", "set", "--display-name", "Alice2"], Some(&seed));
    let show2 = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v2: serde_json::Value = serde_json::from_slice(&show2.stdout).unwrap();
    assert_eq!(v2["display_name"], "Alice2");
    assert!(v2["seq"].as_u64().unwrap() > v["seq"].as_u64().unwrap());
}

#[test]
fn cli_trust_follow_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "bb".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Me"], Some(&seed));
    let peer = "cc".repeat(32); // a 64-hex key
    ok(w, &["trust", "follow", &peer], Some(&seed));
    let list = ok(w, &["trust", "list", "--json"], Some(&seed));
    assert!(String::from_utf8_lossy(&list.stdout).contains(&peer[..8]));
}

// bole-6i1
#[test]
fn cli_trust_vouch_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "ab".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Me"], Some(&seed));
    let peer = "cd".repeat(32);
    ok(w, &["trust", "vouch", &peer, "--name", "buddy"], Some(&seed));
    let list = ok(w, &["trust", "list", "--json"], Some(&seed));
    let s = String::from_utf8_lossy(&list.stdout);
    assert!(s.contains(&peer[..8]), "vouched key present: {s}");
    assert!(s.contains("buddy"), "petname present: {s}");
}
