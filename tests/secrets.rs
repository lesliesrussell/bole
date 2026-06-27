// bole-m9e
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::{
    Accessor, EnvOverlay, EnvValue, PathAcl, PathRole, Permission, Repository, WorkspaceView,
};
use bytes::Bytes;
use std::collections::BTreeMap;

fn key() -> [u8; 32] {
    [42u8; 32]
}
fn wrong_key() -> [u8; 32] {
    [99u8; 32]
}

/// T4: Secret encrypt/decrypt roundtrip via ObjectStore.
/// Verifies put_secret stores ciphertext and get_secret decrypts correctly.
/// Verifies wrong key returns DecryptionFailed.
#[tokio::test]
async fn t4_secret_roundtrip() {
    let repo = Repository::memory();
    let key = key();

    // Store and retrieve
    let id = repo.objects.put_secret(b"s3cr3t value", &key).await.unwrap();
    let got = repo.objects.get_secret(&id, &key).await.unwrap().unwrap();
    assert_eq!(got, b"s3cr3t value");

    // Wrong key fails
    let err = repo.objects.get_secret(&id, &wrong_key()).await.unwrap_err();
    assert!(
        matches!(err, bole::Error::DecryptionFailed),
        "expected DecryptionFailed, got {:?}",
        err
    );

    // Missing id returns None
    let missing = bole::ObjectId::new([0u8; 32]);
    let none = repo.objects.get_secret(&missing, &key).await.unwrap();
    assert!(none.is_none());
}

/// T4: compute_workspace_view resolves Plain and Secret EnvValues,
/// returns correct file set from the snapshot.
#[tokio::test]
async fn t4_workspace_view() {
    let repo = Repository::memory();
    let key = key();

    // Build snapshot with two paths
    let blob1 = repo.objects.put_blob(Bytes::from("app code")).await.unwrap();
    let blob2 = repo
        .objects
        .put_blob(Bytes::from("config code"))
        .await
        .unwrap();
    let mut entries = BTreeMap::new();
    entries.insert(
        "src/app.rs".into(),
        TreeEntry {
            id: blob1,
            kind: EntryKind::Blob,
        },
    );
    entries.insert(
        "src/config.rs".into(),
        TreeEntry {
            id: blob2,
            kind: EntryKind::Blob,
        },
    );
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo
        .objects
        .put_snapshot(Snapshot {
            root: tree_id,
            parents: vec![],
            author: "test".into(),
            created_at: 1,
            message: "initial".into(),
        })
        .await
        .unwrap();

    // Store a secret
    let secret_id = repo
        .objects
        .put_secret(b"postgres://prod", &key)
        .await
        .unwrap();

    // Build overlay with one plain value and one secret reference
    let mut env_entries = BTreeMap::new();
    env_entries.insert("DB_URL".into(), EnvValue::Secret(secret_id));
    env_entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
    let overlay_id = repo
        .objects
        .put_overlay(EnvOverlay {
            entries: env_entries,
        })
        .await
        .unwrap();

    // Full accessor (** glob = can read all paths)
    let accessor = Accessor::new().with_path_role(PathRole {
        glob: "**".into(),
        permission: Permission::Read,
    });

    let view: WorkspaceView = repo
        .compute_workspace_view(snap_id, overlay_id, &key, &accessor)
        .await
        .unwrap()
        .unwrap();

    // Files: both paths visible
    assert_eq!(view.files.len(), 2);
    assert!(view.files.contains_key("src/app.rs"));
    assert!(view.files.contains_key("src/config.rs"));

    // Env: both values resolved
    assert_eq!(view.env.get("DB_URL").map(String::as_str), Some("postgres://prod"));
    assert_eq!(view.env.get("LOG_LEVEL").map(String::as_str), Some("info"));
}

/// T4: compute_workspace_view ACL filters snapshot paths.
/// Protected paths are hidden from callers lacking the role.
/// Env resolves independently of snapshot path ACLs.
#[tokio::test]
async fn t4_workspace_view_acl_filtered() {
    let repo = Repository::memory();
    let key = key();

    // Protect src/config.rs
    repo.acls
        .set_path_acl(PathAcl {
            glob: "src/config.rs".into(),
        })
        .unwrap();

    // Build snapshot with one public + one protected path
    let blob = repo.objects.put_blob(Bytes::from("content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert(
        "src/app.rs".into(),
        TreeEntry {
            id: blob,
            kind: EntryKind::Blob,
        },
    );
    entries.insert(
        "src/config.rs".into(),
        TreeEntry {
            id: blob,
            kind: EntryKind::Blob,
        },
    );
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo
        .objects
        .put_snapshot(Snapshot {
            root: tree_id,
            parents: vec![],
            author: "test".into(),
            created_at: 1,
            message: "m".into(),
        })
        .await
        .unwrap();

    // Secret in overlay
    let secret_id = repo
        .objects
        .put_secret(b"my-api-key", &key)
        .await
        .unwrap();
    let mut env_entries = BTreeMap::new();
    env_entries.insert("API_KEY".into(), EnvValue::Secret(secret_id));
    env_entries.insert("MODE".into(), EnvValue::Plain("dev".into()));
    let overlay_id = repo
        .objects
        .put_overlay(EnvOverlay {
            entries: env_entries,
        })
        .await
        .unwrap();

    // Empty accessor: no path roles → cannot read src/config.rs
    let view: WorkspaceView = repo
        .compute_workspace_view(snap_id, overlay_id, &key, &Accessor::new())
        .await
        .unwrap()
        .unwrap();

    // Only the public path is visible
    assert_eq!(view.files.len(), 1);
    assert!(view.files.contains_key("src/app.rs"));
    assert!(!view.files.contains_key("src/config.rs"));

    // Env resolves regardless of path ACLs
    assert_eq!(view.env.get("API_KEY").map(String::as_str), Some("my-api-key"));
    assert_eq!(view.env.get("MODE").map(String::as_str), Some("dev"));
}
