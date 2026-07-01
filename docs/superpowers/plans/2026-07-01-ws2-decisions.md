# WS2 — Locked Implementation Decisions (bole-1kz)

Resolves the six open questions in
[specs/2026-06-29-ws2-workspace-unification.md](../specs/2026-06-29-ws2-workspace-unification.md)
before implementation. Maintainer-approved 2026-07-01.

| OQ | Decision | Rationale |
|----|----------|-----------|
| OQ1 — placement | **`DiskWorkspace` lives in the `bole` library** (`src/repo/ephemeral.rs`) | `tokio` with the `fs` feature is already a non-dev library dependency, so there is no added cost; both impls share a home and out-of-tree users get disk workspaces without the CLI. |
| OQ2 — async trait | **`#[async_trait]`** (not native AFIT) | Already a dependency (`acl/hook.rs`); supports `dyn Workspace` so spec test #5 stands; avoids the `async_fn_in_trait` lint on a public trait. |
| OQ3 — write model | **Write-through** | `ws.write`/`ws.remove` hit the filesystem immediately; next `collect` reads post-write disk state. CLI needs no rollback. |
| OQ4 — symlinks | **Skip all symlinks** | `collect` checks `is_symlink()` and continues. Prevents content escaping the workspace root and surprising directory-symlink recursion. Slight change from today's implicit follow (verified no existing test depends on symlink-following). |
| OQ5 — `paths()` cache | **No cache in v1** | Each `paths()`/`diff()` walks disk; current command flow never calls both in one command. Two-walk cost documented for callers. |
| OQ6 — deletions | **No tombstone** | `remove(path)` deletes from disk; the next `collect` omits it and `diff_paths` classifies it `removed`. |

**Error type:** `Error::Io(#[from] std::io::Error)` already exists — `DiskWorkspace`
uses `?` directly on IO errors; no new variant needed.

## Build order (TDD)

1. `Workspace` trait + `impl Workspace for EphemeralWorkspace` (additive; 4 existing tests unaffected).
2. `DiskWorkspace` struct + `new`/`bound` + `collect` helper (skips `.bole` + symlinks) + `Workspace` impl. Spec tests 1–5.
3. Retrofit CLI `collect_blobs` to delegate to the shared disk walk; update `snapshot.rs` + `workspace.rs` call sites to `DiskWorkspace`.
4. Full suite green (no existing test changes).
