# Using the bole CLI

These instructions tell you how to version-control a project with the `bole`
CLI. Apply them whenever you work in a directory that contains (or should
contain) a `.bole/` repository, or when the user asks you to snapshot, branch,
merge, or otherwise manage history with bole.

`bole` is **not** Git. Its model is:

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

More precisely: bole puts access control **inside** the repository. Named actors
carry path and timeline grants; the CLI binds an actor before any
access-controlled operation; secrets are encrypted objects in the same store as
source files. Automated agents and human developers are the same concept — just
different grant sets.

- **Repository** — a `.bole/` directory; its parent is the **work tree**.
  Commands discover the repo by walking up from the current directory.
- **Snapshot** — an immutable file tree + metadata; the only durable state.
- **Timeline** — a movable named pointer to a snapshot (≈ a branch).
- **Tag** — a fixed named pointer to a snapshot.
- **Actor** — a named bundle of path/timeline grants; the bound actor is the
  identity used for access checks (no actor bound ⇒ full access).
- **Secret / Env overlay** — an encrypted value / a named variable bundle, each
  referenced by a CLI-local name.

## Rules to follow

- Every `bole` invocation is a separate process; all state lives in `.bole/`.
  There is no in-memory or session mode.
- When you need to parse output, pass `--json` — it is the stable contract.
  Use `--quiet` to suppress non-error text.
- Snapshots are built from the **work tree**; there is no staging/`add` step.
- Timeline policy is enforced: advancing a `ff` or `append` timeline to a
  non-descendant snapshot fails. `unrestricted` accepts any snapshot.
- Secrets require a 64-hex (32-byte) key via `$BOLE_KEY` or `--key-file`; never
  write key material into the repo.
- `--as <actor>` binds capability for all workspace operations: an agent scoped
  to `src/**` write cannot see or touch `secrets/**`, enforced at the API level.
- `workspace prune` / `workspace repair` clean up stale or moved linked worktree
  registrations; run them after moving directories or the store
  (`workspace list --check` flags staleness).
- More commands: `env resolve <name> [--reveal]` and `run --env <name> -- <cmd>`
  (access-checked env injection), `secret rekey` (rotate the master key),
  `git import <path>` (import a Git repo), `store repack` / `store gc`.
- Prefer `bole <command> --help` when unsure of a flag, rather than guessing.

## Core workflow

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

## Command map

- Lifecycle: `init`, `status`, `repo info`
- History: `snapshot create|show|list|parents|diff`, `timeline create|list|show|advance|delete` (alias `branch`/`branches`), `tag create|list|show|delete`
- Work tree: `workspace open|show|diff|materialize|clear|add|list|remove` (add/list/remove = linked worktrees sharing one store)
- Merge: `merge check <src> <dst>` (dry run), `merge run <src> <dst>` (advances dst when clean; reports conflicts otherwise)
- Access: `actor create|grant-path|grant-timeline|use|show|list`, `acl path|timeline protect|unprotect|list`, `acl can-{read,write}-{path,timeline}`, `acl explain-path --actor <a> <path>` (full read/write decision trace — the "why is this hidden?" answer)
- Config: `secret put|reveal|rotate|rekey|list|grant-actor|revoke-actor` (grant/revoke = per-actor key wrapping so each actor reads with their own key), `env create|set|set-secret|show|list`
- Export: `git export --to <path>` (one-way projection to a bare Git repo)
- Plumbing: `object`, `ref`, `store`

## Reference syntax (anywhere a snapshot is expected)

`@` = bound timeline head · `@<name>` = timeline head · `@tag:<name>` = tag
target · 64 hex chars = object id · `<name>` = timeline head or tag target.

## Glob syntax (paths and timeline patterns)

`*` matches within one segment (not across `/`); `**` matches zero or more whole
segments, including mid-pattern (`a/**/z`); a trailing `**` matches descendants
only (`src/**` does not match bare `src`); matching is case-sensitive.

## In-process agents (library, not CLI)

If you drive the bole **library** directly (Rust, in-process) instead of the
CLI, you can work entirely in RAM: `Repository::memory()` plus
`repo.ephemeral_workspace()` (or `ephemeral_workspace_from(snapshot)`) gives a
`read`/`write`/`remove`/`diff`/`commit` worktree over in-memory buffers — no
`.bole/`, no filesystem. `commit` returns a snapshot id (it does not move a
timeline; call `advance_timeline` for that). See `docs/API.md` → *In-memory
workspaces*.

## Further detail

Full reference: `docs/CLI.md`. Flags and worked examples: `bole-cli/README.md`.
