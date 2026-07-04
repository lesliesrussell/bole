---
name: using-bole
description: Use when version-controlling a project with bole — either the `bole` CLI over a `.bole/` repository, or the embeddable library in-process (`Repository::memory()` / `ephemeral_workspace`). bole is access-controlled, multi-actor version control: actor grants, ACL rules, and policy-gated operations live in the repository model, not a hosting platform. Triggers on the `bole` command, `.bole/` repositories, the bole library, or any task involving actors, timelines, secrets, env overlays, or linked worktrees.
---

# Using bole

`bole` is a content-addressed version-control **library** with a `bole` CLI on
top of it. It is **not** Git — the nouns are different and map to bole's model.

## Two first-class backends, one object model

bole runs the same object model — snapshots, timelines, actors, ACLs, policy —
over either backend. Pick per use case:

- **In-memory (library, in-process):** `Repository::memory()` +
  `repo.ephemeral_workspace()` give a `read`/`write`/`remove`/`diff`/`commit`
  working tree entirely in RAM — no `.bole/`, no filesystem. This is the path for
  in-process agents and tests. See *In-memory backend* below.
- **On disk (the `bole` CLI):** the CLI drives a `.bole/` directory for durable
  storage — plus linked worktrees, packs/GC, distributed sync, and git interop.

Neither is a footnote: in-RAM embedding and the on-disk CLI are peer ways to use
bole.

More precisely: bole puts access control **inside** the repository. Named actors
carry path and timeline grants; a bound actor gates every access-controlled
operation; secrets are encrypted objects in the same store as source files.
Automated agents and human developers are the same concept — just different grant
sets.

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

| Concept | Meaning |
|---------|---------|
| Repository | the object/ref/ACL store — in RAM (`Repository::memory()`) or a `.bole/` directory on disk (`Repository::disk()` / the CLI) |
| Snapshot | an immutable file tree + metadata — the durable state |
| Timeline | a movable named pointer to a snapshot (≈ branch) |
| Tag | a fixed named pointer to a snapshot |
| Actor | a named bundle of path/timeline grants; the bound actor is the identity used for access checks |
| Secret / Env overlay | encrypted value / named variable bundle |

CLI commands discover the on-disk repo by walking up from the current directory
(like Git).

## Critical facts

- **The CLI has no session: each `bole` command is a separate process and its
  state lives in `.bole/` on disk.** Don't expect CLI state to persist except
  through the repo. (In-process, the library backend keeps everything in RAM
  instead — see *In-memory backend*.)
- **Add `--json` for any output you need to parse.** It is the stable contract;
  human text is not. `--quiet` suppresses non-error output.
- **Snapshots are created from the work tree**, not staged files. There is no
  "add"/staging step.
- **`.boleignore` excludes files from snapshots.** A gitignore-syntax file at the
  work-tree root; matching files and directories are skipped by the work-tree
  walk (ignored dirs are pruned whole). Manage it with `bole ignore`. `.bole/`
  and symlinks are always excluded regardless, and `.boleignore` itself is a
  normal tracked, versioned file.
- **Timeline policy is enforced**: `ff`/`append` timelines reject an advance to a
  non-descendant; `unrestricted` accepts any snapshot.
- **Secrets need a key**: a 64-hex (32-byte) key via `$BOLE_KEY` or
  `--key-file`. It is never stored in the repo.
- **`--as <actor>` binds capability for all workspace operations.** An agent
  scoped to `src/**` write cannot see or touch `secrets/**` — enforced at the API
  level, not by convention.
- **`workspace prune` / `workspace repair`** clean up stale or moved linked
  worktree registrations; run them after moving directories or the store
  (`workspace list --check` flags staleness).
- **More commands:** `env resolve <name> [--reveal]` and `run --env <name> -- <cmd>`
  (access-checked env injection), `secret rekey` (rotate the master key),
  `git import <path>` (import a Git repo), `store repack` / `store gc`
  (consolidate loose objects / collect garbage).

## Core workflow (CLI / on-disk)

```bash
bole init .                                   # create .bole/
# ...write files...
SNAP=$(bole snapshot create --from-workspace -m "initial" --json | jq -r .snapshot)
bole workspace open main --create --from "$SNAP"   # bind work tree to a new timeline
# ...edit files...
bole workspace diff                           # work tree vs bound head
bole snapshot create --from-workspace -m "changes"  # advances the bound timeline
bole snapshot list                            # history, newest first
```

## Command map (CLI)

- **Lifecycle**: `init`, `status`, `repo info`
- **History**: `snapshot create|show|list|parents|diff`, `timeline create|list|show|advance|delete` (alias `branch`/`branches`), `tag create|list|show|delete`
- **Work tree**: `workspace open|show|diff|materialize|clear|add|list|remove` (add/list/remove = linked worktrees sharing one store)
- **Ignore**: `ignore <pattern>...` (bare = add), `ignore list|remove <pattern>...|check <path>...` — gitignore-style `.boleignore` at the work-tree root; ignored files/dirs are excluded from snapshots. `check` is a dry-run using the same matcher as the snapshot walk.
- **Merge**: `merge check <src> <dst>` (dry run), `merge run <src> <dst>` (advances dst when clean; reports conflicts otherwise)
- **Access**: `actor create|grant-path|grant-timeline|use|show|list`, `acl path|timeline protect|unprotect|list`, `acl can-{read,write}-{path,timeline}`, `acl explain-path --actor <a> <path>` (full read/write decision trace — the "why is this hidden?" answer)
- **Signed approvals**: `policy require-approval <pattern> --needed <n>` (gate advance/merge on N signed approvals), `policy list|unrequire`, `approver add <id> --public-key|--seed <64hex>`, `approver list`, `approve <timeline> <snapshot> --key-id <id>` (sign a head-bound attestation; enforced locally)
- **Config**: `secret put|reveal|rotate|rekey|list|grant-actor|revoke-actor` (grant/revoke = per-actor key wrapping so each actor reads with their own key), `env create|set|set-secret|show|list`
- **Export**: `git export --to <path>` (one-way projection to a bare Git repo)
- **Plumbing**: `object`, `ref`, `store`

## Reference syntax (anywhere a snapshot is expected)

`@` = bound timeline head · `@<name>` = timeline head · `@tag:<name>` = tag
target · 64 hex chars = object id · `<name>` = timeline head or tag target.

## Glob syntax (paths and timeline patterns)

`*` matches within one segment (not across `/`); `**` matches zero or more whole
segments, including mid-pattern (`a/**/z`); trailing `**` is descendants-only
(`src/**` ≠ bare `src`); matching is case-sensitive.

## In-memory backend (library)

The in-RAM backend is a co-equal way to use bole, not a fallback. Drive the
library directly (Rust, in-process) and work entirely in memory:
`Repository::memory()` plus `repo.ephemeral_workspace()` (or
`ephemeral_workspace_from(snapshot)`) give a `read`/`write`/`remove`/`diff`/`commit`
worktree over in-memory buffers — no `.bole/`, no filesystem. `commit` returns a
snapshot id (it does not move a timeline; call `advance_timeline` for that). The
same actors, ACLs, secrets, and policy apply. See `docs/API.md` → *In-memory
workspaces*.

## When unsure

Run `bole <command> --help`, or read `docs/CLI.md` (full CLI reference),
`docs/API.md` (library API, incl. the in-memory backend), and
`bole-cli/README.md` (flags + worked examples) in the bole repo.
