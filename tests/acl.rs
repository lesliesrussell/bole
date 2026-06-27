// bole-63q
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, MergeCheck, PathAcl, PathRole, Permission, Repository, TimelineAcl, TimelineRole};
use bytes::Bytes;
use std::collections::BTreeMap;

/// T3: Snapshot path filtering.
/// Build a snapshot with 3 paths at different ACL levels.
/// Verify visibility depends on accessor's roles.
#[tokio::test]
async fn t3_path_filtering() {
    let repo = Repository::memory();

    // Protect two path namespaces
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();
    repo.acls.set_path_acl(PathAcl { glob: "notes/**".into() }).unwrap();

    // Build snapshot: one public path + two protected paths
    let blob_pub = repo.objects.put_blob(Bytes::from("public code")).await.unwrap();
    let blob_sec = repo.objects.put_blob(Bytes::from("s3cr3t")).await.unwrap();
    let blob_note = repo.objects.put_blob(Bytes::from("private note")).await.unwrap();

    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: blob_pub, kind: EntryKind::Blob });
    entries.insert("secrets/prod.key".into(), TreeEntry { id: blob_sec, kind: EntryKind::Blob });
    entries.insert("notes/private.md".into(), TreeEntry { id: blob_note, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();

    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 1, message: "init".into(),
    }).await.unwrap();

    // Empty accessor: only public path visible
    let empty = Accessor::new();
    let f = repo.get_snapshot_filtered(snap_id, &empty).await.unwrap().unwrap();
    assert_eq!(f.visible_paths.len(), 1);
    assert!(f.visible_paths.contains_key("src/app.rs"));
    assert!(!f.visible_paths.contains_key("secrets/prod.key"));
    assert!(!f.visible_paths.contains_key("notes/private.md"));

    // Accessor with secrets read: public + secrets visible
    let sec_only = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
    let f2 = repo.get_snapshot_filtered(snap_id, &sec_only).await.unwrap().unwrap();
    assert_eq!(f2.visible_paths.len(), 2);
    assert!(f2.visible_paths.contains_key("src/app.rs"));
    assert!(f2.visible_paths.contains_key("secrets/prod.key"));
    assert!(!f2.visible_paths.contains_key("notes/private.md"));

    // Accessor with both roles: all three paths visible
    let full = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read })
        .with_path_role(PathRole { glob: "notes/**".into(), permission: Permission::Read });
    let f3 = repo.get_snapshot_filtered(snap_id, &full).await.unwrap().unwrap();
    assert_eq!(f3.visible_paths.len(), 3);
    assert!(f3.visible_paths.contains_key("src/app.rs"));
    assert!(f3.visible_paths.contains_key("secrets/prod.key"));
    assert!(f3.visible_paths.contains_key("notes/private.md"));
}

/// T3: Timeline filtering.
/// Protected timelines are hidden from callers without the matching role.
#[tokio::test]
async fn t3_timeline_filtering() {
    let repo = Repository::memory();

    repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();

    let id = bole::object::ObjectId::new([1u8; 32]);
    repo.refs.create_tag(RefName::new("main").unwrap(), id, None, 1).unwrap();
    repo.refs.create_tag(RefName::new("leslie/private/exp-foo").unwrap(), id, None, 2).unwrap();

    // Empty accessor: private timeline hidden
    let empty = Accessor::new();
    let visible = repo.list_refs_filtered("", &empty).unwrap();
    let names: Vec<&str> = visible.iter().map(|n| n.as_str()).collect();
    assert!(names.contains(&"main"), "main should be visible");
    assert!(!names.contains(&"leslie/private/exp-foo"), "private timeline should be hidden");

    // Accessor with the matching timeline role: both visible
    let privileged = Accessor::new()
        .with_timeline_role(TimelineRole {
            pattern: "leslie/private/**".into(),
            permission: Permission::Read,
        });
    let visible2 = repo.list_refs_filtered("", &privileged).unwrap();
    let names2: Vec<&str> = visible2.iter().map(|n| n.as_str()).collect();
    assert!(names2.contains(&"main"));
    assert!(names2.contains(&"leslie/private/exp-foo"));
}

/// T3: Merge check.
/// Merging a timeline whose head contains protected paths into a public
/// timeline should be RequiresApproval (if caller has write) or Rejected.
#[tokio::test]
async fn t3_merge_check() {
    let repo = Repository::memory();

    // Protect secrets/** paths
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

    // Build a snapshot with a secret path
    let sec_blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
    let pub_blob = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("secrets/prod.key".into(), TreeEntry { id: sec_blob, kind: EntryKind::Blob });
    entries.insert("src/main.rs".into(), TreeEntry { id: pub_blob, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 1, message: "secret commit".into(),
    }).await.unwrap();

    // Create source timeline pointing at the secret-containing snapshot
    let source = RefName::new("feature/secret-work").unwrap();
    repo.refs.create_timeline(source.clone(), snap_id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();

    // Create a public destination timeline
    let dest = RefName::new("main").unwrap();
    let pub_snap = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 2, message: "public".into(),
    }).await.unwrap();
    repo.refs.create_timeline(dest.clone(), pub_snap, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).unwrap();

    // Accessor with write on dest: RequiresApproval
    let writer = Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write });
    let result = repo.check_merge(&source, &dest, &writer).await.unwrap();
    match result {
        MergeCheck::RequiresApproval(acls) => {
            assert!(!acls.is_empty(), "should report which paths would leak");
            assert!(acls.iter().any(|a| a.glob == "secrets/**"));
        }
        other => panic!("expected RequiresApproval, got {:?}", other),
    }

    // Accessor with no write on dest: Rejected
    let reader = Accessor::new();
    let result2 = repo.check_merge(&source, &dest, &reader).await.unwrap();
    match result2 {
        MergeCheck::Rejected(acls) => {
            assert!(!acls.is_empty());
        }
        other => panic!("expected Rejected, got {:?}", other),
    }

    // Clean source with no protected paths: Allowed
    let clean_blob = repo.objects.put_blob(Bytes::from("clean")).await.unwrap();
    let mut clean_entries = BTreeMap::new();
    clean_entries.insert("src/lib.rs".into(), TreeEntry { id: clean_blob, kind: EntryKind::Blob });
    let clean_tree = repo.objects.put_tree(clean_entries).await.unwrap();
    let clean_snap = repo.objects.put_snapshot(Snapshot {
        root: clean_tree, parents: vec![], author: "test".into(),
        created_at: 3, message: "clean".into(),
    }).await.unwrap();
    let clean_source = RefName::new("feature/clean").unwrap();
    repo.refs.create_timeline(clean_source.clone(), clean_snap, TimelinePolicy::Unrestricted, 3, "persistent".into(), None).unwrap();
    let result3 = repo.check_merge(&clean_source, &dest, &reader).await.unwrap();
    assert_eq!(result3, MergeCheck::Allowed);
}
