# Using bole

`bole` is a content-addressed version-control **library** with a `bole` CLI on
top of it. Apply these instructions whenever you work in a directory that
contains (or should contain) a `.bole/` repository, when you drive the bole
library in-process, or when the user asks you to snapshot, branch, merge, or
otherwise manage history with bole.

`bole` is **not** Git. Its model is:

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

## Two first-class backends, one object model

Same object model over either backend — pick per use case:

- **In-memory (library, in-process):** `Repository::memory()` +
  `repo.ephemeral_workspace()` give a `read`/`write`/`remove`/`diff`/`commit`
  working tree entirely in RAM — no `.bole/`, no filesystem. For in-process
  agents and tests (see *In-process agents*).
- **On disk (the `bole` CLI):** the CLI drives a `.bole/` directory for durable
  storage — linked worktrees, packs/GC, distributed sync, git interop.

Neither is a footnote; both are peer ways to use bole.

More precisely: bole puts access control **inside** the repository. Named actors
carry path and timeline grants; a bound actor gates every access-controlled
operation; secrets are encrypted objects in the same store as source files.
Automated agents and human developers are the same concept — just different grant
sets.

- **Repository** — the object/ref/ACL store: in RAM (`Repository::memory()`) or a
  `.bole/` directory on disk (`Repository::disk()` / the CLI). On disk, the
  parent of `.bole/` is the **work tree** and commands discover it by walking up
  from the current directory.
- **Snapshot** — an immutable file tree + metadata; the only durable state.
- **Timeline** — a movable named pointer to a snapshot (≈ a branch).
- **Tag** — a fixed named pointer to a snapshot.
- **Actor** — a named bundle of path/timeline grants; the bound actor is the
  identity used for access checks (no actor bound ⇒ full access).
- **Secret / Env overlay** — an encrypted value / a named variable bundle, each
  referenced by a CLI-local name.

## Rules to follow

- The CLI has no session: every `bole` invocation is a separate process and CLI
  state lives in `.bole/` on disk. (In-process, the library backend keeps
  everything in RAM instead — see *In-process agents*.)
- When you need to parse output, pass `--json` — it is the stable contract.
  Use `--quiet` to suppress non-error text.
- Snapshots are built from the **work tree**; there is no staging/`add` step.
- A gitignore-syntax `.boleignore` at the work-tree root excludes matching files
  and directories from snapshots (ignored dirs are pruned whole); manage it with
  `bole ignore ...`. `.bole/` and symlinks are always excluded regardless, and
  `.boleignore` itself is a normal tracked, versioned file.
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
- Ignore: `ignore <pattern>...` (bare = add), `ignore list|remove <pattern>...|check <path>...` — gitignore-style `.boleignore` at the work-tree root; ignored files/dirs are excluded from snapshots. `check` is a dry-run using the same matcher as the snapshot walk.
- Merge: `merge check <src> <dst>` (dry run), `merge run <src> <dst>` (advances dst when clean; reports conflicts otherwise)
- Access: `actor create|grant-path|grant-timeline|use|show|list`, `acl path|timeline protect|unprotect|list`, `acl can-{read,write}-{path,timeline}`, `acl explain-path --actor <a> <path>` (full read/write decision trace — the "why is this hidden?" answer)
- Signed approvals: `policy require-approval <pattern> --needed <n>` / `policy list|unrequire`, `approver add <id> --public-key|--seed <64hex>` / `approver list`, `approve <timeline> <snapshot> --key-id <id>` (head-bound signed attestation; enforced locally)
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

## In-process agents (in-memory backend)

The in-RAM backend is a co-equal way to use bole, not a fallback. Drive the bole
**library** directly (Rust, in-process) and work entirely in RAM:
`Repository::memory()` plus
`repo.ephemeral_workspace()` (or `ephemeral_workspace_from(snapshot)`) gives a
`read`/`write`/`remove`/`diff`/`commit` worktree over in-memory buffers — no
`.bole/`, no filesystem. `commit` returns a snapshot id (it does not move a
timeline; call `advance_timeline` for that). See `docs/API.md` → *In-memory
workspaces*.

## Further detail

Full reference: `docs/CLI.md`. Flags and worked examples: `bole-cli/README.md`.
