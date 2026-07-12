// bole-cg06
//! Integration test for native repo sync over TCP: `bole serve` + `bole push`.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use bole::{EntryKind, Repository, Snapshot, TreeEntry};
use bytes::Bytes;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

/// Seeds `dir` with an initialized repo and a `main` timeline pointing at a
/// one-file snapshot. Returns the head id (hex).
async fn seed_main(dir: &Path) -> String {
    // Initialize via the binary so cli-state / layout match a real repo.
    let out = bin().args(["init", "."]).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "init failed: {out:?}");
    let repo = Repository::disk(dir.join(".bole")).await.unwrap();
    let blob = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
    let mut entries = std::collections::BTreeMap::new();
    entries.insert("f.txt".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
    let tree = repo.objects.put_tree(entries).await.unwrap();
    let head = repo
        .objects
        .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "init".into() })
        .await
        .unwrap();
    repo.refs
        .create_timeline(
            bole::RefName::new("main").unwrap(),
            head,
            bole::TimelinePolicy::Unrestricted,
            0,
            "persistent".into(),
            None,
        )
        .unwrap();
    head.to_string()
}

fn wait_for_addr(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(s) = std::fs::read_to_string(path) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("serve never wrote its address to {}", path.display());
}

#[tokio::test]
async fn push_replicates_timeline_to_served_peer() {
    let server = tempfile::tempdir().unwrap();
    let client = tempfile::tempdir().unwrap();

    // Server: an empty initialized repo. Client: has `main`.
    assert!(bin().args(["init", "."]).current_dir(server.path()).output().unwrap().status.success());
    let client_head = seed_main(client.path()).await;

    // Start the server serving exactly one connection; it writes its bound addr.
    let addr_file = server.path().join(".serve-addr");
    let mut serve = bin()
        .args([
            "serve",
            "--listen",
            "127.0.0.1:0",
            "--once",
            "--addr-file",
            addr_file.to_str().unwrap(),
        ])
        .current_dir(server.path())
        .spawn()
        .unwrap();
    let addr = wait_for_addr(&addr_file);

    // Push `main` from the client to the server.
    let push = bin()
        .args(["push", &addr, "main", "--json"])
        .current_dir(client.path())
        .output()
        .unwrap();
    assert!(push.status.success(), "push failed: {push:?}");
    let v: serde_json::Value = serde_json::from_slice(&push.stdout).unwrap();
    let status = v["results"][0]["status"].as_str().unwrap();
    assert!(status.contains("Ok"), "push not accepted: {status}");

    // The --once server should exit on its own after the one connection.
    let _ = serve.wait();

    // The server now has `main` at the client's head.
    let server_repo = Repository::disk(server.path().join(".bole")).await.unwrap();
    let head = server_repo
        .refs
        .get_timeline(&bole::RefName::new("main").unwrap())
        .unwrap()
        .expect("server has main")
        .head;
    assert_eq!(head.to_string(), client_head, "replicated head matches");

    // Explicit closure check: the server can read the snapshot's tree + blob,
    // proving the whole object closure transferred, not just the ref.
    let snap = server_repo.objects.get(&head).await.unwrap().expect("snapshot present");
    let root = match snap { bole::Object::Snapshot(s) => s.root, _ => panic!("not a snapshot") };
    let tree = server_repo.objects.get(&root).await.unwrap().expect("root tree present");
    let blob_id = match tree {
        bole::Object::Tree(t) => t.entries.get("f.txt").expect("f.txt in tree").id,
        _ => panic!("not a tree"),
    };
    assert!(server_repo.objects.get(&blob_id).await.unwrap().is_some(), "blob closure transferred");
}

// bole-1x2v
/// bole serve --hub + bole push --as: an authenticated push lands under the
/// owner's namespace on the hub.
#[tokio::test]
async fn hub_push_lands_in_owner_namespace() {
    let hub = tempfile::tempdir().unwrap();
    let client = tempfile::tempdir().unwrap();
    assert!(bin().args(["init", "."]).current_dir(hub.path()).output().unwrap().status.success());
    let client_head = seed_main(client.path()).await;

    // The owner's key file (64-hex seed).
    let keyfile = client.path().join("owner.key");
    std::fs::write(&keyfile, "ab".repeat(32)).unwrap(); // 0xab * 32
    let owner = bole::RepoSigner::from_seed([0xabu8; 32]).public_key();
    let ns = bole::sync::hub::user_namespace(&owner); // refs/users/<fp>/

    // Hub serving one connection.
    let addr_file = hub.path().join(".addr");
    let mut serve = bin()
        .args(["serve", "--hub", "--listen", "127.0.0.1:0", "--once", "--addr-file", addr_file.to_str().unwrap()])
        .current_dir(hub.path())
        .spawn()
        .unwrap();
    let addr = wait_for_addr(&addr_file);

    // Push local `main` as repo `grove` (→ refs/users/<fp>/grove/main), authenticated.
    let push = bin()
        .args(["push", &addr, "grove:main", "--as", keyfile.to_str().unwrap(), "--json"])
        .current_dir(client.path())
        .output()
        .unwrap();
    assert!(push.status.success(), "hub push failed: {push:?}");
    let v: serde_json::Value = serde_json::from_slice(&push.stdout).unwrap();
    assert!(v["results"][0]["status"].as_str().unwrap().contains("Ok"), "not accepted: {v}");

    let _ = serve.wait();

    // The hub has the repo under the owner's namespace at the client head.
    let hub_repo = Repository::disk(hub.path().join(".bole")).await.unwrap();
    let remote = bole::RefName::new(format!("{ns}grove/main")).unwrap();
    assert_eq!(
        hub_repo.refs.get_timeline(&remote).unwrap().expect("owner namespace populated").head.to_string(),
        client_head
    );
}
