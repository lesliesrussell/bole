# `bole ignore` — Design

**Date:** 2026-07-04
**Status:** Approved, pending implementation

## Problem

The disk-walk that builds a snapshot (`DiskWorkspace::collect`) currently
captures *every* regular file under the workspace root, excluding only `.bole`
and symlinks. There is no way to keep build artifacts, logs, dependency
directories, or scratch files out of snapshots. bole needs a first-class
ignore mechanism.

## Goals

- A user-managed set of ignore patterns with familiar (gitignore) semantics.
- A `bole ignore` CLI command to add, list, remove, and test patterns.
- Enforcement at the single disk-walk so every snapshot honors it.

## Non-goals (v1)

- Nested per-directory `.boleignore` files. The matcher supports them; v1 reads
  only the workspace-root file. Documented future extension.
- Honoring an existing `.gitignore`. Separate concern; not in scope.
- Applying ignore rules to `EphemeralWorkspace` (in-RAM, no filesystem).

## Storage

A plain-text `.boleignore` at the workspace root:

- One pattern per line.
- Lines starting with `#` are comments; blank lines are ignored.
- It is a **normal tracked file** — captured in snapshots, versioned, and
  portable across clones (this is why it lives in the tree, not in `.bole/`).

`.bole` (directory or linked-worktree pointer file) remains **hard-excluded**
unconditionally, independent of any pattern. Symlinks remain skipped.

## Matcher

Add the `ignore` crate. Use `ignore::gitignore::Gitignore` built via
`GitignoreBuilder`. This provides full gitignore pattern semantics — globs,
`**`, dir-only trailing `/`, path anchoring, and negation (`!`) — while letting
us keep our own async directory walk (we do **not** use the crate's walker).

The matcher is built from the root `.boleignore`, rooted at the workspace root.
A missing file yields an empty matcher (nothing ignored, no error).

## Enforcement

Single hook: `DiskWorkspace::collect()` in `src/repo/ephemeral.rs`.

At the start of the walk, load `.boleignore` into a `Gitignore`. In the walk
loop:

- **Directory entry** → `matched(path, /*is_dir=*/ true)`. If ignored, prune:
  do not push it onto the stack. A pattern like `target/` skips the entire
  subtree cheaply.
- **File entry** → `matched(path, /*is_dir=*/ false)`. If ignored, skip: do not
  store the blob or record the path.

Paths are matched relative to the workspace root, using forward slashes,
consistent with how `collect` already builds its keys. Skips are silent — they
simply do not appear in the resulting `path → blob id` map, so no snapshot
mutation or diff noise.

`EphemeralWorkspace` is unaffected (it has no disk to walk).

## CLI surface

New command `bole ignore`, implemented in `bole-cli/src/commands/ignore.rs`,
wired into the `Command` enum in `bole-cli/src/main.rs`.

- `bole ignore <pattern>...` — append pattern(s) to `.boleignore`, creating the
  file if absent. Deduplicates (skip patterns already present), writes one per
  line. This bare form is the headline UX; `add` is its default subcommand.
- `bole ignore list` — print the active patterns (excluding comments/blanks).
- `bole ignore remove <pattern>...` — delete lines matching the given
  pattern(s) exactly.
- `bole ignore check <path>...` — for each path, report whether it would be
  ignored and, if so, which pattern matched. Uses the **same** matcher as
  `collect`, so it is a faithful dry-run.

### Clap ambiguity risk

The bare `bole ignore "*.log"` form and the `list`/`remove`/`check`
subcommands can collide (a pattern literally named `list`). Plan: configure
`add` as the default subcommand so bare patterns route to it. If clap cannot
express this cleanly, fall back to requiring explicit `bole ignore add
<pattern>` and keep `list`/`remove`/`check` as siblings. Decide during
implementation.

## Error handling

- Missing `.boleignore` → empty matcher; no error on any command.
- Malformed pattern on `add` → validate through `GitignoreBuilder` before
  writing; reject with a clear message, do not modify the file.
- `remove` of a non-present pattern → no-op with an informational note.

## Testing

**Unit (matcher / `collect`):**
- File matching `*.log` is absent from the resulting snapshot map.
- `target/` prunes the whole subtree (no descendant blobs stored).
- Negation: `!keep.log` re-includes a file excluded by a broader pattern.
- Dir-only `build/` matches the directory, not a file named `build`.
- Missing `.boleignore` ignores nothing.

**CLI (existing `run()` / `ok()` harness in `bole-cli/tests`):**
- `add` then `list` roundtrip; dedup on repeated add.
- `remove` deletes the line; `list` reflects it.
- `check` reports ignored/not with the matching pattern.
- Malformed pattern is rejected without writing.

## Files touched

- `Cargo.toml` — add `ignore` dependency.
- `src/repo/ephemeral.rs` — load matcher, prune/skip in `collect`.
- `bole-cli/src/commands/ignore.rs` — new command module.
- `bole-cli/src/commands/mod.rs`, `bole-cli/src/main.rs` — wire it in.
- Tests alongside the above.
