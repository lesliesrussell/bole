// bole-7rn
//! Tests for the `explain_path` decision-trace API: the access decision an
//! actor gets on a path, plus *why* — the effective label, the rules that set
//! it, and the per-clearance evaluation that granted or denied access.

use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::{Accessor, PathAcl, PathRole, Permission, Repository};
use bytes::Bytes;
use std::collections::BTreeMap;

/// Builds a snapshot with one public path and two protected namespaces.
async fn fixture(repo: &Repository) -> bole::ObjectId {
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();
    repo.acls.set_path_acl(PathAcl { glob: "notes/**".into() }).unwrap();

    let blob_pub = repo.objects.put_blob(Bytes::from("public code")).await.unwrap();
    let blob_sec = repo.objects.put_blob(Bytes::from("s3cr3t")).await.unwrap();
    let blob_note = repo.objects.put_blob(Bytes::from("note")).await.unwrap();

    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: blob_pub, kind: EntryKind::Blob });
    entries.insert("secrets/prod.key".into(), TreeEntry { id: blob_sec, kind: EntryKind::Blob });
    entries.insert("notes/private.md".into(), TreeEntry { id: blob_note, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();

    repo.objects
        .put_snapshot(Snapshot {
            root: tree_id,
            parents: vec![],
            author: "test".into(),
            created_at: 1,
            message: "init".into(),
        })
        .await
        .unwrap()
}

/// A public (bottom-label) path is readable by everyone via the repo-level
/// short-circuit, even an accessor with no clearances.
#[tokio::test]
async fn public_path_readable_by_empty_accessor() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let exp = repo
        .explain_path(&Accessor::new(), snap, "src/app.rs")
        .await
        .unwrap();

    assert!(exp.present, "path exists in the snapshot");
    assert!(exp.read.allowed, "public path is readable by all");
    assert!(exp.matched_rules.is_empty(), "no protection rule matches a public path");
    // The reason must attribute the grant to the public/bottom short-circuit.
    assert!(
        exp.read.reason.to_lowercase().contains("public")
            || exp.read.reason.to_lowercase().contains("bottom"),
        "reason should cite the public/bottom short-circuit, got: {}",
        exp.read.reason
    );
}

/// A protected path is denied to an accessor with no matching clearance, and
/// the trace names the protecting rule.
#[tokio::test]
async fn protected_path_denied_without_clearance() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let exp = repo
        .explain_path(&Accessor::new(), snap, "secrets/prod.key")
        .await
        .unwrap();

    assert!(exp.present);
    assert!(!exp.read.allowed, "no clearance -> denied");
    assert!(
        exp.matched_rules.iter().any(|g| g == "secrets/**"),
        "the secrets/** rule should be reported as the label source, got {:?}",
        exp.matched_rules
    );
    // No clearance was decisive.
    assert!(exp.read.clearances.iter().all(|c| !c.decisive));
}

/// A matching read clearance grants read, and the decisive clearance is flagged.
#[tokio::test]
async fn read_clearance_grants_and_is_decisive() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let accessor = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });

    let exp = repo.explain_path(&accessor, snap, "secrets/prod.key").await.unwrap();

    assert!(exp.read.allowed, "read clearance grants read");
    let decisive: Vec<_> = exp.read.clearances.iter().filter(|c| c.decisive).collect();
    assert_eq!(decisive.len(), 1, "exactly one clearance is decisive");
    let d = decisive[0];
    assert!(d.scope_applies, "the decisive clearance is in scope");
    assert!(d.grants_capability, "it carries the read capability");
    assert!(d.dominates, "its ceiling dominates the path label");
}

/// A read-only clearance does not grant write.
#[tokio::test]
async fn read_clearance_does_not_grant_write() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let accessor = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });

    let exp = repo.explain_path(&accessor, snap, "secrets/prod.key").await.unwrap();

    assert!(exp.read.allowed);
    assert!(!exp.write.allowed, "a Read role must not grant write");
}

/// An absent path is reported as not present, with the read decision still
/// computed from the label rules (so callers can reason about would-be access).
#[tokio::test]
async fn absent_path_reported_absent() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let exp = repo
        .explain_path(&Accessor::new(), snap, "secrets/does-not-exist")
        .await
        .unwrap();

    assert!(!exp.present, "path is not in the snapshot");
    assert!(exp.matched_rules.iter().any(|g| g == "secrets/**"));
}
