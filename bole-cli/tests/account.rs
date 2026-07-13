// bole-ohi0
//! Account-seed hygiene: `bole account create` keeps seeds out of the repo, and
//! `bole snapshot create` warns if it captures a file that looks like a seed.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    bin().args(args).current_dir(dir).output().unwrap()
}

// `account create --out` writes a 0600 key file and prints the account id.
#[test]
fn account_create_writes_seed_and_reports_id() {
    let dir = tempfile::tempdir().unwrap();
    let keyfile = dir.path().join("acc.key");
    let out = run(dir.path(), &["account", "create", "--out", keyfile.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "account create failed: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["account"].as_str().unwrap().len(), 64);
    // The seed file exists and holds a 64-hex line.
    let seed = std::fs::read_to_string(&keyfile).unwrap();
    assert_eq!(seed.trim().len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&keyfile).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "seed file must be owner-only");
    }
}

// `account create` with no --out defaults OUTSIDE the working tree (into
// $HOME/.bole/keys) so a snapshot can't scoop up the seed.
#[test]
fn account_create_defaults_outside_repo() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    assert!(run(repo.path(), &["init", "."]).status.success());
    let out = bin()
        .args(["account", "create", "--json"])
        .current_dir(repo.path())
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "account create failed: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let key_file = v["key_file"].as_str().unwrap();
    // The seed landed under the home keyring, NOT inside the repo working tree.
    assert!(key_file.starts_with(home.path().to_str().unwrap()), "seed not in keyring: {key_file}");
    assert!(!key_file.starts_with(repo.path().to_str().unwrap()), "seed leaked into repo tree: {key_file}");
    assert!(std::path::Path::new(key_file).exists());
}

// `snapshot create` warns (on stderr) when it captures a bare-seed-looking file.
#[test]
fn snapshot_create_warns_on_seed_like_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    // A stray key file in the working tree (the footgun).
    let seed = "a1".repeat(32); // 64 hex chars
    std::fs::write(dir.path().join("leaked.key"), format!("{seed}\n")).unwrap();
    std::fs::write(dir.path().join("README.md"), "# hi").unwrap();

    let out = run(dir.path(), &["snapshot", "create", "--from-workspace", "--no-advance", "-m", "snap"]);
    assert!(out.status.success(), "snapshot create failed: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("private account seed"), "no seed warning: {stderr}");
    assert!(stderr.contains("leaked.key"), "warning names the file: {stderr}");
}

// A snapshot with no seed-like files stays quiet.
#[test]
fn snapshot_create_quiet_without_seeds() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init", "."]).status.success());
    std::fs::write(dir.path().join("README.md"), "# just docs\nnothing secret").unwrap();
    let out = run(dir.path(), &["snapshot", "create", "--from-workspace", "--no-advance", "-m", "snap"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("private account seed"), "false-positive seed warning: {stderr}");
}
