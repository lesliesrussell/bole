// bole-aqk
//! Integration tests driving the compiled `bole` binary.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

#[test]
fn init_then_status_reports_empty_repo() {
    let dir = tempfile::tempdir().unwrap();

    let init = bin().arg("init").arg(".").current_dir(dir.path()).output().unwrap();
    assert!(init.status.success(), "init failed: {init:?}");
    assert!(dir.path().join(".bole").is_dir(), ".bole/ not created");

    let status = bin()
        .arg("status")
        .arg("--json")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(status.status.success(), "status failed: {status:?}");
    let v: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(v["ref_count"], 0);
    assert!(v["timeline"].is_null());
}

#[test]
fn status_discovers_repo_from_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let init = bin().arg("init").arg(".").current_dir(dir.path()).output().unwrap();
    assert!(init.status.success());

    let sub = dir.path().join("a").join("b");
    std::fs::create_dir_all(&sub).unwrap();
    let status = bin().arg("status").arg("--json").current_dir(&sub).output().unwrap();
    assert!(status.status.success(), "discovery failed: {status:?}");
}

#[test]
fn init_twice_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(bin().arg("init").arg(".").current_dir(dir.path()).output().unwrap().status.success());
    let second = bin().arg("init").arg(".").current_dir(dir.path()).output().unwrap();
    assert!(!second.status.success(), "second init should fail");
}

#[test]
fn status_outside_repo_errors() {
    let dir = tempfile::tempdir().unwrap();
    let status = bin().arg("status").current_dir(dir.path()).output().unwrap();
    assert!(!status.status.success(), "status with no repo should fail");
}
