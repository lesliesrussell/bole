// bole-d45
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, Error, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use std::collections::BTreeMap;

fn src_write_accessor() -> Accessor {
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })
}

fn full_write_accessor() -> Accessor {
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
}

/// T6: Two agents editing different paths merge cleanly.
#[tokio::test]
async fn t6_merge_non_conflicting() {
    let repo = Repository::memory();

    // Common ancestor: src/app.rs and src/config.rs both at v1
    let v1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
    let v2 = repo.objects.put_blob(Bytes::from("v2-app")).await.unwrap();
    let v3 = repo.objects.put_blob(Bytes::from("v2-config")).await.unwrap();

    let mut base_entries = BTreeMap::new();
    base_entries.insert("src/app.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
    base_entries.insert("src/config.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
    let base_tree = repo.objects.put_tree(base_entries).await.unwrap();
    let base_snap = repo.objects.put_snapshot(Snapshot {
        root: base_tree, parents: vec![],
        author: "base".into(), created_at: 1, message: "initial".into(),
    }).await.unwrap();

    // Agent A changes src/app.rs only
    let mut a_entries = BTreeMap::new();
    a_entries.insert("src/app.rs".into(), TreeEntry { id: v2, kind: EntryKind::Blob });
    a_entries.insert("src/config.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
    let a_tree = repo.objects.put_tree(a_entries).await.unwrap();
    let a_snap = repo.objects.put_snapshot(Snapshot {
        root: a_tree, parents: vec![base_snap],
        author: "agent-a".into(), created_at: 2, message: "update app".into(),
    }).await.unwrap();

    // Agent B changes src/config.rs only
    let mut b_entries = BTreeMap::new();
    b_entries.insert("src/app.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
    b_entries.insert("src/config.rs".into(), TreeEntry { id: v3, kind: EntryKind::Blob });
    let b_tree = repo.objects.put_tree(b_entries).await.unwrap();
    let b_snap = repo.objects.put_snapshot(Snapshot {
        root: b_tree, parents: vec![base_snap],
        author: "agent-b".into(), created_at: 3, message: "update config".into(),
    }).await.unwrap();

    let source = RefName::new("agent/a").unwrap();
    let target = RefName::new("agent/b").unwrap();
    repo.refs.create_timeline(source.clone(), a_snap, TimelinePolicy::Unrestricted, 2, "ephemeral".into(), None).unwrap();
    repo.refs.create_timeline(target.clone(), b_snap, TimelinePolicy::Unrestricted, 3, "ephemeral".into(), None).unwrap();

    let result = repo.merge_timelines(&source, &target, &full_write_accessor()).await.unwrap();

    assert!(result.is_clean(), "expected clean merge, got conflicts: {:?}", result.conflicts);
    assert_eq!(result.merged.get("src/app.rs"), Some(&v2));
    assert_eq!(result.merged.get("src/config.rs"), Some(&v3));
}

/// T6: Two agents editing the same path produce a conflict.
/// Caller resolves by choosing the source side; advance_timeline succeeds.
#[tokio::test]
async fn t6_merge_conflict() {
    let repo = Repository::memory();

    let v1 = repo.objects.put_blob(Bytes::from("original")).await.unwrap();
    let va = repo.objects.put_blob(Bytes::from("agent-a version")).await.unwrap();
    let vb = repo.objects.put_blob(Bytes::from("agent-b version")).await.unwrap();

    let mut base_entries = BTreeMap::new();
    base_entries.insert("src/shared.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
    let base_tree = repo.objects.put_tree(base_entries).await.unwrap();
    let base_snap = repo.objects.put_snapshot(Snapshot {
        root: base_tree, parents: vec![],
        author: "base".into(), created_at: 1, message: "initial".into(),
    }).await.unwrap();

    let mut a_entries = BTreeMap::new();
    a_entries.insert("src/shared.rs".into(), TreeEntry { id: va, kind: EntryKind::Blob });
    let a_tree = repo.objects.put_tree(a_entries).await.unwrap();
    let a_snap = repo.objects.put_snapshot(Snapshot {
        root: a_tree, parents: vec![base_snap],
        author: "agent-a".into(), created_at: 2, message: "a edits shared".into(),
    }).await.unwrap();

    let mut b_entries = BTreeMap::new();
    b_entries.insert("src/shared.rs".into(), TreeEntry { id: vb, kind: EntryKind::Blob });
    let b_tree = repo.objects.put_tree(b_entries).await.unwrap();
    let b_snap = repo.objects.put_snapshot(Snapshot {
        root: b_tree, parents: vec![base_snap],
        author: "agent-b".into(), created_at: 3, message: "b edits shared".into(),
    }).await.unwrap();

    let source = RefName::new("tl/a").unwrap();
    let target = RefName::new("tl/b").unwrap();
    repo.refs.create_timeline(source.clone(), a_snap, TimelinePolicy::Unrestricted, 2, "ephemeral".into(), None).unwrap();
    repo.refs.create_timeline(target.clone(), b_snap, TimelinePolicy::Unrestricted, 3, "ephemeral".into(), None).unwrap();

    let result = repo.merge_timelines(&source, &target, &full_write_accessor()).await.unwrap();

    assert_eq!(result.conflicts.len(), 1);
    let conflict = &result.conflicts[0];
    assert_eq!(conflict.path, "src/shared.rs");
    // ours = target's blob (b), theirs = source's blob (a)
    assert_eq!(conflict.ours, Some(vb));
    assert_eq!(conflict.theirs, Some(va));

    // Caller resolves: pick theirs (agent A's version)
    let mut resolved = result.merged;
    resolved.insert("src/shared.rs".into(), conflict.theirs.unwrap());
    let resolved_tree = repo.objects.put_tree(resolved.iter().map(|(k, &v)| {
        (k.clone(), TreeEntry { id: v, kind: EntryKind::Blob })
    }).collect()).await.unwrap();
    let resolved_snap = repo.objects.put_snapshot(Snapshot {
        root: resolved_tree, parents: vec![a_snap, b_snap],
        author: "resolver".into(), created_at: 4, message: "merge".into(),
    }).await.unwrap();
    repo.advance_timeline(&target, resolved_snap, &full_write_accessor()).await.unwrap();

    let head = repo.refs.get_timeline(&target).unwrap().unwrap().head;
    assert_eq!(head, resolved_snap);
}

/// T6: Agent restricted to src/** cannot advance a timeline containing secrets/**.
#[tokio::test]
async fn t6_agent_capability_enforced() {
    let repo = Repository::memory();

    let secret_blob = repo.objects.put_blob(Bytes::from("PRIVATE")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: secret_blob, kind: EntryKind::Blob });
    entries.insert("secrets/prod.key".into(), TreeEntry { id: secret_blob, kind: EntryKind::Blob });
    let tree = repo.objects.put_tree(entries).await.unwrap();
    let snap = repo.objects.put_snapshot(Snapshot {
        root: tree, parents: vec![],
        author: "agent".into(), created_at: 1, message: "m".into(),
    }).await.unwrap();

    let name = RefName::new("agent/restricted").unwrap();
    repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), None).unwrap();

    // Agent can write src/** but not secrets/**
    let err = repo.advance_timeline(&name, snap, &src_write_accessor()).await.unwrap_err();
    assert!(
        matches!(err, Error::AccessDenied(_)),
        "expected AccessDenied, got {:?}", err
    );
}

/// T6: Ephemeral timeline is pruned after TTL; tagged head survives pruning.
#[tokio::test]
async fn t6_ephemeral_prune() {
    let repo = Repository::memory();

    let blob = repo.objects.put_blob(Bytes::from("work")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/work.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
    let tree = repo.objects.put_tree(entries).await.unwrap();
    let snap = repo.objects.put_snapshot(Snapshot {
        root: tree, parents: vec![],
        author: "agent".into(), created_at: 1, message: "session work".into(),
    }).await.unwrap();

    // Ephemeral timeline expires at t=100
    let name = RefName::new("ephemeral/session-xyz").unwrap();
    repo.refs.create_timeline(
        name.clone(), snap, TimelinePolicy::Unrestricted, 1,
        "ephemeral".into(), Some(100),
    ).unwrap();

    // At t=200 (past expiry), no tags → pruned
    let pruned = repo.prune_timeline(&name, 200).unwrap();
    assert!(pruned, "expected timeline to be pruned");
    assert!(repo.refs.get_timeline(&name).unwrap().is_none());

    // Re-create the timeline and add a tag → should survive pruning
    repo.refs.create_timeline(
        name.clone(), snap, TimelinePolicy::Unrestricted, 1,
        "ephemeral".into(), Some(100),
    ).unwrap();
    repo.refs.create_tag(RefName::new("v1.0-promoted").unwrap(), snap, None, 1).unwrap();

    let pruned = repo.prune_timeline(&name, 200).unwrap();
    assert!(!pruned, "expected timeline to survive because head is tagged");
    assert!(repo.refs.get_timeline(&name).unwrap().is_some());
}
