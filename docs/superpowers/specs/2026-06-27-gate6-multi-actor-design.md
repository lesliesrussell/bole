# Gate 6: Multi-Actor / Agents

**Project:** bole — a next-generation version control system
**Language:** Rust (async, tokio)
**Date:** 2026-06-27
**Spec ref:** spec.md Gate 6, Test T6

---

## Context

Gates 1–5 delivered content-addressed object storage, mutable references, granular ACL enforcement, secrets and env overlays, and pluggable backends. Gate 6 adds multi-actor semantics: multiple concurrent writers on isolated timelines, deterministic three-way merge with conflict surfacing, timeline retention via TTL and tag-based promotion, and write capability enforcement on `Repository` operations.

Key architectural decisions:

- **Three-way merge, not LWW** — the library detects conflicts; callers resolve them. Last-writer-wins is silent data loss and not offered as a first-class strategy. A `MergeResult { merged, conflicts }` gives callers full control over resolution.
- **Full DAG BFS for LCA** — ancestor search walks the full `parents` graph with a visited set for cycle safety. No depth limit; depth limits trade a slow edge case for a silently-wrong one.
- **`kind: String` + `expires_at: Option<u64>` on `Timeline`** — retention is controlled by a Unix timestamp; `kind` is a free-form label for caller conventions (`"ephemeral"`, `"review"`, `"release"`). No enum needed — the label communicates intent, the timestamp enforces it.
- **A tag is promotion** — a snapshot with any tag pointing to it survives timeline pruning. No new "promoted" flag or set; the existing tag system is the signal.
- **Methods on `Repository`** — all new logic follows the established pattern (`get_snapshot_filtered`, `compute_workspace_view`, `check_merge`). New merge types live in `src/repo/merge.rs`.

---

## New Types

### `Timeline` extension

Add two fields to `src/refs/timeline.rs`:

```rust
pub struct Timeline {
    pub head: ObjectId,
    pub policy: TimelinePolicy,
    pub created_at: u64,
    pub kind: String,             // "ephemeral" | "review" | "release" | caller-defined
    pub expires_at: Option<u64>,  // Unix seconds; None = never expires
}
```

`kind` defaults to `"persistent"` when not specified. `expires_at: None` means the timeline never expires.

### `MergeConflict` and `MergeResult`

New in `src/repo/merge.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct MergeConflict {
    pub path: String,
    pub ours: ObjectId,    // target timeline's blob at this path
    pub theirs: ObjectId,  // source timeline's blob at this path
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub merged: BTreeMap<String, ObjectId>, // resolved paths (non-conflicting)
    pub conflicts: Vec<MergeConflict>,      // caller must resolve these
}
```

`MergeResult::is_clean()` is a convenience method returning `self.conflicts.is_empty()`.

### `Accessor::privileged()`

New constructor on `Accessor` in `src/acl/mod.rs`:

```rust
pub fn privileged() -> Self {
    Self::new()
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
}
```

Used internally by `prune_timeline` to scan all tags without caller-supplied access context.

---

## Repository Methods

All new methods on `impl Repository` in `src/repo/mod.rs`. Merge logic helpers in `src/repo/merge.rs`.

### `find_common_ancestor`

```rust
pub async fn find_common_ancestor(
    &self,
    a: ObjectId,
    b: ObjectId,
) -> Result<Option<ObjectId>>;
```

BFS from both `a` and `b` simultaneously over the `parents` DAG. Maintains two `HashSet<ObjectId>` (visited from each side). Each BFS step expands one frontier level; after each expansion, intersects the two visited sets. Returns the first common member, or `None` if both frontiers exhaust without intersection. A `visited` set on each side prevents cycles.

### `merge_timelines`

```rust
pub async fn merge_timelines(
    &self,
    source: &RefName,
    target: &RefName,
    accessor: &Accessor,
) -> Result<MergeResult>;
```

Algorithm:
1. Check `accessor.can_write_timeline(target.as_str())` → `Err(AccessDenied)` if false
2. Resolve `source` and `target` refs to snapshot `ObjectId`s via `RefStore`
3. Call `find_common_ancestor(source_head, target_head)` → `ancestor_opt`
4. Load all three trees as `BTreeMap<String, ObjectId>`:
   - `ancestor_tree`: from `ancestor_opt` (empty map if `None`)
   - `source_tree`: from source head snapshot
   - `target_tree`: from target head snapshot
5. Three-way diff over the union of all path keys:
   - Path added on one side (not in ancestor, not in the other side): take the addition
   - Path added on both sides with the same blob: take it (not a conflict)
   - Path added on both sides with different blobs: emit `MergeConflict`
   - Path in ancestor, unchanged on both sides: keep it
   - Path in ancestor, changed on one side only: take the changed side
   - Path in ancestor, deleted on one side, unchanged on the other: remove it (deletion wins)
   - Path in ancestor, deleted on one side, modified on the other: emit `MergeConflict`
   - Path in ancestor, deleted on both sides: remove it
   - Path in ancestor, changed on both sides to the same `ObjectId`: keep it (not a conflict)
   - Path in ancestor, changed on both sides to different `ObjectId`s: emit `MergeConflict`
6. Return `MergeResult { merged, conflicts }`

`merge_timelines` does **not** automatically advance the target timeline's head. The caller examines `conflicts`, resolves them (by choosing a side, inserting a new blob, or aborting), then calls `advance_timeline` with the final resolved snapshot.

### `advance_timeline`

```rust
pub async fn advance_timeline(
    &self,
    name: &RefName,
    snapshot_id: ObjectId,
    accessor: &Accessor,
) -> Result<()>;
```

Algorithm:
1. Check `accessor.can_write_timeline(name.as_str())` → `Err(AccessDenied)` if false
2. Resolve `snapshot_id` to its `Snapshot`, extract `root` tree
3. Walk all paths in the tree (reusing `walk_tree_filtered` with a privileged accessor to enumerate all paths)
4. For each path: `accessor.can_write_path(path)` → `Err(AccessDenied)` on first failure
5. Update the timeline's head via `refs.advance_head(name, snapshot_id)`

### `prune_timeline`

```rust
pub fn prune_timeline(
    &self,
    name: &RefName,
    now: u64,
) -> Result<bool>; // true = pruned, false = kept
```

Algorithm:
1. Fetch the timeline via `RefStore`; if not found, return `Ok(false)`
2. If `expires_at.is_none() || expires_at > now`: return `Ok(false)`
3. List all tags in the ref store (using `Accessor::privileged()`)
4. If any tag's target equals the timeline's `head`: return `Ok(false)` (promoted — survives)
5. Delete the timeline ref; return `Ok(true)`

---

## Error Handling

No new error variants. Existing variants cover all failure cases:

- `Error::AccessDenied(String)` — write capability check failed
- `Error::Storage(String)` — ref or snapshot not found during merge/prune
- `Error::Codec(String)` — malformed object during DAG walk

---

## Crate Structure Changes

```
src/
├── refs/
│   └── timeline.rs      # add kind: String, expires_at: Option<u64>
├── acl/
│   └── mod.rs           # add Accessor::privileged()
├── repo/
│   ├── mod.rs           # add find_common_ancestor, merge_timelines,
│   │                    #     advance_timeline, prune_timeline
│   └── merge.rs         # MergeConflict, MergeResult, three-way diff logic
└── lib.rs               # re-export MergeConflict, MergeResult

tests/
└── multi_actor.rs       # T6 integration tests
```

---

## lib.rs Re-exports

```rust
pub use repo::merge::{MergeConflict, MergeResult};
```

---

## Testing Approach

### Unit tests

**`src/repo/merge.rs` (in-module):**
- `lca_linear_chain` — A→B→C: ancestor of B and C is B
- `lca_diamond` — A→B→D, A→C→D: ancestor of B and C is A
- `lca_unrelated` — two snapshots with no shared parent: `None`
- `three_way_one_side_changed` — only source changed a path → non-conflicting, source wins
- `three_way_both_same_blob` — both sides changed same path to same `ObjectId` → non-conflicting
- `three_way_conflict` — both sides changed same path to different blobs → `MergeConflict`
- `three_way_no_ancestor` — no common ancestor → overlapping paths are all conflicts

**`src/repo/mod.rs` (in-module):**
- `prune_expired_no_tag` — expired timeline, no tags → pruned (`true`)
- `prune_expired_with_tag` — expired timeline, tag points to head → kept (`false`)
- `prune_not_expired` — `expires_at > now` → kept (`false`)
- `advance_write_role_succeeds` — accessor with timeline + path write roles → `Ok(())`
- `advance_no_timeline_role` — accessor missing timeline write role → `Err(AccessDenied)`
- `advance_no_path_role` — timeline write granted but path write denied → `Err(AccessDenied)`

### T6 integration tests (`tests/multi_actor.rs`)

**`t6_merge_non_conflicting`**
- Build snapshot S0 (common ancestor) with `src/app.rs` and `src/config.rs`
- Agent A: create timeline `agent/a`, advance head to S1 (only `src/app.rs` changed)
- Agent B: create timeline `agent/b`, advance head to S2 (only `src/config.rs` changed)
- `merge_timelines("agent/a", "agent/b", &privileged_accessor)` → `MergeResult` with no conflicts
- `merged` contains both updated paths

**`t6_merge_conflict`**
- Build S0 with `src/shared.rs`
- Agent A: S1 with `src/shared.rs` → blob `0xAA`
- Agent B: S2 with `src/shared.rs` → blob `0xBB`
- `merge_timelines` → `MergeResult` with one `MergeConflict { path: "src/shared.rs", ours: 0xBB, theirs: 0xAA }`
- Caller resolves by taking `theirs`; calls `advance_timeline` with resolved snapshot → succeeds

**`t6_agent_capability_enforced`**
- Agent accessor: write role on `src/**` timeline and `src/**` paths only
- Build snapshot touching `secrets/prod.key`
- `advance_timeline("src/agent", snapshot_id, &agent_accessor)` → `Err(AccessDenied)`

**`t6_ephemeral_prune`**
- Create timeline with `kind = "ephemeral"`, `expires_at = Some(1)`
- `prune_timeline(name, 2)` → `true` (pruned)
- Re-create same timeline, add a tag pointing to its head
- `prune_timeline(name, 2)` → `false` (tag = promoted, survives)

---

## Out of Scope (Gate 6)

- Sub-file (line-level) diff during merge — bole merges at blob granularity; file-level diffing is the caller's responsibility
- Conflict markers or auto-resolution strategies — `MergeResult` surfaces conflicts; resolution is caller code
- Multi-timeline octopus merges (more than two timelines at once)
- GC / unreachable object collection after timeline pruning
- Audit logging of agent-initiated state transitions (non-functional requirement, deferred)
- Streaming or incremental merge for very large trees
