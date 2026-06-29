# WS2 — Workspace Unification Design Spec

**Bead:** `bole-1kz`
**Dependencies:** none
**Foundations:** [docs/superpowers/specs/2026-06-29-roadmap-foundations.md](./2026-06-29-roadmap-foundations.md)

---

## Goal

bole currently has two parallel worktree implementations that share library
primitives but are not the same abstraction:

- `EphemeralWorkspace` — a pure in-RAM worktree used by agents and library
  consumers; lives in `src/repo/ephemeral.rs`.
- The CLI disk walk — `bole-cli/src/worktree.rs` + the four command-level call
  sites in `snapshot.rs` and `workspace.rs`; walks the filesystem using
  `collect_blobs` and delegates tree-building and diff to the shared library
  primitives.

Both represent the same concept: *a mutable, path-keyed byte store that can be
diffed against a base snapshot and committed to the object graph.* They differ
only in their backing medium.

This workstream extracts a single `Workspace` trait from this duality, adds
`DiskWorkspace` as a filesystem-backed implementation, and retrofits
`EphemeralWorkspace` as a second implementation — preserving all existing public
API, all CLI command behaviour, and all 247 passing tests.

---

## Architecture

### Component boundaries

```
bole (library crate)
  src/repo/ephemeral.rs
    PathDiff                 -- unchanged
    build_tree               -- unchanged, stays as free fn
    snapshot_paths           -- unchanged, stays as free fn
    diff_paths               -- unchanged, stays as free fn
    Workspace trait          -- NEW
    EphemeralWorkspace       -- existing struct; gains Workspace impl
    DiskWorkspace            -- NEW struct + Workspace impl

bole-cli (CLI crate)
  src/worktree.rs
    collect_blobs            -- retained as the disk-walk primitive
                             -- re-used inside DiskWorkspace::collect
    re-exports               -- unchanged (build_root_tree, diff, snapshot_blobs)
  src/commands/workspace.rs  -- call sites updated to use DiskWorkspace
  src/commands/snapshot.rs   -- call sites updated to use DiskWorkspace
```

> **Placement rationale.** `DiskWorkspace` lives in the library, not the CLI,
> because both trait and impls should be co-located so that out-of-tree users
> can drive a disk workspace without depending on the CLI. This requires
> `tokio::fs` as a library dependency (see Open questions §1).

---

## The `Workspace` trait

### Exact method signatures

```rust
/// A mutable, path-keyed byte store that can be diffed against a base snapshot
/// and committed to the object graph.
///
/// Two implementations exist: [`EphemeralWorkspace`] (in-memory) and
/// [`DiskWorkspace`] (filesystem-backed).  The work tree is not a second model
/// — it is one model with a filesystem backing.
pub trait Workspace {
    /// The snapshot this workspace considers its starting point.  Used as the
    /// sole parent when [`commit`](Self::commit) creates a new snapshot.
    fn base(&self) -> Option<ObjectId>;

    /// Returns the bytes at `path`, or `None` if the path is absent.
    async fn read(&self, path: &str) -> Result<Option<Bytes>>;

    /// Creates or overwrites `path` with `bytes`.
    async fn write(&mut self, path: &str, bytes: Bytes) -> Result<()>;

    /// Deletes `path`.  Returns `true` if it existed.
    async fn remove(&mut self, path: &str) -> Result<bool>;

    /// Returns all paths in the workspace, in sorted order.
    async fn paths(&self) -> Result<Vec<String>>;

    /// Computes the diff of the current workspace state against the base
    /// snapshot.  Has the same side-effect contract as
    /// [`EphemeralWorkspace::diff`]: it stores blobs in the object store
    /// (content-addressed, idempotent).
    async fn diff(&self) -> Result<PathDiff>;

    /// Commits the current workspace state as a new snapshot whose parent is
    /// [`base`](Self::base).  Advances `base` to the new snapshot id.
    ///
    /// Does **not** advance any timeline.  Call [`Repository::advance_timeline`]
    /// after commit to publish the snapshot on a timeline.
    async fn commit(
        &mut self,
        author: &str,
        message: &str,
        created_at: u64,
    ) -> Result<ObjectId>;
}
```

### Design decisions

**All methods are async.** `EphemeralWorkspace`'s inherent `read`/`write`/
`remove`/`paths` are currently synchronous; they become trivially async in the
trait impl (no `.await` needed). Making the trait async throughout avoids a
split surface and correctly models `DiskWorkspace`, where every operation
crosses the I/O boundary.

**`read` returns `Result<Option<Bytes>>` (owned).** The inherent
`EphemeralWorkspace::read` returns `Option<&[u8]>` (borrowed from self). A
borrowed return in an async trait method creates complex lifetime constraints.
The trait method returns an owned `Bytes`; the inherent method is preserved
unchanged for existing callers.

**`write` takes `&str` / `Bytes` (concrete, not generic).** Inherent methods on
`EphemeralWorkspace` keep their `impl Into<String>` / `impl Into<Bytes>`
signatures. Trait methods use concrete types to avoid HRTB in trait objects.

**`base` is synchronous.** It reads a field; no I/O is involved in either
implementation.

**Error type is `bole::error::Result`.** `DiskWorkspace` maps
`std::io::Error` to `bole::Error::Storage`. A new `Error::Io(String)` variant
is the clean home for disk errors, keeping the bole error type self-contained.
Alternatively, `Error::Storage` can absorb IO strings; the exact variant is an
implementation-time call and does not affect this spec.

**`async fn` in traits (AFIT), not `async_trait`.** Rust 1.75 stabilised AFIT.
`dyn Workspace` is not required by any current call site; if it becomes needed,
the trait can be wrapped with `async_trait` at that point. See Open questions §2.

---

## `DiskWorkspace`

### Data model

```rust
pub struct DiskWorkspace<'a> {
    repo: &'a Repository,
    root: PathBuf,
    base: Option<ObjectId>,
}
```

### Constructors

```rust
impl<'a> DiskWorkspace<'a> {
    /// Creates a workspace rooted at `root` with no base snapshot.
    pub fn new(repo: &'a Repository, root: impl Into<PathBuf>) -> Self { … }

    /// Creates a workspace rooted at `root` with `snapshot` as the base.
    /// Does not read any files from disk; construction is cheap.
    pub fn bound(
        repo: &'a Repository,
        root: impl Into<PathBuf>,
        snapshot: ObjectId,
    ) -> Self { … }
}
```

### Backing strategy — lazy read

`DiskWorkspace` does **not** eagerly load files into memory at construction
time.  Files are read from disk on demand by `read()`, and the full directory
walk is deferred to `paths()`, `diff()`, and `commit()`.

Rationale: the CLI's most common operations (`diff`, `snapshot create`) already
do a single full walk at operation time via `collect_blobs`.  Eager loading
would do the walk twice (once at construction, once at the operation).  Lazy
reading also keeps large repositories from loading into RAM unnecessarily.

### `.bole` exclusion

`DiskWorkspace` internally delegates the directory walk to a `collect` helper
that is functionally identical to the existing `collect_blobs` in
`bole-cli/src/worktree.rs`:

- Any directory entry whose name equals `REPO_DIR` (`.bole`) is skipped,
  whether it is a directory (primary worktree's repo dir) or a regular file
  (linked worktree's pointer file).  This matches the existing two-branch
  exclusion in `collect_blobs`.
- The helper stores each non-excluded regular file as a blob in the object
  store and returns `BTreeMap<String, ObjectId>` with forward-slash-separated
  paths relative to `root`.

This helper is the *single implementation* of the disk walk; the CLI's
`collect_blobs` in `worktree.rs` is kept as a re-export or thin wrapper that
delegates to it, eliminating the duplication.

### `commit` write-through

`DiskWorkspace::commit` performs the same three-step sequence as
`EphemeralWorkspace::commit`, but sourced from the disk:

1. `collect` (disk walk → `BTreeMap<String, ObjectId>`)
2. `build_tree(&repo.objects, &blobs)` — shared library primitive
3. `repo.objects.put_snapshot(Snapshot { root, parents, author, created_at, message })`

`parents` is `self.base.map(|b| vec![b]).unwrap_or_default()`.  After a
successful commit, `self.base` is updated to the new snapshot id.

`write(path, bytes)` and `remove(path)` write directly through to the
filesystem (write-through model, see Open questions §3).  The in-memory file
content is not buffered; the next `collect` call reads the post-write disk
state.

### `diff`

```
base_map  = snapshot_paths(&repo.objects, self.base)  // or empty if no base
target_map = collect(&repo.objects, &self.root)
PathDiff  = diff_paths(&base_map, &target_map)
```

Both `snapshot_paths` and `diff_paths` are the shared library primitives from
`ephemeral.rs`.  No new code is needed for the diff algorithm.

---

## `EphemeralWorkspace` as a `Workspace` impl

`EphemeralWorkspace` keeps its entire existing public API as inherent methods.
The `Workspace` trait impl is additive; it thin-wraps the inherent methods.
No existing call site changes.

```rust
impl Workspace for EphemeralWorkspace<'_> {
    fn base(&self) -> Option<ObjectId> { self.base() }

    async fn read(&self, path: &str) -> Result<Option<Bytes>> {
        Ok(self.read(path).map(Bytes::copy_from_slice))
    }

    async fn write(&mut self, path: &str, bytes: Bytes) -> Result<()> {
        self.write(path, bytes);
        Ok(())
    }

    async fn remove(&mut self, path: &str) -> Result<bool> {
        Ok(self.remove(path))
    }

    async fn paths(&self) -> Result<Vec<String>> {
        Ok(self.paths().map(str::to_owned).collect())
    }

    async fn diff(&self) -> Result<PathDiff> { self.diff().await }

    async fn commit(&mut self, author: &str, message: &str, created_at: u64) -> Result<ObjectId> {
        self.commit(author, message, created_at).await
    }
}
```

The four existing unit tests (`empty_commit_then_seed_roundtrip`,
`diff_reports_add_modify_remove`, `commit_chains_parents`,
`clean_diff_is_empty`) call the inherent methods and are entirely unaffected.

---

## CLI command re-expression

### `snapshot create --from-workspace`

**Before**
```rust
let blobs = worktree::collect_blobs(&ctx.repo.objects, &ctx.work_dir).await?;
let root  = worktree::build_root_tree(&ctx.repo.objects, &blobs).await?;
ctx.repo.objects.put_snapshot(Snapshot { root, parents, … }).await?;
```

**After**
```rust
let mut ws = DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, head_snapshot);
let snap_id = ws.commit(author, &message, resolve::now()).await?;
```

Timeline advance logic (after `commit`) is unchanged.  The `parents` field is
now derived from `ws.base()`, which holds `head_snapshot`.

### `workspace diff`

**Before**
```rust
let base   = worktree::snapshot_blobs(&ctx.repo.objects, head).await?;
let target = worktree::collect_blobs(&ctx.repo.objects, &ctx.work_dir).await?;
let d      = worktree::diff(&base, &target);
```

**After**
```rust
let ws = DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, head);
let d  = ws.diff().await?;
```

### `workspace show`

Identical change to `workspace diff`: construct `DiskWorkspace::bound`, call
`diff()`, format the `PathDiff`.

### `workspace open`

`open` materialises a snapshot to disk and binds a timeline.  The
materialisation step (`bole::materialize`) is unchanged.  After materialisation,
a `DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, head)` represents the
workspace — but the command does not construct one explicitly; it merely records
the binding in CLI state as today.  The workspace is constructed on demand by
subsequent `diff`/`commit` calls.

### Linked worktrees (`workspace add / list / remove`)

These commands manage the worktree registry (pointer files + per-worktree
`state.json`) and materialise timeline heads.  They do not change.

Each registered linked worktree directory is *representable* as a
`DiskWorkspace::bound(repo, linked_path, head)` by any code that needs to
operate on it programmatically.  The workspace abstraction fits naturally
without any structural change to the add/list/remove logic.

The per-worktree `REPO_DIR` pointer file exclusion in `collect_blobs` already
handles the case where a linked worktree's `.bole` is a file rather than a
directory.  This exclusion carries forward into `DiskWorkspace::collect`
unchanged.

---

## Backward compatibility and migration

| Surface | Change | Risk |
|---------|--------|------|
| `EphemeralWorkspace` public API | None — all inherent methods kept | None |
| `build_tree`, `snapshot_paths`, `diff_paths` free functions | None — kept as-is | None |
| CLI command output | None — identical bytes out | None |
| `worktree::collect_blobs` in CLI | Delegates to `DiskWorkspace::collect` | Low — same logic |
| `worktree::build_root_tree`, `diff`, `snapshot_blobs` re-exports | Unchanged | None |
| Error type | Possible new `Error::Io` variant | Additive only |

No existing test needs to change.  The 247 tests that pass today must pass
after the refactor.  New tests cover `DiskWorkspace` exclusively.

---

## Testing strategy

### Unit tests (new, in `src/repo/ephemeral.rs` or a sibling module)

1. **`disk_workspace_roundtrip`** — create a `DiskWorkspace` over a temp dir,
   write a file via `ws.write()`, call `ws.commit()`, verify the returned
   snapshot contains the file via `snapshot_paths`.
2. **`disk_workspace_diff_add_modify_remove`** — seed a `DiskWorkspace::bound`
   from a snapshot, add/modify/remove files on disk directly, call `ws.diff()`,
   assert the `PathDiff` matches.
3. **`disk_workspace_excludes_bole_dir`** — place a `.bole/` directory and a
   `.bole` file inside the temp dir, call `ws.paths()`, assert neither appears.
4. **`disk_workspace_commit_chains_base`** — commit twice, assert the second
   snapshot's parent is the first.
5. **`trait_object_dispatch`** — `let ws: &mut dyn Workspace = &mut DiskWorkspace::new(…);`
   exercises that the trait impl compiles and is callable through a reference
   (relevant only if dyn dispatch remains possible; gated on Open question §2).

### Existing tests (unchanged)

The four `EphemeralWorkspace` unit tests in `ephemeral.rs` pass without
modification.

### CLI integration (existing test suite)

Run the full 247-test suite after the refactor.  Any failure is a regression.
No new CLI integration tests are required in this workstream; WS7 (worktree
hardening) owns end-to-end CLI coverage.

---

## Open questions

> These require the maintainer's call before implementation begins.

**OQ1 — `DiskWorkspace` crate placement.** This spec places `DiskWorkspace` in
the library (`bole` crate) so both implementations share a home.  This adds
`tokio::fs` as a direct runtime dependency of the library.  If `tokio` is
already a non-dev dependency of `bole`, this is free.  If it is dev-only today,
the cost is a heavier library crate.  **Alternative:** place `DiskWorkspace` in
`bole-cli` and export the trait from the library.  Downside: out-of-tree library
users who want disk-backed workspaces must depend on the CLI crate.

**OQ2 — AFIT vs `async_trait` for `dyn Workspace`.** AFIT (stable Rust ≥ 1.75)
does not support `dyn Trait` out of the box; each impl generates a unique future
type.  If any current or planned call site needs a trait object (`Box<dyn
Workspace>` or `&mut dyn Workspace`), the trait must either use `async_trait`
(heap-allocates futures, works with dyn) or carry an explicit `BoxFuture` return.
If no dyn dispatch is needed (all call sites are generic `impl Workspace`), AFIT
is the right choice.  **Needs a decision before the trait is stabilised.**

**OQ3 — Write-through vs buffered writes for `DiskWorkspace`.** This spec
specifies write-through: `ws.write(path, bytes)` immediately writes to
`root/path`.  This is the simplest model and matches user expectations (files
appear on disk immediately after `write`).  A buffered model (writes held in
memory, flushed on `commit`) would support transactional rollback but is
significantly more complex.  The CLI does not need rollback.  **Recommendation:
write-through in v1; revisit in WS7 if atomic workspace updates are required.**

**OQ4 — Symlink handling.** `collect_blobs` calls `file_type.is_file()`, which
in Rust's `std::fs` follows symlinks (a symlink to a regular file returns
`true`).  Symlinks to directories are skipped (they return `false` for `is_file()`
and `false` for `is_dir()` when the target is absent, but `true` for `is_dir()`
when present — causing them to be stacked for further walk, potentially
surprising).  **Decision needed:** follow symlinks (current implicit behaviour),
skip all symlinks explicitly, or error on symlinks.

**OQ5 — `paths()` re-walk on repeated calls.** `DiskWorkspace::paths()` walks
the disk each time it is called.  A single command that calls `paths()` then
`diff()` walks twice.  Caching risks stale results if the disk changes between
calls.  The current CLI command flow never calls `paths()` followed by `diff()`
in the same command, so this is a v1 non-issue.  **No cache in v1; document the
two-walk cost for callers that need both.**

**OQ6 — Deletions representation via `remove()`.** `DiskWorkspace::remove(path)`
deletes the file from disk.  The next `collect` call will not see it, and
`diff_paths` will correctly classify it as `removed`.  No tombstone or
in-memory deletion record is required.  **Confirm this is sufficient for all
anticipated call sites before implementing.**
