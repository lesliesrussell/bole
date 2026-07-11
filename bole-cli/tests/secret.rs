// bole-1q9
//! Integration tests for the secret and env command groups.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KEY2: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bole"));
    c.env("BOLE_KEY", KEY);
    c
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

// bole-amy
/// Writes `value` to a new secret `name` via stdin, using the default granter key.
fn put_stdin(dir: &Path, name: &str, value: &[u8]) {
    let mut child = bin()
        .args(["secret", "put", name, "--from-stdin"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(value).unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
}

// bole-amy
/// Runs `bole` with a specific `BOLE_KEY`, returning the raw output.
fn run_as(dir: &Path, key: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bole"))
        .env("BOLE_KEY", key)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

// bole-amy
#[test]
fn secret_grant_and_revoke_actor() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);

    put_stdin(w, "prod/db", b"postgres://secret");

    // The granter (KEY) can read; Bob (KEY2) cannot yet.
    assert!(run_as(w, KEY, &["secret", "reveal", "prod/db"]).status.success());
    assert!(!run_as(w, KEY2, &["secret", "reveal", "prod/db"]).status.success());

    // Grant Bob: granter key from BOLE_KEY, recipient key from BOLE_RECIPIENT_KEY.
    let grant = bin()
        .args(["secret", "grant-actor", "prod/db", "--recipient-key-env", "BOLE_RECIPIENT_KEY"])
        .env("BOLE_RECIPIENT_KEY", KEY2)
        .current_dir(w)
        .output()
        .unwrap();
    assert!(grant.status.success(), "grant failed: {grant:?}");

    // Now Bob can read, and the granter still can.
    let bob = run_as(w, KEY2, &["secret", "reveal", "prod/db", "--json"]);
    assert!(bob.status.success(), "bob reveal failed: {bob:?}");
    let v: serde_json::Value = serde_json::from_slice(&bob.stdout).unwrap();
    assert_eq!(v["value"], "postgres://secret");
    assert!(run_as(w, KEY, &["secret", "reveal", "prod/db"]).status.success());

    // Revoke Bob: he can no longer read; the granter still can.
    let revoke = bin()
        .args(["secret", "revoke-actor", "prod/db", "--recipient-key-env", "BOLE_RECIPIENT_KEY"])
        .env("BOLE_RECIPIENT_KEY", KEY2)
        .current_dir(w)
        .output()
        .unwrap();
    assert!(revoke.status.success(), "revoke failed: {revoke:?}");
    assert!(!run_as(w, KEY2, &["secret", "reveal", "prod/db"]).status.success());
    assert!(run_as(w, KEY, &["secret", "reveal", "prod/db"]).status.success());
}

#[test]
fn secret_put_reveal_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);

    // put from stdin
    let mut child = bin()
        .args(["secret", "put", "prod/db", "--from-stdin"])
        .current_dir(w)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"postgres://secret").unwrap();
    assert!(child.wait_with_output().unwrap().status.success());

    // reveal returns the plaintext
    let rev = json(w, &["secret", "reveal", "prod/db"]);
    assert_eq!(rev["value"], "postgres://secret");

    // list contains the name
    let list = json(w, &["secret", "list"]);
    assert_eq!(list, serde_json::json!(["prod/db"]));
}

#[test]
fn secret_reveal_wrong_key_fails() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("s.txt"), b"value").unwrap();
    ok(w, &["secret", "put", "k", "--from-file", "s.txt"]);

    let out = bin()
        .env("BOLE_KEY", KEY2)
        .args(["secret", "reveal", "k"])
        .current_dir(w)
        .output()
        .unwrap();
    assert!(!out.status.success(), "wrong key should fail to decrypt");
}

#[test]
fn env_overlay_with_plain_and_secret() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("s.txt"), b"postgres://secret").unwrap();
    ok(w, &["secret", "put", "prod/db", "--from-file", "s.txt"]);

    ok(w, &["env", "create", "dev"]);
    ok(w, &["env", "set", "dev", "RUST_LOG", "debug"]);
    ok(w, &["env", "set-secret", "dev", "DATABASE_URL", "prod/db"]);

    let show = json(w, &["env", "show", "dev"]);
    let entries = show["entries"].as_array().unwrap();
    let log = entries.iter().find(|e| e["var"] == "RUST_LOG").unwrap();
    assert_eq!(log["kind"], "plain");
    assert_eq!(log["value"], "debug");
    let db = entries.iter().find(|e| e["var"] == "DATABASE_URL").unwrap();
    assert_eq!(db["kind"], "secret");

    // Human output redacts the secret value.
    let human = String::from_utf8(ok(w, &["env", "show", "dev"]).stdout).unwrap();
    assert!(human.contains("RUST_LOG=debug"));
    assert!(human.contains("DATABASE_URL=<secret>"));
    assert!(!human.contains("postgres://secret"));

    let list = json(w, &["env", "list"]);
    assert_eq!(list, serde_json::json!(["dev"]));
}

// bole-9mz
#[test]
fn env_resolve_and_run() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("s.txt"), b"postgres://secret").unwrap();
    ok(w, &["secret", "put", "prod/db", "--from-file", "s.txt"]);
    ok(w, &["env", "create", "dev"]);
    ok(w, &["env", "set", "dev", "RUST_LOG", "debug"]);
    ok(w, &["env", "set-secret", "dev", "DATABASE_URL", "prod/db"]);

    // Default resolve redacts the secret value.
    let human = String::from_utf8(ok(w, &["env", "resolve", "dev"]).stdout).unwrap();
    assert!(human.contains("RUST_LOG=debug"));
    assert!(human.contains("DATABASE_URL=<redacted>"));
    assert!(!human.contains("postgres://secret"));

    // --reveal decrypts (no secret ACL → public → cleared).
    let revealed = json(w, &["env", "resolve", "dev", "--reveal"]);
    assert_eq!(revealed["env"]["DATABASE_URL"], "postgres://secret");
    assert_eq!(revealed["env"]["RUST_LOG"], "debug");

    // `bole run` injects the vars into the child process.
    let out = run(w, &["run", "--env", "dev", "--", "sh", "-c", "printf %s \"$DATABASE_URL\""]);
    assert!(out.status.success(), "run failed: {out:?}");
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "postgres://secret");
}

// bole-9mz
#[test]
fn secret_rekey_rotates_master_key() {
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);
    std::fs::write(w.join("s.txt"), b"the-value").unwrap();
    ok(w, &["secret", "put", "k", "--from-file", "s.txt"]);

    // Rotate the master key from KEY to KEY2.
    let out = bin()
        .env("BOLE_KEY", KEY)
        .env("BOLE_NEW_KEY", KEY2)
        .args(["secret", "rekey", "--all"])
        .current_dir(w)
        .output()
        .unwrap();
    assert!(out.status.success(), "rekey failed: {out:?}");

    // The new key decrypts; the old key no longer does.
    let rev = bin()
        .env("BOLE_KEY", KEY2)
        .args(["secret", "reveal", "k", "--json"])
        .current_dir(w)
        .output()
        .unwrap();
    assert!(rev.status.success());
    let v: serde_json::Value = serde_json::from_slice(&rev.stdout).unwrap();
    assert_eq!(v["value"], "the-value");

    let old = bin()
        .env("BOLE_KEY", KEY)
        .args(["secret", "reveal", "k"])
        .current_dir(w)
        .output()
        .unwrap();
    assert!(!old.status.success(), "old key should no longer decrypt");
}

// bole-oea4
/// `secret share` creates a multi-recipient secret for several recipients at
/// once (each identified by a recipient key file), plus the sharer. Every
/// recipient — and the sharer — can reveal it; an unrelated key cannot.
#[test]
fn secret_share_multi_recipient() {
    const KEY3: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY4: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let dir = tempfile::tempdir().unwrap();
    let w = dir.path();
    ok(w, &["init", "."]);

    // Write two recipient key files (KEY2, KEY3); the sharer is BOLE_KEY (KEY).
    let f2 = w.join("r2.key");
    let f3 = w.join("r3.key");
    std::fs::write(&f2, KEY2).unwrap();
    std::fs::write(&f3, KEY3).unwrap();

    let mut child = bin()
        .args([
            "secret", "share", "team/token", "--from-stdin",
            "--recipient-key-file", f2.to_str().unwrap(),
            "--recipient-key-file", f3.to_str().unwrap(),
        ])
        .current_dir(w)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"shared-value").unwrap();
    assert!(child.wait_with_output().unwrap().status.success(), "share failed");

    // The sharer and both recipients can reveal; an unrelated key cannot.
    for key in [KEY, KEY2, KEY3] {
        let out = run_as(w, key, &["secret", "reveal", "team/token", "--json"]);
        assert!(out.status.success(), "reveal as {key} failed: {out:?}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["value"], "shared-value");
    }
    assert!(!run_as(w, KEY4, &["secret", "reveal", "team/token"]).status.success(),
        "an unrelated key must not reveal the shared secret");
}
