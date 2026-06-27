// bole-9lj
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::error::Result;
use crate::object::{Object, ObjectId};
use crate::store::ObjectStore;

/// Represents a path that could not be automatically merged.
/// `ours` or `theirs` is `None` when that side deleted the path.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeConflict {
    pub path: String,
    pub ours: Option<ObjectId>,
    pub theirs: Option<ObjectId>,
}

/// Result of a three-way merge over flat path→blob maps.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub merged: BTreeMap<String, ObjectId>,
    pub conflicts: Vec<MergeConflict>,
}

impl MergeResult {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Pure three-way diff over flat file maps.
///
/// Each argument is a `path → blob-id` map produced by walking a tree snapshot.
/// Returns the merged map plus any conflicts that need human resolution.
pub(crate) fn three_way_diff(
    ancestor: &BTreeMap<String, ObjectId>,
    ours: &BTreeMap<String, ObjectId>,
    theirs: &BTreeMap<String, ObjectId>,
) -> MergeResult {
    let mut all_keys: BTreeSet<String> = BTreeSet::new();
    all_keys.extend(ancestor.keys().cloned());
    all_keys.extend(ours.keys().cloned());
    all_keys.extend(theirs.keys().cloned());

    let mut merged: BTreeMap<String, ObjectId> = BTreeMap::new();
    let mut conflicts: Vec<MergeConflict> = Vec::new();

    for key in all_keys {
        let anc = ancestor.get(&key);
        let our = ours.get(&key);
        let thr = theirs.get(&key);

        // Both sides agree (same id or both absent) → no conflict
        if our == thr {
            if let Some(&id) = our {
                merged.insert(key, id);
            }
            // Both absent → deleted on both sides; omit from result
            continue;
        }

        // Sides differ — classify by what each side changed relative to ancestor
        match (anc, our, thr) {
            // Both sides present with distinct values
            (_, Some(&o), Some(&t)) => {
                if anc == Some(&o) {
                    // Ours unchanged; theirs advanced → accept theirs
                    merged.insert(key, t);
                } else if anc == Some(&t) {
                    // Theirs unchanged; ours advanced → accept ours
                    merged.insert(key, o);
                } else {
                    // Both changed independently → conflict
                    conflicts.push(MergeConflict { path: key, ours: Some(o), theirs: Some(t) });
                }
            }
            // Ours present, theirs absent — distinguish new-add from modified+deleted
            (Some(&a_id), Some(&o), None) => {
                if o == a_id {
                    // Ours unchanged; theirs deleted → accept deletion
                } else {
                    // Ours modified; theirs deleted → conflict
                    conflicts.push(MergeConflict { path: key, ours: Some(o), theirs: None });
                }
            }
            (None, Some(&o), None) => {
                // File is new on our side only; theirs never had it → take ours
                merged.insert(key, o);
            }
            // Theirs present, ours absent — symmetric to above
            (Some(&a_id), None, Some(&t)) => {
                if t == a_id {
                    // Theirs unchanged; ours deleted → accept deletion
                } else {
                    // Theirs modified; ours deleted → conflict
                    conflicts.push(MergeConflict { path: key, ours: None, theirs: Some(t) });
                }
            }
            (None, None, Some(&t)) => {
                // File is new on their side only; ours never had it → take theirs
                merged.insert(key, t);
            }
            // Both absent is already handled by the `our == thr` check above
            (_, None, None) => {}
        }
    }

    MergeResult { merged, conflicts }
}

/// BFS from both snapshot ids simultaneously to find a lowest common ancestor.
///
/// Returns `None` when the histories are completely disjoint (no shared root).
pub(crate) async fn find_common_ancestor(
    store: &ObjectStore,
    a: ObjectId,
    b: ObjectId,
) -> Result<Option<ObjectId>> {
    if a == b {
        return Ok(Some(a));
    }

    let mut visited_a: BTreeSet<ObjectId> = BTreeSet::new();
    let mut visited_b: BTreeSet<ObjectId> = BTreeSet::new();
    let mut frontier_a: VecDeque<ObjectId> = VecDeque::new();
    let mut frontier_b: VecDeque<ObjectId> = VecDeque::new();

    visited_a.insert(a);
    visited_b.insert(b);
    frontier_a.push_back(a);
    frontier_b.push_back(b);

    // Check immediate overlap before any expansion
    if visited_b.contains(&a) {
        return Ok(Some(a));
    }
    if visited_a.contains(&b) {
        return Ok(Some(b));
    }

    loop {
        let grew_a = expand_frontier(store, &mut frontier_a, &mut visited_a).await?;
        if let Some(&common) = visited_a.iter().find(|id| visited_b.contains(*id)) {
            return Ok(Some(common));
        }

        let grew_b = expand_frontier(store, &mut frontier_b, &mut visited_b).await?;
        if let Some(&common) = visited_b.iter().find(|id| visited_a.contains(*id)) {
            return Ok(Some(common));
        }

        if !grew_a && !grew_b {
            return Ok(None);
        }
    }
}

/// Drain `frontier`, load each snapshot's parents, and push any newly-seen
/// parents back into `frontier`.  Returns `true` if at least one new node
/// was added to `visited`.
async fn expand_frontier(
    store: &ObjectStore,
    frontier: &mut VecDeque<ObjectId>,
    visited: &mut BTreeSet<ObjectId>,
) -> Result<bool> {
    let current: Vec<ObjectId> = frontier.drain(..).collect();
    let mut found_new = false;
    for id in current {
        if let Some(Object::Snapshot(snap)) = store.get(&id).await? {
            for parent in snap.parents {
                if visited.insert(parent) {
                    frontier.push_back(parent);
                    found_new = true;
                }
            }
        }
    }
    Ok(found_new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{ObjectId, Snapshot};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use bytes::Bytes;

    fn id(b: u8) -> ObjectId {
        ObjectId::new([b; 32])
    }

    fn map(pairs: &[(&str, u8)]) -> BTreeMap<String, ObjectId> {
        pairs.iter().map(|(k, b)| (k.to_string(), id(*b))).collect()
    }

    // ── three_way_diff ────────────────────────────────────────────────────────

    #[test]
    fn clean_merge_takes_changed_sides() {
        let anc = map(&[("a", 1), ("b", 2)]);
        let ours = map(&[("a", 10), ("b", 2)]); // ours changed "a"
        let theirs = map(&[("a", 1), ("b", 20)]); // theirs changed "b"
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["a"], id(10));
        assert_eq!(r.merged["b"], id(20));
    }

    #[test]
    fn both_sides_same_change_is_clean() {
        let anc = map(&[("x", 1)]);
        let ours = map(&[("x", 99)]);
        let theirs = map(&[("x", 99)]);
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["x"], id(99));
    }

    #[test]
    fn both_changed_differently_is_conflict() {
        let anc = map(&[("f", 1)]);
        let ours = map(&[("f", 2)]);
        let theirs = map(&[("f", 3)]);
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(!r.is_clean());
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].path, "f");
        assert_eq!(r.conflicts[0].ours, Some(id(2)));
        assert_eq!(r.conflicts[0].theirs, Some(id(3)));
    }

    #[test]
    fn modify_vs_delete_is_conflict() {
        let anc = map(&[("g", 1)]);
        let ours = map(&[("g", 2)]); // ours modified
        let theirs = map(&[]); // theirs deleted
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(!r.is_clean());
        assert_eq!(r.conflicts[0].ours, Some(id(2)));
        assert_eq!(r.conflicts[0].theirs, None);
    }

    #[test]
    fn delete_vs_modify_is_conflict() {
        let anc = map(&[("g", 1)]);
        let ours = map(&[]); // ours deleted
        let theirs = map(&[("g", 2)]); // theirs modified
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(!r.is_clean());
        assert_eq!(r.conflicts[0].ours, None);
        assert_eq!(r.conflicts[0].theirs, Some(id(2)));
    }

    #[test]
    fn both_delete_same_file_is_clean() {
        let anc = map(&[("h", 1)]);
        let ours = map(&[]);
        let theirs = map(&[]);
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(r.is_clean());
        assert!(!r.merged.contains_key("h"));
    }

    #[test]
    fn unchanged_side_delete_accepted() {
        // Theirs unchanged, ours deleted → accept deletion (ours wins)
        let anc = map(&[("z", 5)]);
        let ours = map(&[]); // deleted
        let theirs = map(&[("z", 5)]); // unchanged
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(r.is_clean());
        assert!(!r.merged.contains_key("z"));
    }

    #[test]
    fn new_file_only_on_one_side_is_clean() {
        let anc = map(&[]);
        let ours = map(&[("new", 7)]);
        let theirs = map(&[]);
        let r = three_way_diff(&anc, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["new"], id(7));
    }

    // bole-9lj
    #[test]
    fn both_added_differently_is_conflict() {
        let ancestor = map(&[]);  // no common ancestor
        let ours = map(&[("a.rs", 1)]);
        let theirs = map(&[("a.rs", 2)]);  // different blob
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].path, "a.rs");
        assert_eq!(r.conflicts[0].ours, Some(id(1)));
        assert_eq!(r.conflicts[0].theirs, Some(id(2)));
    }

    // ── find_common_ancestor ─────────────────────────────────────────────────

    /// Each snapshot gets a unique root blob so content-addressed hashing
    /// never collides between snapshots with different labels.
    async fn make_snapshot(store: &ObjectStore, parents: Vec<ObjectId>, label: &str) -> ObjectId {
        let root = store.put_blob(Bytes::from(format!("root-{label}"))).await.unwrap();
        store.put_snapshot(Snapshot {
            root,
            parents,
            author: label.into(),
            created_at: 0,
            message: label.into(),
        }).await.unwrap()
    }

    #[tokio::test]
    async fn same_id_is_its_own_ancestor() {
        let store = ObjectStore::new(MemoryBackend::new());
        let snap = make_snapshot(&store, vec![], "only").await;
        assert_eq!(find_common_ancestor(&store, snap, snap).await.unwrap(), Some(snap));
    }

    #[tokio::test]
    async fn linear_history_finds_ancestor() {
        let store = ObjectStore::new(MemoryBackend::new());
        let base = make_snapshot(&store, vec![], "base").await;
        let mid  = make_snapshot(&store, vec![base], "mid").await;
        let tip_a = make_snapshot(&store, vec![mid], "tip_a").await;
        let tip_b = make_snapshot(&store, vec![mid], "tip_b").await;
        let lca = find_common_ancestor(&store, tip_a, tip_b).await.unwrap();
        // The closest common ancestor is mid; base is also valid but further back
        assert_eq!(lca, Some(mid));
    }

    #[tokio::test]
    async fn disjoint_histories_return_none() {
        let store = ObjectStore::new(MemoryBackend::new());
        let a = make_snapshot(&store, vec![], "alpha").await;
        let b = make_snapshot(&store, vec![], "beta").await;
        assert_ne!(a, b, "test setup: snapshots must be distinct");
        assert_eq!(find_common_ancestor(&store, a, b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn direct_parent_is_common_ancestor() {
        let store = ObjectStore::new(MemoryBackend::new());
        let base  = make_snapshot(&store, vec![], "base").await;
        let child = make_snapshot(&store, vec![base], "child").await;
        let lca = find_common_ancestor(&store, base, child).await.unwrap();
        assert_eq!(lca, Some(base));
    }
}
