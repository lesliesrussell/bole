// bole-ehx
//! End-to-end integration test for the signed-approval CLI workflow: configure a
//! policy, register approvers, sign attestations, and confirm a gated timeline
//! advance is denied until enough distinct signed approvals of the exact head
//! exist.

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

/// Signs an attestation as `key_id` with `seed` (seed passed via env, not argv).
fn approve(dir: &Path, timeline: &str, snapshot: &str, key_id: &str, seed: &str) {
    let out = bin()
        .args(["approve", timeline, snapshot, "--key-id", key_id])
        .env("BOLE_APPROVER_KEY", seed)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "approve as {key_id} failed: {out:?}");
}

#[test]
fn signed_approval_workflow_gates_timeline_advance() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."]);

    // Initial snapshot + a release timeline off it.
    std::fs::write(w.join("app.rs"), "v0").unwrap();
    let s0 = json(w, &["snapshot", "create", "--from-workspace", "-m", "init", "--no-advance"]);
    let s0 = s0["snapshot"].as_str().unwrap().to_string();
    ok(w, &["timeline", "create", "release/1.0", "--from", &s0]);

    // Build the next snapshot without advancing (so we can approve its exact id).
    std::fs::write(w.join("app.rs"), "v1").unwrap();
    let s1 = json(w, &["snapshot", "create", "--from-workspace", "-m", "v1", "--no-advance"]);
    let s1 = s1["snapshot"].as_str().unwrap().to_string();

    // Require 2 distinct signed approvals to advance into release/**.
    ok(w, &["policy", "require-approval", "release/**", "--needed", "2"]);
    assert!(json(w, &["policy", "list"]).to_string().contains("release/**"));

    // Advance with no approvals -> denied, head unchanged.
    let denied = run(w, &["timeline", "advance", "release/1.0", "--to", &s1]);
    assert!(!denied.status.success(), "advance must be denied without approvals: {denied:?}");
    assert_eq!(json(w, &["timeline", "show", "release/1.0"])["head"].as_str().unwrap(), s0);

    // Register two approvers by deriving their public keys from seeds.
    let alice_seed = "11".repeat(32);
    let bob_seed = "22".repeat(32);
    ok(w, &["approver", "add", "alice", "--seed", &alice_seed]);
    ok(w, &["approver", "add", "bob", "--seed", &bob_seed]);
    assert!(json(w, &["approver", "list"]).to_string().contains("alice"));

    // One approval -> still short of the 2 required.
    approve(w, "release/1.0", &s1, "alice", &alice_seed);
    let one = run(w, &["timeline", "advance", "release/1.0", "--to", &s1]);
    assert!(!one.status.success(), "one approval must be < needed 2");

    // A second distinct approval of the exact head -> advance allowed.
    approve(w, "release/1.0", &s1, "bob", &bob_seed);
    ok(w, &["timeline", "advance", "release/1.0", "--to", &s1]);
    assert_eq!(json(w, &["timeline", "show", "release/1.0"])["head"].as_str().unwrap(), s1);
}

#[test]
fn approve_rejects_unregistered_key() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("f"), "x").unwrap();
    let s0 = json(w, &["snapshot", "create", "--from-workspace", "-m", "i", "--no-advance"]);
    let s0 = s0["snapshot"].as_str().unwrap().to_string();
    ok(w, &["timeline", "create", "release/1.0", "--from", &s0]);

    // No approver registered under "mallory" -> `approve` refuses to sign.
    let out = bin()
        .args(["approve", "release/1.0", &s0, "--key-id", "mallory"])
        .env("BOLE_APPROVER_KEY", "99".repeat(32))
        .current_dir(w)
        .output()
        .unwrap();
    assert!(!out.status.success(), "approve as an unregistered key must fail");
}
