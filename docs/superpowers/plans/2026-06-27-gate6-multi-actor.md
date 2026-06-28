# Gate 6: Multi-Actor / Agents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add multi-actor semantics to `bole`: three-way merge with conflict surfacing, timeline retention via TTL + tag-based promotion, and write capability enforcement on Repository operations.

**Architecture:** New types (`MergeConflict`, `MergeResult`) and pure merge logic live in `src/repo/merge.rs`. Four new `Repository` methods delegate to that module. `Timeline` gains `kind` and `expires_at` fields (serde-defaulted for backward compatibility). No new storage abstractions — all new logic uses existing `ObjectStore`, `RefStore`, and `Accessor` APIs.

**Tech Stack:** Rust (edition 2021, stable, tokio), serde + postcard, thiserror — no new dependencies.

## Global Constraints

- `thiserror` only — no `anyhow` anywhere in library code
- No new crate dependencies
- Both `MemoryBackend` and `DiskBackend` always compiled — no feature flags
- `// <bead-id>` comment on each contiguous block of new code — one per block, not per line; never retroactively tag pre-existing code
- Branch name = bead ID exactly
- Tests must pass before merge; delete branch after merge; close bead after delete
- Conservative git: no push, no dolt sync

---

## File Map

| File | Status | Purpose |
|------|--------|---------|
| `src/refs/timeline.rs` | Modify | Add `kind: String`, `expires_at: Option<u64>` to `Timeline` |
| `src/refs/mod.rs` | Modify | Update `create_timeline` signature; fix all callsites in file |
| `src/acl/mod.rs` | Modify | Add `Accessor::privileged()` |
| `src/repo/merge.rs` | Create | `MergeConflict`, `MergeResult`, `three_way_diff`, `find_common_ancestor` |
| `src/repo/mod.rs` | Modify | Add `pub mod merge;` + four `Repository` methods |
| `src/lib.rs` | Modify | Re-export `MergeConflict`, `MergeResult` |
| `tests/refs.rs` | Modify | Fix `create_timeline` callsites (add `kind`, `expires_at` args) |
| `tests/acl.rs` | Modify | Fix `create_timeline` callsites |
| `tests/multi_actor.rs` | Create | T6 integration tests |

---

## Task 1: Timeline fields + Accessor::privileged

**Files:**
- Modify: `src/refs/timeline.rs`
- Modify: `src/refs/mod.rs`
- Modify: `src/acl/mod.rs`
- Modify: `tests/refs.rs`
- Modify: `tests/acl.rs`

**Interfaces:**
- Produces:
  - `Timeline { head, policy, created_at, kind: String, expires_at: Option<u64> }`
  - `RefStore::create_timeline(name, head, policy, now, kind: String, expires_at: Option<u64>)`
  - `Accessor::privileged() -> Self` — grants read on all paths and timelines

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 6 T1: Timeline kind+expires_at fields, Accessor::privileged" \
  --description="Add kind:String and expires_at:Option<u64> to Timeline struct (serde-defaulted). Update create_timeline signature. Add Accessor::privileged() constructor. Fix all existing callsites in src/ and tests/." \
  --type=task --priority=2
# note the printed bead ID, e.g. bole-abc
bd update bole-abc --claim
git checkout -b bole-abc
```

- [ ] **Step 2: Write failing tests for Timeline fields**

In `src/refs/mod.rs`, add to the existing `#[cfg(test)] mod tests` block:

```rust
    // <bead-id>
    #[test]
    fn timeline_kind_and_expires_at_stored_and_retrieved() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(
            name("ephemeral"),
            id,
            TimelinePolicy::Unrestricted,
            1,
            "ephemeral".into(),
            Some(9999),
        ).unwrap();
        let tl = s.get_timeline(&name("ephemeral")).unwrap().unwrap();
        assert_eq!(tl.kind, "ephemeral");
        assert_eq!(tl.expires_at, Some(9999));
    }

    #[test]
    fn timeline_default_kind_is_persistent() {
        let s = store();
        let id = ObjectId::new([2u8; 32]);
        s.create_timeline(
            name("main"),
            id,
            TimelinePolicy::Unrestricted,
            1,
            "persistent".into(),
            None,
        ).unwrap();
        let tl = s.get_timeline(&name("main")).unwrap().unwrap();
        assert_eq!(tl.kind, "persistent");
        assert_eq!(tl.expires_at, None);
    }
```

- [ ] **Step 3: Run tests to verify compile failure**

```bash
cargo test 2>&1 | grep "error\[" | head -10
```

Expected: compile errors — `create_timeline` called with wrong number of arguments.

- [ ] **Step 4: Update `src/refs/timeline.rs`**

Replace the file completely:

```rust
// bole-prn
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimelinePolicy {
    FastForwardOnly,
    Append,
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub head: ObjectId,
    pub policy: TimelinePolicy,
    pub created_at: u64,
    // <bead-id>
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

// <bead-id>
fn default_kind() -> String {
    "persistent".into()
}
```

- [ ] **Step 5: Update `create_timeline` in `src/refs/mod.rs`**

Find the `create_timeline` function body (currently around line 85) and replace it:

```rust
        pub fn create_timeline(
            &self,
            name: RefName,
            head: ObjectId,
            policy: TimelinePolicy,
            now: u64,
            // <bead-id>
            kind: String,
            expires_at: Option<u64>,
        ) -> Result<()> {
            if self.backend.get(&name)?.is_some() {
                return Err(Error::Storage(format!(
                    "ref already exists: {}",
                    name.as_str()
                )));
            }
            self.backend.set(&name, &Ref::Timeline(Timeline {
                head,
                policy,
                created_at: now,
                // <bead-id>
                kind,
                expires_at,
            }))
        }
```

- [ ] **Step 6: Fix all `create_timeline` callsites inside `src/refs/mod.rs`**

There are 6 test callsites inside `src/refs/mod.rs`. Each needs `"persistent".into(), None` appended. Find each call (around lines 179, 190, 214, 248, 255, 256) and update:

Before: `s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();`
After:  `s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();`

Apply to every `create_timeline(...)` call in the test block of `src/refs/mod.rs`. Keep exact existing arguments; only append `"persistent".into(), None`.

There is also one call in `src/repo/mod.rs` (around line 297, inside a test helper). Update it too:

Before: `from.create_timeline(RefName::new("main").unwrap(), id, TimelinePolicy::Unrestricted, 3).unwrap();`
After:  `from.create_timeline(RefName::new("main").unwrap(), id, TimelinePolicy::Unrestricted, 3, "persistent".into(), None).unwrap();`

- [ ] **Step 7: Fix callsites in `tests/refs.rs` and `tests/acl.rs`**

In `tests/refs.rs` (lines ~22, ~51, ~87): append `"persistent".into(), None` to each `create_timeline` call.

In `tests/acl.rs` (lines ~118, ~126, ~160): append `"persistent".into(), None` to each `create_timeline` call.

- [ ] **Step 8: Add `Accessor::privileged()` to `src/acl/mod.rs`**

In the `impl Accessor` block, after `with_timeline_role`, add:

```rust
    // <bead-id>
    pub fn privileged() -> Self {
        Self::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
    }
```

Add a test in `src/acl/mod.rs`'s test block:

```rust
    // <bead-id>
    #[test]
    fn privileged_accessor_can_read_everything() {
        let a = Accessor::privileged();
        assert!(a.can_read_path("secrets/prod.key"));
        assert!(a.can_read_path("src/main.rs"));
        assert!(a.can_read_timeline("leslie/private/exp-foo"));
        assert!(a.can_read_timeline("main"));
        // privileged does not grant write
        assert!(!a.can_write_path("src/main.rs"));
        assert!(!a.can_write_timeline("main"));
    }
```

- [ ] **Step 9: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass plus the 2 new Timeline tests and 1 new Accessor test.

- [ ] **Step 10: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add src/refs/timeline.rs src/refs/mod.rs src/acl/mod.rs src/repo/mod.rs tests/refs.rs tests/acl.rs
git commit -m "feat(refs): add Timeline kind+expires_at; add Accessor::privileged"
```

- [ ] **Step 12: Merge and close**

```bash
git checkout master && git merge bole-abc
git branch -d bole-abc
bd close bole-abc
```

---

## Task 2: MergeConflict, MergeResult, three-way diff, LCA

**Files:**
- Create: `src/repo/merge.rs`
- Modify: `src/repo/mod.rs` (add `pub mod merge;`)
- Modify: `src/lib.rs` (re-export)

**Interfaces:**
- Consumes (from Task 1): nothing new
- Consumes (existing): `crate::object::{Object, ObjectId, Snapshot}`, `crate::store::ObjectStore`, `crate::error::{Error, Result}`
- Produces:
  - `pub struct MergeConflict { pub path: String, pub ours: Option<ObjectId>, pub theirs: Option<ObjectId> }`
  - `pub struct MergeResult { pub merged: BTreeMap<String, ObjectId>, pub conflicts: Vec<MergeConflict> }` with `pub fn is_clean(&self) -> bool`
  - `pub(crate) fn three_way_diff(ancestor, ours, theirs) -> MergeResult`
  - `pub(crate) async fn find_common_ancestor(store, a, b) -> Result<Option<ObjectId>>`

**Note on `MergeConflict` fields:** The spec shows `ours: ObjectId` (non-optional), but the delete-vs-modify conflict case (one side deleted a path, the other modified it) cannot be represented without `Option<ObjectId>`. The plan uses `Option<ObjectId>` — `None` means "this side deleted the path."

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 6 T2: MergeConflict, MergeResult, three_way_diff, find_common_ancestor" \
  --description="Create src/repo/merge.rs with pure merge logic: MergeConflict+MergeResult types, three_way_diff (pure fn over BTreeMaps), find_common_ancestor (async BFS over ObjectStore parents DAG). Add pub mod merge to repo/mod.rs. Re-export from lib.rs." \
  --type=task --priority=2
bd update bole-def --claim
git checkout -b bole-def
```

- [ ] **Step 2: Add `pub mod merge;` to `src/repo/mod.rs`**

After `pub mod materialize;` and `pub mod workspace;` add:

```rust
// <bead-id>
pub mod merge;
```

- [ ] **Step 3: Write failing tests**

Create `src/repo/merge.rs` with stubs and tests:

```rust
// <bead-id>
use crate::error::Result;
use crate::object::{Object, ObjectId};
use crate::store::ObjectStore;
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq)]
pub struct MergeConflict {
    pub path: String,
    pub ours: Option<ObjectId>,    // None = this side deleted the path
    pub theirs: Option<ObjectId>,  // None = this side deleted the path
}

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

pub(crate) fn three_way_diff(
    _ancestor: &BTreeMap<String, ObjectId>,
    _ours: &BTreeMap<String, ObjectId>,
    _theirs: &BTreeMap<String, ObjectId>,
) -> MergeResult {
    todo!()
}

pub(crate) async fn find_common_ancestor(
    _store: &ObjectStore,
    _a: ObjectId,
    _b: ObjectId,
) -> Result<Option<ObjectId>> {
    todo!()
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::ObjectId;

    fn id(b: u8) -> ObjectId { ObjectId::new([b; 32]) }

    fn map(pairs: &[(&str, u8)]) -> BTreeMap<String, ObjectId> {
        pairs.iter().map(|(k, v)| (k.to_string(), id(*v))).collect()
    }

    // three_way_diff tests

    #[test]
    fn both_sides_same_blob_not_a_conflict() {
        let ancestor = map(&[("a.rs", 1)]);
        let ours = map(&[("a.rs", 2)]);
        let theirs = map(&[("a.rs", 2)]);
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["a.rs"], id(2));
    }

    #[test]
    fn one_side_changed_other_unchanged() {
        let ancestor = map(&[("a.rs", 1)]);
        let ours = map(&[("a.rs", 2)]);   // ours changed
        let theirs = map(&[("a.rs", 1)]); // theirs same as ancestor
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["a.rs"], id(2)); // take ours (changed side)
    }

    #[test]
    fn both_sides_changed_differently_is_conflict() {
        let ancestor = map(&[("a.rs", 1)]);
        let ours = map(&[("a.rs", 2)]);
        let theirs = map(&[("a.rs", 3)]);
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].path, "a.rs");
        assert_eq!(r.conflicts[0].ours, Some(id(2)));
        assert_eq!(r.conflicts[0].theirs, Some(id(3)));
        assert!(r.merged.is_empty());
    }

    #[test]
    fn no_common_ancestor_overlapping_paths_are_conflicts() {
        let ancestor = map(&[]);
        let ours = map(&[("a.rs", 1)]);
        let theirs = map(&[("a.rs", 2)]);
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
    }

    #[test]
    fn addition_on_one_side_only() {
        let ancestor = map(&[]);
        let ours = map(&[("new.rs", 5)]);
        let theirs = map(&[]);
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert!(r.is_clean());
        assert_eq!(r.merged["new.rs"], id(5));
    }

    #[test]
    fn clean_deletion_on_one_side() {
        // ours deleted a.rs (kept ancestor version → delete), theirs unchanged
        let ancestor = map(&[("a.rs", 1)]);
        let ours = map(&[]); // deleted
        let theirs = map(&[("a.rs", 1)]); // kept ancestor
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert!(r.is_clean());
        assert!(!r.merged.contains_key("a.rs")); // deletion wins
    }

    #[test]
    fn modify_vs_delete_is_conflict() {
        let ancestor = map(&[("a.rs", 1)]);
        let ours = map(&[("a.rs", 2)]); // modified
        let theirs = map(&[]); // deleted
        let r = three_way_diff(&ancestor, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].ours, Some(id(2)));
        assert_eq!(r.conflicts[0].theirs, None);
    }

    // find_common_ancestor tests (async, need ObjectStore)

    #[tokio::test]
    async fn lca_same_snapshot_is_itself() {
        use crate::store::MemoryBackend;
        let store = ObjectStore::new(MemoryBackend::new());
        let id = ObjectId::new([1u8; 32]);
        let result = find_common_ancestor(&store, id, id).await.unwrap();
        assert_eq!(result, Some(id));
    }

    #[tokio::test]
    async fn lca_linear_chain() {
        use crate::object::Snapshot;
        use crate::store::MemoryBackend;
        let store = ObjectStore::new(MemoryBackend::new());
        let root_tree = ObjectId::new([0u8; 32]);
        // A → B → C; find ancestor of B and C → should be B
        let snap_a = Snapshot { root: root_tree, parents: vec![], author: "t".into(), created_at: 1, message: "A".into() };
        let id_a = store.put_snapshot(snap_a).await.unwrap();
        let snap_b = Snapshot { root: root_tree, parents: vec![id_a], author: "t".into(), created_at: 2, message: "B".into() };
        let id_b = store.put_snapshot(snap_b).await.unwrap();
        let snap_c = Snapshot { root: root_tree, parents: vec![id_b], author: "t".into(), created_at: 3, message: "C".into() };
        let id_c = store.put_snapshot(snap_c).await.unwrap();
        let result = find_common_ancestor(&store, id_b, id_c).await.unwrap();
        assert_eq!(result, Some(id_b));
    }

    #[tokio::test]
    async fn lca_diamond() {
        use crate::object::Snapshot;
        use crate::store::MemoryBackend;
        let store = ObjectStore::new(MemoryBackend::new());
        let root_tree = ObjectId::new([0u8; 32]);
        // A → B → D, A → C → D; find ancestor of B and C → A
        let snap_a = Snapshot { root: root_tree, parents: vec![], author: "t".into(), created_at: 1, message: "A".into() };
        let id_a = store.put_snapshot(snap_a).await.unwrap();
        let snap_b = Snapshot { root: root_tree, parents: vec![id_a], author: "t".into(), created_at: 2, message: "B".into() };
        let id_b = store.put_snapshot(snap_b).await.unwrap();
        let snap_c = Snapshot { root: root_tree, parents: vec![id_a], author: "t".into(), created_at: 3, message: "C".into() };
        let id_c = store.put_snapshot(snap_c).await.unwrap();
        let result = find_common_ancestor(&store, id_b, id_c).await.unwrap();
        assert_eq!(result, Some(id_a));
    }

    #[tokio::test]
    async fn lca_unrelated_returns_none() {
        use crate::object::Snapshot;
        use crate::store::MemoryBackend;
        let store = ObjectStore::new(MemoryBackend::new());
        let root_tree = ObjectId::new([0u8; 32]);
        let snap_a = Snapshot { root: root_tree, parents: vec![], author: "t".into(), created_at: 1, message: "A".into() };
        let id_a = store.put_snapshot(snap_a).await.unwrap();
        let snap_b = Snapshot { root: root_tree, parents: vec![], author: "t".into(), created_at: 2, message: "B".into() };
        let id_b = store.put_snapshot(snap_b).await.unwrap();
        let result = find_common_ancestor(&store, id_a, id_b).await.unwrap();
        assert!(result.is_none());
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
cargo test repo::merge 2>&1 | head -20
```

Expected: tests for `three_way_diff` and `find_common_ancestor` panic at `todo!()`.

- [ ] **Step 5: Implement `three_way_diff`**

Replace the `three_way_diff` stub:

```rust
pub(crate) fn three_way_diff(
    ancestor: &BTreeMap<String, ObjectId>,
    ours: &BTreeMap<String, ObjectId>,
    theirs: &BTreeMap<String, ObjectId>,
) -> MergeResult {
    let all_paths: std::collections::BTreeSet<&str> = ancestor.keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .map(String::as_str)
        .collect();

    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();

    for path in all_paths {
        let a = ancestor.get(path);
        let o = ours.get(path);
        let t = theirs.get(path);

        match (a, o, t) {
            // Both sides agree (same blob or both absent) — including both deleted
            (_, o, t) if o == t => {
                if let Some(id) = o {
                    merged.insert(path.to_owned(), *id);
                }
            }
            // Ours unchanged (kept ancestor blob), theirs deleted → deletion wins
            (Some(a_id), Some(o_id), None) if o_id == a_id => {}
            // Theirs unchanged (kept ancestor blob), ours deleted → deletion wins
            (Some(a_id), None, Some(t_id)) if t_id == a_id => {}
            // Ours modified, theirs deleted → conflict
            (Some(_), Some(o_id), None) => {
                conflicts.push(MergeConflict {
                    path: path.to_owned(),
                    ours: Some(*o_id),
                    theirs: None,
                });
            }
            // Theirs modified, ours deleted → conflict
            (Some(_), None, Some(t_id)) => {
                conflicts.push(MergeConflict {
                    path: path.to_owned(),
                    ours: None,
                    theirs: Some(*t_id),
                });
            }
            // Ours added (not in ancestor), theirs absent → take ours
            (None, Some(o_id), None) => {
                merged.insert(path.to_owned(), *o_id);
            }
            // Theirs added (not in ancestor), ours absent → take theirs
            (None, None, Some(t_id)) => {
                merged.insert(path.to_owned(), *t_id);
            }
            // Both have different blobs (changed differently, or both added differently)
            (_, Some(o_id), Some(t_id)) => {
                conflicts.push(MergeConflict {
                    path: path.to_owned(),
                    ours: Some(*o_id),
                    theirs: Some(*t_id),
                });
            }
            // Both absent (not in union — unreachable, but for exhaustiveness)
            (_, None, None) => {}
        }
    }

    MergeResult { merged, conflicts }
}
```

- [ ] **Step 6: Run `three_way_diff` tests**

```bash
cargo test repo::merge::tests::both_sides 2>&1 | tail -5
cargo test repo::merge::tests 2>&1 | grep "test repo::merge" | head -15
```

Expected: all `three_way_diff` tests pass; `find_common_ancestor` tests still panic at `todo!()`.

- [ ] **Step 7: Implement `find_common_ancestor`**

Replace the `find_common_ancestor` stub:

```rust
pub(crate) async fn find_common_ancestor(
    store: &ObjectStore,
    a: ObjectId,
    b: ObjectId,
) -> Result<Option<ObjectId>> {
    if a == b {
        return Ok(Some(a));
    }

    let mut visited_a: HashSet<ObjectId> = HashSet::from([a]);
    let mut visited_b: HashSet<ObjectId> = HashSet::from([b]);
    let mut frontier_a: VecDeque<ObjectId> = VecDeque::from([a]);
    let mut frontier_b: VecDeque<ObjectId> = VecDeque::from([b]);

    loop {
        let progressed_a = expand_frontier(store, &mut frontier_a, &mut visited_a).await?;
        if let Some(&common) = visited_a.iter().find(|id| visited_b.contains(id)) {
            return Ok(Some(common));
        }

        let progressed_b = expand_frontier(store, &mut frontier_b, &mut visited_b).await?;
        if let Some(&common) = visited_b.iter().find(|id| visited_a.contains(id)) {
            return Ok(Some(common));
        }

        if !progressed_a && !progressed_b {
            break;
        }
    }

    Ok(None)
}

async fn expand_frontier(
    store: &ObjectStore,
    frontier: &mut VecDeque<ObjectId>,
    visited: &mut HashSet<ObjectId>,
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
```

- [ ] **Step 8: Add re-exports to `src/lib.rs`**

After `pub use repo::{FilteredSnapshot, MergeCheck};`:

```rust
// <bead-id>
pub use repo::merge::{MergeConflict, MergeResult};
```

- [ ] **Step 9: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests plus all 11 new `merge` tests pass.

- [ ] **Step 10: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean. Common warning: unused `import VecDeque` if you put it in `use` but also import `HashSet`. Check that all imports at top of `src/repo/merge.rs` are used.

- [ ] **Step 11: Commit**

```bash
git add src/repo/merge.rs src/repo/mod.rs src/lib.rs
git commit -m "feat(repo): add MergeConflict, MergeResult, three_way_diff, find_common_ancestor"
```

- [ ] **Step 12: Merge and close**

```bash
git checkout master && git merge bole-def
git branch -d bole-def
bd close bole-def
```

---

## Task 3: Repository methods — find_common_ancestor, merge_timelines, advance_timeline, prune_timeline

**Files:**
- Modify: `src/repo/mod.rs`

**Interfaces:**
- Consumes (from Tasks 1+2):
  - `Timeline { kind: String, expires_at: Option<u64>, head: ObjectId, ... }`
  - `Accessor::privileged() -> Self`
  - `merge::find_common_ancestor(store, a, b) -> Result<Option<ObjectId>>`
  - `merge::three_way_diff(ancestor, ours, theirs) -> MergeResult`
  - `merge::{MergeConflict, MergeResult}`
  - `refs.advance_head(name, new_head) -> Result<()>`
  - `refs.delete_ref(name) -> Result<()>`
  - `refs.list(prefix) -> Result<Vec<RefName>>`
  - `refs.get(name) -> Result<Option<Ref>>`
  - `refs.get_timeline(name) -> Result<Option<Timeline>>`
  - `walk_tree_filtered(objects, acls, tree_id, prefix, accessor, out) -> Result<()>` (module-private fn, already in scope)
- Produces:
  - `pub async fn Repository::find_common_ancestor(&self, a: ObjectId, b: ObjectId) -> Result<Option<ObjectId>>`
  - `pub async fn Repository::merge_timelines(&self, source: &RefName, target: &RefName, accessor: &Accessor) -> Result<MergeResult>`
  - `pub async fn Repository::advance_timeline(&self, name: &RefName, snapshot_id: ObjectId, accessor: &Accessor) -> Result<()>`
  - `pub fn Repository::prune_timeline(&self, name: &RefName, now: u64) -> Result<bool>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 6 T3: Repository find_common_ancestor, merge_timelines, advance_timeline, prune_timeline" \
  --description="Add four methods to impl Repository: find_common_ancestor delegates to merge::find_common_ancestor; merge_timelines does write-cap check + LCA + three-way diff; advance_timeline enforces write caps on timeline and paths; prune_timeline removes expired timelines with no tags pointing to head." \
  --type=task --priority=2
bd update bole-ghi --claim
git checkout -b bole-ghi
```

- [ ] **Step 2: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/repo/mod.rs`:

```rust
    // <bead-id>
    async fn repo_with_two_timelines() -> (
        Repository,
        RefName,
        RefName,
        ObjectId, // common ancestor snap
        ObjectId, // source head snap
        ObjectId, // target head snap
    ) {
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        let repo = Repository::memory();
        // Common ancestor: a.rs=1, b.rs=1
        let blob1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
        let blob2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
        let blob3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
        let mut e0 = BTreeMap::new();
        e0.insert("a.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        e0.insert("b.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        let tree0 = repo.objects.put_tree(e0).await.unwrap();
        let snap0 = repo.objects.put_snapshot(Snapshot {
            root: tree0, parents: vec![], author: "t".into(), created_at: 1, message: "base".into(),
        }).await.unwrap();
        // Source: changes a.rs
        let mut e1 = BTreeMap::new();
        e1.insert("a.rs".into(), TreeEntry { id: blob2, kind: EntryKind::Blob });
        e1.insert("b.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        let tree1 = repo.objects.put_tree(e1).await.unwrap();
        let snap1 = repo.objects.put_snapshot(Snapshot {
            root: tree1, parents: vec![snap0], author: "agent-a".into(), created_at: 2, message: "change a.rs".into(),
        }).await.unwrap();
        // Target: changes b.rs
        let mut e2 = BTreeMap::new();
        e2.insert("a.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        e2.insert("b.rs".into(), TreeEntry { id: blob3, kind: EntryKind::Blob });
        let tree2 = repo.objects.put_tree(e2).await.unwrap();
        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: tree2, parents: vec![snap0], author: "agent-b".into(), created_at: 3, message: "change b.rs".into(),
        }).await.unwrap();
        let source = RefName::new("agent/source").unwrap();
        let target = RefName::new("agent/target").unwrap();
        repo.refs.create_timeline(source.clone(), snap1, TimelinePolicy::Unrestricted, 2, "ephemeral".into(), None).unwrap();
        repo.refs.create_timeline(target.clone(), snap2, TimelinePolicy::Unrestricted, 3, "ephemeral".into(), None).unwrap();
        (repo, source, target, snap0, snap1, snap2)
    }

    #[tokio::test]
    async fn merge_non_conflicting_timelines() {
        use crate::acl::{Accessor, PathRole, Permission, TimelineRole};
        use crate::refs::RefName;
        let (repo, source, target, _ancestor, _s1, _s2) = repo_with_two_timelines().await;
        let accessor = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });
        let result = repo.merge_timelines(&source, &target, &accessor).await.unwrap();
        assert!(result.is_clean(), "expected no conflicts, got: {:?}", result.conflicts);
        assert!(result.merged.contains_key("a.rs"));
        assert!(result.merged.contains_key("b.rs"));
    }

    #[tokio::test]
    async fn merge_conflicting_timelines() {
        use crate::acl::{Accessor, PathRole, Permission, TimelineRole};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        let repo = Repository::memory();
        // Common ancestor: shared.rs=1
        let v1 = repo.objects.put_blob(Bytes::from("original")).await.unwrap();
        let v2 = repo.objects.put_blob(Bytes::from("version-a")).await.unwrap();
        let v3 = repo.objects.put_blob(Bytes::from("version-b")).await.unwrap();
        let mut e0 = BTreeMap::new();
        e0.insert("shared.rs".into(), TreeEntry { id: v1, kind: EntryKind::Blob });
        let tree0 = repo.objects.put_tree(e0).await.unwrap();
        let snap0 = repo.objects.put_snapshot(Snapshot {
            root: tree0, parents: vec![], author: "t".into(), created_at: 1, message: "base".into(),
        }).await.unwrap();
        // Both sides changed shared.rs to different blobs
        let mut e1 = BTreeMap::new();
        e1.insert("shared.rs".into(), TreeEntry { id: v2, kind: EntryKind::Blob });
        let tree1 = repo.objects.put_tree(e1).await.unwrap();
        let snap1 = repo.objects.put_snapshot(Snapshot {
            root: tree1, parents: vec![snap0], author: "a".into(), created_at: 2, message: "a".into(),
        }).await.unwrap();
        let mut e2 = BTreeMap::new();
        e2.insert("shared.rs".into(), TreeEntry { id: v3, kind: EntryKind::Blob });
        let tree2 = repo.objects.put_tree(e2).await.unwrap();
        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: tree2, parents: vec![snap0], author: "b".into(), created_at: 3, message: "b".into(),
        }).await.unwrap();
        let source = RefName::new("src-tl").unwrap();
        let target = RefName::new("tgt-tl").unwrap();
        repo.refs.create_timeline(source.clone(), snap1, TimelinePolicy::Unrestricted, 2, "ephemeral".into(), None).unwrap();
        repo.refs.create_timeline(target.clone(), snap2, TimelinePolicy::Unrestricted, 3, "ephemeral".into(), None).unwrap();
        let accessor = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });
        let result = repo.merge_timelines(&source, &target, &accessor).await.unwrap();
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, "shared.rs");
        assert_eq!(result.conflicts[0].ours, Some(v3));   // target's blob
        assert_eq!(result.conflicts[0].theirs, Some(v2)); // source's blob
    }

    #[tokio::test]
    async fn advance_timeline_write_role_succeeds() {
        use crate::acl::{Accessor, PathRole, Permission, TimelineRole};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        let repo = Repository::memory();
        let blob = repo.objects.put_blob(Bytes::from("code")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        let name = RefName::new("agent/main").unwrap();
        repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), None).unwrap();
        let snap2_tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: snap2_tree, parents: vec![snap], author: "t".into(), created_at: 2, message: "m2".into(),
        }).await.unwrap();
        let accessor = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "agent/**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });
        repo.advance_timeline(&name, snap2, &accessor).await.unwrap();
        let head = repo.refs.get_timeline(&name).unwrap().unwrap().head;
        assert_eq!(head, snap2);
    }

    #[tokio::test]
    async fn advance_timeline_without_timeline_write_role_fails() {
        use crate::acl::Accessor;
        use crate::object::{Snapshot};
        use crate::refs::{RefName, TimelinePolicy};
        let repo = Repository::memory();
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        let name = RefName::new("protected").unwrap();
        repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), None).unwrap();
        let err = repo.advance_timeline(&name, snap, &Accessor::new()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)));
    }

    #[tokio::test]
    async fn advance_timeline_without_path_write_role_fails() {
        use crate::acl::{Accessor, PathRole, Permission, TimelineRole};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        let repo = Repository::memory();
        let blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("secrets/prod.key".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        let name = RefName::new("agent/main").unwrap();
        repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), None).unwrap();
        // Accessor can write timeline but only src/** paths
        let accessor = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write });
        let err = repo.advance_timeline(&name, snap, &accessor).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)));
    }

    #[tokio::test]
    fn prune_expired_timeline_no_tag_returns_true() {
        use crate::refs::{RefName, TimelinePolicy};
        use crate::object::ObjectId;
        let repo = Repository::memory();
        let id = ObjectId::new([9u8; 32]);
        let name = RefName::new("ephemeral/session-1").unwrap();
        repo.refs.create_timeline(name.clone(), id, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), Some(100)).unwrap();
        let pruned = repo.prune_timeline(&name, 200).unwrap();
        assert!(pruned);
        assert!(repo.refs.get_timeline(&name).unwrap().is_none());
    }

    #[tokio::test]
    async fn prune_expired_timeline_with_tag_on_head_returns_false() {
        use crate::refs::{RefName, TimelinePolicy};
        use crate::object::ObjectId;
        let repo = Repository::memory();
        let id = ObjectId::new([9u8; 32]);
        let name = RefName::new("ephemeral/session-2").unwrap();
        repo.refs.create_timeline(name.clone(), id, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), Some(100)).unwrap();
        // Tag points to the timeline's head — promotes it
        repo.refs.create_tag(RefName::new("v1.0").unwrap(), id, None, 1).unwrap();
        let pruned = repo.prune_timeline(&name, 200).unwrap();
        assert!(!pruned); // kept because tagged
        assert!(repo.refs.get_timeline(&name).unwrap().is_some());
    }

    #[test]
    fn prune_not_expired_returns_false() {
        use crate::refs::{RefName, TimelinePolicy};
        use crate::object::ObjectId;
        let repo = Repository::memory();
        let id = ObjectId::new([9u8; 32]);
        let name = RefName::new("ephemeral/session-3").unwrap();
        repo.refs.create_timeline(name.clone(), id, TimelinePolicy::Unrestricted, 1, "ephemeral".into(), Some(9999)).unwrap();
        let pruned = repo.prune_timeline(&name, 100).unwrap();
        assert!(!pruned); // not yet expired
    }

    #[test]
    fn prune_no_expiry_returns_false() {
        use crate::refs::{RefName, TimelinePolicy};
        use crate::object::ObjectId;
        let repo = Repository::memory();
        let id = ObjectId::new([8u8; 32]);
        let name = RefName::new("release/stable").unwrap();
        repo.refs.create_timeline(name.clone(), id, TimelinePolicy::Unrestricted, 1, "release".into(), None).unwrap();
        let pruned = repo.prune_timeline(&name, u64::MAX).unwrap();
        assert!(!pruned); // no expiry = never pruned
    }
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test "merge_non_conflicting\|merge_conflicting\|advance_timeline\|prune" 2>&1 | head -20
```

Expected: compile errors — methods not found on `Repository`.

- [ ] **Step 4: Implement the four Repository methods**

In `src/repo/mod.rs`, add to `impl Repository` (after the existing `compute_workspace_view` method). Add the necessary imports at the top of the file too. Make sure `merge` is imported:

At the top of `src/repo/mod.rs`, in the existing use block, add (under a new bead tag):

```rust
// <bead-id>
use merge::{MergeResult, three_way_diff, find_common_ancestor as lca};
use crate::refs::Ref;
```

Then add to `impl Repository`:

```rust
    // <bead-id>
    pub async fn find_common_ancestor(
        &self,
        a: ObjectId,
        b: ObjectId,
    ) -> Result<Option<ObjectId>> {
        lca(&self.objects, a, b).await
    }

    pub async fn merge_timelines(
        &self,
        source: &RefName,
        target: &RefName,
        accessor: &Accessor,
    ) -> Result<MergeResult> {
        if !accessor.can_write_timeline(target.as_str()) {
            return Err(Error::AccessDenied(format!(
                "cannot write timeline: {}", target.as_str()
            )));
        }
        let source_head = self.refs.get_timeline(source)?
            .ok_or_else(|| Error::Storage(format!("timeline not found: {}", source.as_str())))?
            .head;
        let target_head = self.refs.get_timeline(target)?
            .ok_or_else(|| Error::Storage(format!("timeline not found: {}", target.as_str())))?
            .head;

        let ancestor_tree = match lca(&self.objects, source_head, target_head).await? {
            Some(ancestor_id) => {
                match self.objects.get(&ancestor_id).await? {
                    Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?,
                    _ => BTreeMap::new(),
                }
            }
            None => BTreeMap::new(),
        };

        let source_tree = match self.objects.get(&source_head).await? {
            Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?,
            _ => BTreeMap::new(),
        };
        let target_tree = match self.objects.get(&target_head).await? {
            Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?,
            _ => BTreeMap::new(),
        };

        Ok(three_way_diff(&ancestor_tree, &target_tree, &source_tree))
    }

    pub async fn advance_timeline(
        &self,
        name: &RefName,
        snapshot_id: ObjectId,
        accessor: &Accessor,
    ) -> Result<()> {
        if !accessor.can_write_timeline(name.as_str()) {
            return Err(Error::AccessDenied(format!(
                "cannot write timeline: {}", name.as_str()
            )));
        }
        let snap = match self.objects.get(&snapshot_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Err(Error::Storage(format!(
                "snapshot not found: {}", snapshot_id
            ))),
        };
        let mut all_paths = BTreeMap::new();
        walk_tree_filtered(
            &self.objects, &self.acls, snap.root, "",
            &Accessor::privileged(), &mut all_paths,
        ).await?;
        for path in all_paths.keys() {
            if !accessor.can_write_path(path) {
                return Err(Error::AccessDenied(format!(
                    "cannot write path: {}", path
                )));
            }
        }
        self.refs.advance_head(name, snapshot_id)
    }

    pub fn prune_timeline(&self, name: &RefName, now: u64) -> Result<bool> {
        let tl = match self.refs.get_timeline(name)? {
            Some(t) => t,
            None => return Ok(false),
        };
        match tl.expires_at {
            None => return Ok(false),
            Some(exp) if exp > now => return Ok(false),
            _ => {}
        }
        // Check if any tag points to this timeline's head (promoted)
        let all_refs = self.refs.list("")?;
        for ref_name in all_refs {
            if let Some(Ref::Tag(tag)) = self.refs.get(&ref_name)? {
                if tag.target == tl.head {
                    return Ok(false); // promoted — survives
                }
            }
        }
        self.refs.delete_ref(name)?;
        Ok(true)
    }

    async fn tree_as_map(&self, tree_id: ObjectId) -> Result<BTreeMap<String, ObjectId>> {
        let mut out = BTreeMap::new();
        walk_tree_filtered(
            &self.objects, &self.acls, tree_id, "",
            &Accessor::privileged(), &mut out,
        ).await?;
        Ok(out)
    }
```

Note: `tree_as_map` is a private helper — no bead tag needed (it's part of the same block of new code; include it under the same `// <bead-id>` comment block as the methods above, since it's a contiguous addition).

Also add `Accessor::privileged` to the existing import that brings in `Accessor` at the top of `src/repo/mod.rs`.

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests plus the 8 new Repository tests pass.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean. If clippy warns about `async fn prune_timeline` not being async (it isn't), that's fine — the method signature is `pub fn`, not `pub async fn`.

- [ ] **Step 7: Commit**

```bash
git add src/repo/mod.rs
git commit -m "feat(repo): add find_common_ancestor, merge_timelines, advance_timeline, prune_timeline"
```

- [ ] **Step 8: Merge and close**

```bash
git checkout master && git merge bole-ghi
git branch -d bole-ghi
bd close bole-ghi
```

---

## Task 4: T6 Integration Tests

**Files:**
- Create: `tests/multi_actor.rs`

**Interfaces:**
- Consumes all public APIs from Tasks 1–3:
  - `bole::{Repository, ObjectId, MergeConflict, MergeResult}`
  - `bole::{Accessor, PathRole, TimelineRole, Permission}`
  - `bole::object::{EntryKind, Snapshot, TreeEntry}`
  - `bole::refs::{RefName, TimelinePolicy}`
  - `bole::Error`
  - `bytes::Bytes`
  - `std::collections::BTreeMap`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 6 T4: T6 integration tests" \
  --description="Create tests/multi_actor.rs with t6_merge_non_conflicting, t6_merge_conflict, t6_agent_capability_enforced, t6_ephemeral_prune integration tests." \
  --type=task --priority=2
bd update bole-jkl --claim
git checkout -b bole-jkl
```

- [ ] **Step 2: Create `tests/multi_actor.rs`**

```rust
// <bead-id>
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, Error, MergeResult, PathRole, Permission, Repository, TimelineRole};
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
```

- [ ] **Step 3: Run T6 tests**

```bash
cargo test --test multi_actor 2>&1 | tail -20
```

Expected: all 4 T6 tests pass.

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1 | tail -15
```

Expected: all tests pass.

- [ ] **Step 5: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tests/multi_actor.rs
git commit -m "test(multi_actor): add T6 integration tests"
```

- [ ] **Step 7: Merge and close**

```bash
git checkout master && git merge bole-jkl
git branch -d bole-jkl
bd close bole-jkl
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|---|---|
| `Timeline { kind: String, expires_at: Option<u64> }` with serde defaults | Task 1 |
| `create_timeline` accepts `kind` and `expires_at` | Task 1 |
| `Accessor::privileged()` — read all paths and timelines | Task 1 |
| `MergeConflict { path, ours: Option<ObjectId>, theirs: Option<ObjectId> }` | Task 2 |
| `MergeResult { merged, conflicts }` + `is_clean()` | Task 2 |
| `three_way_diff` — all 10 case combinations from spec | Task 2 |
| `find_common_ancestor` — full BFS with visited set, handles linear/diamond/unrelated | Task 2 |
| `pub use repo::merge::{MergeConflict, MergeResult}` in lib.rs | Task 2 |
| `Repository::find_common_ancestor` | Task 3 |
| `Repository::merge_timelines` — write cap check, LCA, three-way diff | Task 3 |
| `Repository::advance_timeline` — timeline write cap + path write cap | Task 3 |
| `Repository::prune_timeline` — expires_at check, tag scan for promotion | Task 3 |
| `t6_merge_non_conflicting` | Task 4 |
| `t6_merge_conflict` with caller resolution + advance | Task 4 |
| `t6_agent_capability_enforced` — AccessDenied on secrets/** | Task 4 |
| `t6_ephemeral_prune` — pruned when expired, kept when tagged | Task 4 |

**Placeholder scan:** None found.

**Type consistency:**
- `MergeConflict.ours: Option<ObjectId>` — used in Task 2 definition, Task 2 tests, Task 3 `merge_timelines`, Task 4 `t6_merge_conflict` assertion ✓
- `create_timeline(..., kind: String, expires_at: Option<u64>)` — all callsites updated in Task 1, all Task 3/4 uses pass explicit kind/expires_at ✓
- `Accessor::privileged()` — defined Task 1, used in Task 3 (`walk_tree_filtered` call in `advance_timeline` and `tree_as_map`) ✓
- `three_way_diff(ancestor, ours, theirs)` — Task 3 calls as `three_way_diff(&ancestor_tree, &target_tree, &source_tree)` — `ours` = target (the timeline being merged *into*), `theirs` = source ✓
- `refs.advance_head(name, snapshot_id)` — correct method name from existing RefStore API ✓
- `refs.get(name) -> Result<Option<Ref>>` — used in `prune_timeline` for tag scan ✓
