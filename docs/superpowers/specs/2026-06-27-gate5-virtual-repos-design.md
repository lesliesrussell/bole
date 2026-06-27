# Gate 5: In-Memory and Virtual Repos

**Project:** bole — a next-generation version control system  
**Language:** Rust (async, tokio)  
**Date:** 2026-06-27  
**Spec ref:** spec.md Gate 5, Test T5

---

## Context

Gates 1 and 2 delivered pluggable async `ObjectStore` (wrapping `Box<dyn StorageBackend>`) and pluggable sync `RefStore` (wrapping `Box<dyn RefBackend>`). The backend abstractions exist; Gate 5 makes them accessible through a single unified `Repository` entry point, adds the ability to copy repos between backends, and adds a `materialize` operation to project a Snapshot's tree to a real filesystem directory.

Key architectural decisions made during brainstorming:
- **Thin `Repository` wrapper** — `{ objects: ObjectStore, refs: RefStore }` with constructors only. Callers use `.objects` and `.refs` directly; no convenience methods added.
- **`list()` on `StorageBackend` trait** — enables generic backend-to-backend copy without coupling concrete types.
- **`materialize` as a free function** — not a method on `Repository`, keeping `Repository` minimal.

---

## StorageBackend Extension

One new method added to the existing `StorageBackend` trait in `src/store/backend.rs`:

```rust
async fn list(&self) -> Result<Vec<ObjectId>>;
```

`MemoryBackend` iterates its `Arc<RwLock<HashMap<[u8;32], Bytes>>>` keys.

`DiskBackend` walks the `objects/<xx>/` shard directories and parses each filename as a hex-encoded `ObjectId`. Files that cannot be parsed as valid hex are skipped silently (same defensive pattern as `DiskRefBackend::walk_refs`).

A free async function handles object-level copy:

```rust
pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()>
```

It calls `from.list()`, then for each `ObjectId` fetches raw bytes via `StorageBackend::get` and writes them directly via `StorageBackend::put` — no re-encoding. Content-addressed identity is preserved automatically.

---

## Repository Type

```rust
pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
}

impl Repository {
    /// Fully in-memory repo. Never touches disk.
    pub fn memory() -> Self;

    /// Disk-backed repo. Both stores root at `root`.
    /// Objects: `<root>/objects/`. Refs: `<root>/refs/`.
    pub async fn disk(root: impl AsRef<Path>) -> Result<Self>;

    /// Copy all objects and refs from self to dest.
    pub async fn copy_to(&self, dest: &Repository) -> Result<()>;
}
```

`memory()` pairs `MemoryBackend::new()` with `MemoryRefBackend::new()`.

`disk(root)` pairs `DiskBackend::open(root).await?` with `DiskRefBackend::open(root)?`. Both are rooted at the same directory — they coordinate via their respective subdirectories (`objects/` and `refs/`), which is the same layout the existing backends already use.

`copy_to` calls `copy_objects(&self.objects, &dest.objects).await?` then `copy_refs(&self.refs, &dest.refs)?`. The ref copy uses `self.refs.list("")` (returns all ref names), then for each name calls `self.refs.get(name)` and writes directly to `dest.refs` via a `pub(crate) fn set_raw(&self, name: &RefName, r: &Ref) -> Result<()>` on `RefStore` — bypassing the `create_tag`/`create_timeline` existence checks, which would reject a second copy into a non-empty destination.

Lives in `src/repo/mod.rs`. `pub mod repo` added to `src/lib.rs` with re-exports of `Repository`, `copy_objects`, and `materialize`.

---

## Materialize

```rust
pub async fn materialize(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
    dest: impl AsRef<Path>,
) -> Result<()>
```

**Algorithm:**

1. Fetch the object at `snapshot_id`. If missing or not a `Snapshot`, return `Error::Storage`.
2. Call the recursive tree walker with `snapshot.root` and `dest`.
3. Tree walker for a given `(tree_id, base_path)`:
   - Fetch the `Tree` at `tree_id`. If missing or not a `Tree`, return `Error::Storage`.
   - For each `(name, entry)` in `tree.entries`:
     - `EntryKind::Blob` → fetch blob, `tokio::fs::write(base_path/name, blob.data)`.
     - `EntryKind::Tree` → `tokio::fs::create_dir_all(base_path/name)`, then recurse.

`dest` is created if it does not exist. Existing files at `dest` are overwritten. No cleanup is performed on error (partial materialization left in place — callers use a `TempDir` in tests).

Lives in `src/repo/materialize.rs`, re-exported from `src/repo/mod.rs`.

---

## Crate Structure Changes

```
src/
├── lib.rs              # add: pub mod repo; re-export Repository, materialize, copy_objects
├── repo/
│   ├── mod.rs          # Repository struct + memory() + disk() + copy_to() + copy_refs()
│   └── materialize.rs  # materialize() free function
├── store/
│   ├── backend.rs      # add list() to StorageBackend trait
│   ├── memory.rs       # impl list()
│   └── disk.rs         # impl list() — walk objects/ shards
└── (refs/, object/, codec.rs, error.rs unchanged)

tests/
└── repo.rs             # T5 integration tests
```

---

## Testing Approach

Integration tests in `tests/repo.rs`. All tests are `#[tokio::test]`.

**t5_memory_to_disk_round_trip:**
- Create `Repository::memory()`.
- Build 1000 snapshots in sequence, each a single-file tree with distinct content. Record all snapshot IDs and tag 10 of them via `repo.refs.create_tag(...)`.
- Call `repo.copy_to(&disk_repo)` where `disk_repo = Repository::disk(tmp).await`.
- Create `Repository::disk(same_tmp).await` as a fresh reload.
- For each recorded snapshot ID, `disk_reload.objects.get(id)` must return `Some` with identical content.
- For each recorded tag name, `disk_reload.refs.get_tag(name)` must return the same target `ObjectId`.

**t5_materialize_and_rematerialize:**
- Create `Repository::memory()`.
- Build a snapshot with a small tree: `src/main.rs` ("fn main() {}"), `README.md` ("hello"), `nested/a.txt` ("a").
- `materialize(&repo.objects, snap_id, &dest1)` → verify all three file contents match.
- Drop `dest1` (TempDir auto-cleanup).
- `materialize(&repo.objects, snap_id, &dest2)` → verify all three file contents match again (objects still in store).

**Unit tests** (in-module):
- `StorageBackend::list()` on `MemoryBackend` — put 3 objects, list returns all 3 ids.
- `StorageBackend::list()` on `DiskBackend` — same.
- `copy_objects` — put 5 objects in memory, copy to a second memory store, assert all 5 present.
- `copy_refs` — create 2 tags + 1 timeline in one `RefStore`, copy to another, assert all 3 present with correct targets.
- `materialize` with a missing snapshot id — returns `Error::Storage`.
- `materialize` with a tree entry referencing a missing blob — returns `Error::Storage`.

---

## Key Dependencies

No new crates required. All dependencies from Gates 1 and 2 apply:

| Crate | Gate 5 use |
|-------|-----------|
| `tokio` | async I/O in `materialize`, `disk()` constructor |
| `serde` + `postcard` | existing codec unchanged |
| `thiserror` | existing error types unchanged |
| `tempfile` | dev-dependency for TempDir in tests |

---

## Out of Scope (Gate 5)

- Remote / network backends (custom KV store plugin API — future gate)
- Incremental / partial materialization
- Watched materialization (live-sync to disk)
- Permission filtering during materialization (Gate 3)
- Secret/env overlay injection during materialization (Gate 4)
