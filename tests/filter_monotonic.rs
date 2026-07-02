// bole-lgt
//! Property test pinning the core invariant of ACL filtering: **filtering only
//! removes visibility.** For any snapshot and any accessor, the filtered view
//!
//!   1. is a *subset* of the full (privileged) path set — no synthetic nodes;
//!   2. maps every retained path to the *same* `ObjectId` as the full tree —
//!      filtering never rewrites content or changes hashes;
//!   3. preserves the snapshot's own identity and metadata; and
//!   4. is monotonic in grants — a strictly more-privileged accessor sees a
//!      superset.
//!
//! We approximate a property test without a proptest dependency by enumerating
//! the powerset of read-grants over the protected namespaces (plus the empty
//! accessor) and checking the invariants for every resulting accessor and every
//! comparable pair.

use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::{Accessor, Object, ObjectId, PathAcl, PathRole, Permission, Repository};
use bytes::Bytes;
use std::collections::BTreeMap;

/// The protected namespaces a grant may cover.
const NAMESPACES: [&str; 3] = ["secrets/**", "notes/**", "models/**"];

/// Builds a nested tree spanning public and several protected namespaces, each
/// path holding distinct content so its `ObjectId` is unique and identity
/// preservation is a meaningful check.
async fn fixture(repo: &Repository) -> ObjectId {
    for ns in NAMESPACES {
        repo.acls.set_path_acl(PathAcl { glob: ns.into() }).unwrap();
    }

    // (path, contents) — public + nested protected paths.
    let files = [
        ("src/main.rs", "fn main() {}"),
        ("src/deep/mod.rs", "pub mod deep;"),
        ("secrets/prod.key", "prod-secret"),
        ("secrets/prod/db.key", "prod-db-secret"),
        ("notes/private.md", "private note"),
        ("models/weights.bin", "0101"),
    ];

    // Build a nested tree by grouping paths under their top directory so we
    // exercise the recursive walk, not just a flat root.
    let mut roots: BTreeMap<String, BTreeMap<String, TreeEntry>> = BTreeMap::new();
    for (path, body) in files {
        let id = repo.objects.put_blob(Bytes::from(body)).await.unwrap();
        let (top, rest) = path.split_once('/').unwrap();
        roots
            .entry(top.to_string())
            .or_default()
            .insert(rest.to_string(), TreeEntry { id, kind: EntryKind::Blob });
    }

    // Each top-level dir may itself be nested (e.g. secrets/prod/db.key); build
    // sub-trees bottom-up.
    let mut root_entries = BTreeMap::new();
    for (top, mut children) in roots {
        // Pull out any grandchild paths (contain a '/') into sub-trees.
        let mut nested: BTreeMap<String, BTreeMap<String, TreeEntry>> = BTreeMap::new();
        let mut flat = BTreeMap::new();
        for (name, entry) in std::mem::take(&mut children) {
            match name.split_once('/') {
                Some((sub, leaf)) => {
                    nested.entry(sub.to_string()).or_default().insert(leaf.to_string(), entry);
                }
                None => {
                    flat.insert(name, entry);
                }
            }
        }
        for (sub, leaves) in nested {
            let sub_id = repo.objects.put_tree(leaves).await.unwrap();
            flat.insert(sub, TreeEntry { id: sub_id, kind: EntryKind::Tree });
        }
        let top_id = repo.objects.put_tree(flat).await.unwrap();
        root_entries.insert(top, TreeEntry { id: top_id, kind: EntryKind::Tree });
    }
    let root = repo.objects.put_tree(root_entries).await.unwrap();

    repo.objects
        .put_snapshot(Snapshot {
            root,
            parents: vec![],
            author: "alice".into(),
            created_at: 1_700_000_000,
            message: "fixture".into(),
            })
        .await
        .unwrap()
}

/// Builds an accessor granted Read on the namespaces selected by `mask` (one bit
/// per entry of `NAMESPACES`).
fn accessor_for(mask: usize) -> Accessor {
    let mut acc = Accessor::new();
    for (i, ns) in NAMESPACES.iter().enumerate() {
        if mask & (1 << i) != 0 {
            acc = acc.with_path_role(PathRole {
                glob: (*ns).into(),
                permission: Permission::Read,
            });
        }
    }
    acc
}

/// Invariants 1–3 hold for every accessor in the powerset of grants.
#[tokio::test]
async fn filtering_only_removes_visibility() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    // Canonical "full" view: privileged reads everything.
    let full = repo
        .get_snapshot_filtered(snap, &Accessor::privileged())
        .await
        .unwrap()
        .unwrap();

    for mask in 0..(1 << NAMESPACES.len()) {
        let acc = accessor_for(mask);
        let view = repo.get_snapshot_filtered(snap, &acc).await.unwrap().unwrap();

        // (3) Snapshot identity & metadata are never rewritten by filtering.
        assert_eq!(view.id, snap, "mask {mask}: snapshot id changed");
        assert_eq!(view.author, full.author, "mask {mask}: author changed");
        assert_eq!(view.created_at, full.created_at, "mask {mask}: created_at changed");
        assert_eq!(view.message, full.message, "mask {mask}: message changed");
        assert_eq!(view.parents, full.parents, "mask {mask}: parents changed");

        for (path, id) in &view.visible_paths {
            // (1) No synthetic nodes: everything visible exists in the full set.
            assert!(
                full.visible_paths.contains_key(path),
                "mask {mask}: path {path} not present in full view (synthesized?)"
            );
            // (2) Identity preserved: same path -> same ObjectId as unfiltered.
            assert_eq!(
                full.visible_paths.get(path),
                Some(id),
                "mask {mask}: path {path} has a different ObjectId under filtering"
            );
        }

        // (4a) Ordering is canonical (BTreeMap): keys are sorted, never reordered.
        let keys: Vec<_> = view.visible_paths.keys().cloned().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "mask {mask}: visible paths are not in canonical order");

        // Public paths are visible to *every* accessor (bottom-label short-circuit).
        assert!(view.visible_paths.contains_key("src/main.rs"), "mask {mask}: lost public path");
        assert!(view.visible_paths.contains_key("src/deep/mod.rs"), "mask {mask}: lost nested public path");
    }
}

/// Invariant 4: grants are monotonic — if grant set A ⊆ B then the visible set
/// under A ⊆ the visible set under B, for every comparable pair.
#[tokio::test]
async fn visibility_is_monotonic_in_grants() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let n = 1usize << NAMESPACES.len();
    let mut views: Vec<BTreeMap<String, ObjectId>> = Vec::with_capacity(n);
    for mask in 0..n {
        let acc = accessor_for(mask);
        let v = repo.get_snapshot_filtered(snap, &acc).await.unwrap().unwrap();
        views.push(v.visible_paths);
    }

    // For every pair where A's grant bits are a subset of B's, A's visible set
    // must be a subset of B's.
    for a in 0..n {
        for b in 0..n {
            if a & b == a {
                // a ⊆ b as bitmasks
                for (path, id) in &views[a] {
                    assert_eq!(
                        views[b].get(path),
                        Some(id),
                        "monotonicity violated: grant {a} sees {path} but superset grant {b} does not (identically)"
                    );
                }
            }
        }
    }

    // Sanity: the full grant set strictly dominates the empty one.
    assert!(
        views[n - 1].len() > views[0].len(),
        "granting all namespaces must reveal strictly more than the empty accessor"
    );
}

/// A direct check that a hidden path's blob is untouched in the store: filtering
/// removes it from the *view*, not from content-addressed storage.
#[tokio::test]
async fn hidden_paths_remain_in_store() {
    let repo = Repository::memory();
    let snap = fixture(&repo).await;

    let full = repo
        .get_snapshot_filtered(snap, &Accessor::privileged())
        .await
        .unwrap()
        .unwrap();
    let empty = repo.get_snapshot_filtered(snap, &Accessor::new()).await.unwrap().unwrap();

    // secrets/prod.key is hidden from the empty accessor...
    assert!(!empty.visible_paths.contains_key("secrets/prod.key"));
    let hidden_id = full.visible_paths.get("secrets/prod.key").copied().unwrap();

    // ...yet its blob is still retrievable directly by id (nothing was deleted).
    let obj = repo.objects.get(&hidden_id).await.unwrap();
    assert!(
        matches!(obj, Some(Object::Blob(_))),
        "hidden path's blob must remain in the store"
    );
}
