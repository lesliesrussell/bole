# Gate 7: Git Projection

**Project:** bole — a next-generation version control system
**Language:** Rust (async, tokio)
**Date:** 2026-06-27
**Spec ref:** spec.md Gate 7 (G7), Test T7

---

## Context

Gates 1–6 delivered content-addressed object storage, mutable references, granular ACL enforcement, secrets and env overlays, pluggable backends, and multi-actor merge semantics. Gate 7 adds backward-compatible export: project a bole repository's full object graph into a standard bare git repository so existing git tooling (`git log`, `git diff`, `git blame`, etc.) can read bole history without modification.

Key architectural decisions:

- **`gix` crate (pure Rust)** — write git objects and refs using the `gix` ecosystem (the same library used by `cargo`). No external process dependency; no native libgit2. Self-contained.
- **One-shot projection** — `project_to_git` writes the full history every call. No persistent bole→git mapping table. Simple, correct, and easy to reason about. Re-running produces a fresh consistent export.
- **All timelines at once** — one call produces all branches and tags. An in-memory `HashMap<ObjectId, gix_hash::ObjectId>` deduplicates shared ancestor snapshots within a single run so common commits are written exactly once.
- **Caller-supplied `Accessor`** — same pattern as all other `Repository` methods. Private paths and private timelines are excluded from the projection. "Leakage by projection" is impossible when the accessor is correctly scoped.
- **`Secret` / `EnvOverlay` silently skipped** — tree entries whose ObjectId resolves to a non-Blob/non-Tree object are omitted from the projected git tree. No error, no tombstone.
- **Synthetic git identity** — git commits require `Name <email> timestamp +timezone`. Bole `Snapshot` provides `author: String` and `created_at: u64`. Projected commits use `{author} <bole@local> {created_at} +0000` for both author and committer fields.

---

## API

### New function

```rust
// src/repo/git_projection.rs
pub async fn project_to_git(
    repo: &Repository,
    target_path: &std::path::Path,
    accessor: &Accessor,
) -> Result<()>
```

Re-exported from `src/lib.rs`:

```rust
pub use repo::git_projection::project_to_git;
```

### New error variant

```rust
// src/error.rs
#[error("git projection failed: {0}")]
GitProjection(String),
```

All `gix` errors are converted via `.map_err(|e| Error::GitProjection(e.to_string()))`.

---

## Algorithm

`project_to_git` executes five sequential passes:

### Pass 1: Initialize target

```rust
let git_repo = gix::init::bare(target_path)
    .or_else(|_| gix::open(target_path))
    .map_err(|e| Error::GitProjection(e.to_string()))?;
```

Creates a bare git repository at `target_path`. If the directory already exists and contains a valid git repo, opens it instead (idempotent). If the directory exists but is not a git repo, returns `Err(GitProjection(...))`.

### Pass 2: Collect reachable snapshots (topological sort)

1. List all refs via `repo.refs.list("")?`
2. Filter to `Ref::Timeline` entries where `accessor.can_read_timeline(name)` is true
3. DFS from each timeline head through `Snapshot.parents`, using a `HashSet<ObjectId>` as a visited set to handle shared ancestry and prevent cycles
4. Accumulate snapshots in **post-order** (a snapshot is appended only after all its parents have been appended) — this produces a topological ordering where parents always precede children

### Pass 3: Write objects

Iterate the topo-sorted snapshot list. For each snapshot, maintain an in-memory dedup map:

```rust
let mut id_map: HashMap<ObjectId, gix_hash::ObjectId> = HashMap::new();
```

**Tree conversion:**

Use the existing module-private `walk_tree_filtered(objects, acls, tree_id, "", accessor, &mut out)` to get a flat `BTreeMap<String, ObjectId>` of all ACL-visible paths for the snapshot. This reuses the battle-tested ACL logic (unprotected paths always included, protected paths gated by `accessor.can_read_path`).

Then convert the flat map to nested git tree objects bottom-up:
1. For each leaf path, fetch the object; if it resolves to `Object::Secret` or `Object::EnvOverlay` — skip silently; otherwise write a git blob and record the SHA.
2. Group paths by their directory components. Write git tree objects from deepest to shallowest, each referencing child blob/tree SHAs already written.
3. Return the root git tree SHA.

**Commit conversion**:

```rust
let identity = format!(
    "{} <bole@local> {} +0000",
    if snapshot.author.is_empty() { "bole" } else { &snapshot.author },
    snapshot.created_at,
);
```

Write a git commit object with:
- `tree`: git tree SHA from converted root
- `parent`: git SHAs of each `snapshot.parents` entry, looked up from `id_map`
- `author` and `committer`: `identity` (identical)
- `message`: `snapshot.message`

Insert `bole_snapshot_id → git_commit_sha` into `id_map`.

### Pass 4: Write branch refs

For each projected timeline, write:

```
refs/heads/{timeline_name} → git_commit_sha
```

`timeline_name` is the `RefName` string verbatim — it is already a valid git ref path component.

### Pass 5: Write tag refs

List all refs via `repo.refs.list("")?`, filter to `Ref::Tag` entries. For each tag where `tag.target` is present in `id_map`, write:

```
refs/tags/{tag_name} → git_commit_sha (lightweight tag)
```

Tags whose target is not in `id_map` (e.g., they point to a snapshot on a non-projected private timeline) are silently skipped.

---

## Object Mapping

| Bole object | Git object | Mode / notes |
|---|---|---|
| `Blob` bytes | git blob | raw bytes unchanged |
| `Tree { entries }` | git tree | one git tree per bole tree |
| `Snapshot` | git commit | author/committer from synthetic identity |
| `Tag.target` | git lightweight tag ref | `refs/tags/{name}` |
| `Timeline.head` | git branch ref | `refs/heads/{name}` |
| `Secret` | — | silently skipped (entry removed from tree) |
| `EnvOverlay` | — | silently skipped |
| ACL-denied path | — | silently skipped |
| ACL-denied timeline | — | no branch ref written |

**Git tree entry modes:**
- Regular file: `100644`
- Subtree: `040000`

**Git commit identity:**
```
{snapshot.author} <bole@local> {snapshot.created_at} +0000
```
Both `author` and `committer` use this identical value. If `snapshot.author` is empty, use `bole` as the name.

**RefName → git ref name:** verbatim (e.g., `agent/main` → `refs/heads/agent/main`).

---

## New Dependency

```toml
# Cargo.toml [dependencies]
gix = { version = "0.70", default-features = false, features = ["max-performance-safe"] }
```

The `gix` crate is pure Rust and used in production by `cargo`. Exact feature set to be confirmed by implementer — minimum required: object writing (`gix-odb`), hash types (`gix-hash`), ref writing (`gix-ref`), repo initialization (`gix-init`). The `max-performance-safe` preset enables these without unsafe code.

---

## Error Handling

No new error scenarios beyond the existing `Error` variants:

| Condition | Error |
|---|---|
| `target_path` exists but is not a git repo | `Error::GitProjection(...)` |
| gix write failure (disk full, permissions) | `Error::GitProjection(...)` |
| Object not found during tree walk | `Error::Storage(...)` (existing) |
| ACL denied (timeline or path) | silently skipped, not an error |
| `Secret` / `EnvOverlay` tree entry | silently skipped, not an error |

---

## Crate Structure Changes

```
src/
├── error.rs          # add GitProjection variant
├── repo/
│   ├── mod.rs        # add pub mod git_projection
│   └── git_projection.rs  # project_to_git + convert_tree
└── lib.rs            # re-export project_to_git

tests/
└── git_projection.rs  # T7 integration tests
```

---

## Testing (T7)

All tests in `tests/git_projection.rs`. Verification reads back the exported repo using `gix::open(target_path)` — no `git` binary required.

### `t7_linear_timeline_projects`

Setup:
- Create a `Repository::memory()` with one timeline `"main"`
- Commit 3 snapshots in a linear chain (each parents the previous)
- `project_to_git(repo, target_path, &full_write_accessor)`

Verify:
- `refs/heads/main` exists in the exported git repo
- Walking the commit chain from `refs/heads/main` yields exactly 3 commits in order
- The head commit's author field contains `"bole@local"`
- Commit parentage is correct (second commit parents first, third parents second)

### `t7_private_paths_excluded`

Setup:
- Repository with one timeline, snapshot tree containing `src/app.rs` (public) and `secrets/key` (ACL-protected)
- Projection 1: accessor with read on `src/**` only
- Projection 2: `Accessor::privileged()`

Verify:
- Projection 1 git tree: contains `src/app.rs`, does NOT contain `secrets/key`
- Projection 2 git tree: contains both paths

### `t7_shared_ancestry_deduplicated`

Setup:
- Two timelines `"branch-a"` and `"branch-b"` that share a common ancestor snapshot
- Each has one additional snapshot after the fork

Verify:
- Both `refs/heads/branch-a` and `refs/heads/branch-b` exist
- The common ancestor commit appears exactly once in the git object store (not written twice)
- Each branch head commit has the common ancestor as its parent

### `t7_tags_projected`

Setup:
- One timeline, two snapshots; a bole `Tag` named `"v1.0"` pointing to the first snapshot

Verify:
- `refs/tags/v1.0` exists in the exported git repo
- It points to the same commit SHA as the first snapshot

### `t7_secret_entries_skipped`

Setup:
- Manually store a `Secret` object in the object store, get its `ObjectId`
- Build a tree entry with `EntryKind::Blob` pointing to that `ObjectId`
- Include that tree in a snapshot, project with `Accessor::privileged()`

Verify:
- The projected git tree does NOT contain the secret entry's path
- `project_to_git` returns `Ok(())` (no error)

---

## Out of Scope (Gate 7)

- Incremental projection with a persistent bole→git mapping table
- Annotated git tags (only lightweight tags are written)
- Git pack files (loose object format is sufficient for correctness; packing is an optimization)
- `.gitattributes`, `.gitignore` generation
- Git LFS integration
- Projecting `EnvOverlay` values as git notes or custom refs
- Two-way sync (git → bole import)
- `git fast-import` stream output format
