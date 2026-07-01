<!-- bole-ou7 -->
# bole-cli — the bole CLI

The command-line interface to [bole](../README.md), a version control library for
multi-actor, access-controlled workflows. `bole-cli` is a thin wrapper: every
command maps directly onto the library's `Repository`, `ObjectStore`, `RefStore`,
and `AclStore` APIs. Access control — which actors can see which files and
timelines — is enforced by the library at the API boundary, not by the CLI. The
crate produces a single binary named **`bole`**.

- Full prose reference: [`docs/CLI.md`](../docs/CLI.md)
- Library overview: [`../README.md`](../README.md)

## Install

```bash
cargo build --release -p bole-cli     # binary at target/release/bole
# optional: put it on your PATH
cp target/release/bole ~/.local/bin/
```

## Mental model

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

| Concept | What it is |
|---------|-----------|
| **Repository** | a `.bole/` directory; the directory containing it is the **work tree** |
| **Snapshot** | an immutable file tree + metadata — the only durable state |
| **Timeline** | a movable named pointer to a snapshot (like a branch) |
| **Tag** | a fixed named pointer to a snapshot |
| **Worktree** | a directory with a `.bole/` store (primary) or a `.bole` pointer file (linked); many directories can share one store, each on a different timeline |
| **Actor** | a named bundle of path/timeline grants; the bound actor is the identity used for access checks |
| **Secret** | an encrypted value, addressed by a CLI-local name |
| **Env overlay** | a named bundle of environment variables (plain or secret-backed) |

Commands discover the repository by walking up from the current directory, just
like Git.

## Global flags

These apply to every command:

| Flag | Effect |
|------|--------|
| `--json` | Emit machine-readable JSON instead of human-readable text. |
| `--quiet` | Suppress non-error output. |
| `-h`, `--help` | Show help for the binary or any subcommand. |
| `-V`, `--version` | Print the version. |

## Reference syntax

Anywhere a snapshot is expected (`--from`, `--to`, `--target`, `snapshot show`,
`merge`, diff arguments, …) you may use:

| Form | Meaning |
|------|---------|
| `@` | head of the currently-bound timeline |
| `@<name>` | head of timeline `<name>` |
| `@tag:<name>` | target of tag `<name>` |
| 64 hex chars | that object id, verbatim |
| `<name>` | head of timeline `<name>` (or target if `<name>` is a tag) |

## Glob syntax

Path globs (`acl path`, `actor grant-path`) and timeline patterns (`acl
timeline`, `actor grant-timeline`) share one matcher:

- `*` matches within a single segment (does not cross `/`); may match zero characters.
- `**` matches zero or more whole segments, including mid-pattern (`secrets/**`, `**/key`, `a/**/z`).
- A trailing `**` matches descendants only (`src/**` matches `src/main.rs`, not bare `src`).
- Matching is case-sensitive.

## CLI-local state

In addition to the library's stores, the CLI keeps small JSON files under
`.bole/`. None of them contain key material.

| File | Contents |
|------|----------|
| `cli-state.json` | currently bound timeline and actor |
| `actors.json` | named actors and their grants |
| `secrets.json` | secret name → object id |
| `envs.json` | overlay name → object id |

---

# Command reference

Every command accepts the global flags above; only command-specific flags are
listed below. Arguments in `<angle brackets>` are required; `[brackets]` are
optional.

## Repository lifecycle

### `bole init [path]`
Create a new repository (`.bole/`) under `path` (default: current directory).
Errors if `.bole/` already exists.

### `bole status`
Print the work tree, repository path, bound timeline, bound actor, and ref
count.

### `bole repo info`
Print repository paths, backend, object/ref counts, and the current binding.

## Timelines

`branch` is an alias for `timeline`; `branches` is an alias for `timeline list`.

### `bole timeline list`
List all timelines (name, short head, policy).

### `bole timeline create <name>`
Create a timeline pointing at a snapshot.

| Flag | Default | Meaning |
|------|---------|---------|
| `--from <snap>` | *(required)* | snapshot the timeline points at |
| `--policy <ff\|append\|unrestricted>` | `unrestricted` | advancement policy |
| `--kind <string>` | `persistent` | lifecycle category (e.g. `ephemeral`) |
| `--expires-at <unix>` | *(none)* | timestamp after which it may be pruned |

### `bole timeline show <name>`
Show a timeline's head, policy, kind, created-at, and expiry.

### `bole timeline advance <name> --to <snap>`
Move a timeline's head to another snapshot. Enforced against the bound actor's
write permissions **and** the timeline's policy: under `ff`/`append` the new
head must be a descendant of the current head (otherwise the advance is
rejected); `unrestricted` allows any snapshot.

### `bole timeline delete <name>`
Delete a timeline.

## Tags

### `bole tag create <name> --target <snap> [--message <msg>]`
Create an immutable tag pointing at a snapshot.

### `bole tag list`
List all tags (name, short target, message).

### `bole tag show <name>`
Show a tag's target, created-at, and message.

### `bole tag delete <name>`
Delete a tag.

## Snapshots

### `bole snapshot create --from-workspace -m <msg>`
Build a snapshot from the current work tree.

| Flag | Default | Meaning |
|------|---------|---------|
| `--from-workspace` | *(required)* | source the tree from the work tree |
| `-m`, `--message <msg>` | *(required)* | commit message |
| `--author <name>` | `$BOLE_AUTHOR`, then `$USER`, then `unknown` | author |
| `--no-advance` | off | do **not** advance the bound timeline |

When a timeline is bound, the snapshot's parent is its head and the timeline is
advanced to the new snapshot (unless `--no-advance`).

### `bole snapshot show <snap>`
Show author, created-at, message, parents, and file count.

### `bole snapshot list [--timeline <name>] [--limit <n>]`
Walk a timeline's history newest-first (defaults to the bound timeline;
`--limit` defaults to 50).

### `bole snapshot parents <snap>`
Print a snapshot's parent ids.

### `bole snapshot diff <a> <b>`
Show added / removed / modified paths going from snapshot `a` to `b`.

## Workspace

### `bole workspace open <timeline>`
Bind the work tree to a timeline and materialise its head into the work tree.

| Flag | Meaning |
|------|---------|
| `--create` | create the timeline first (requires `--from`) |
| `--from <snap>` | snapshot to create the timeline at |
| `--as <actor>` | also bind the CLI to act as this actor |

### `bole workspace show`
Show the bound timeline/actor/head and pending changes against the head.

### `bole workspace diff`
Show how the work tree differs from the bound timeline's head.

### `bole workspace materialize --snapshot <snap> --to <dir>`
Write a snapshot's files into a directory (read-only export).

### `bole workspace clear`
Unbind the work tree from its timeline.

### `bole workspace add <path> --timeline <name> [--as <actor>]`
Create a **linked worktree**: a new directory bound to an existing timeline that
shares this repo's `.bole/` store, with the timeline's head materialized into it.
Many directories can share one store, each on a different timeline (like
`git worktree`). The directory gets a `.bole` pointer file; the binding lives
under `<store>/worktrees/<id>/`.

### `bole workspace list`
List the primary worktree and all linked worktrees: path, bound timeline, and
short head (linked ones are marked).

### `bole workspace remove <path>`
Unregister a linked worktree and delete its pointer file and metadata. It
**never** deletes your working files.

## Merge

### `bole merge check <source> <target>`
Dry run: report whether merging `source` into `target` is `allowed`,
`requires-approval`, or `rejected` (with the protected paths involved).

### `bole merge run <source> <target> [-m <msg>]`
Perform the three-way merge. On a clean merge, store a snapshot with both heads
as parents and advance `target`. On conflict, report the conflicting paths and
leave `target` unchanged (non-zero exit). `--message` defaults to `merge`.

## Actors

### `bole actor create <name>`
Create a new, empty actor.

### `bole actor grant-path <name> <glob> <read|write>`
Grant a path role (e.g. `src/**` `write`).

### `bole actor grant-timeline <name> <pattern> <read|write>`
Grant a timeline role (e.g. `agent/**` `write`).

### `bole actor show <name>` / `bole actor list`
Show one actor's grants / list all actor names.

### `bole actor use <name>`
Bind the CLI to act as this actor for subsequent access-controlled operations.

### `bole actor current`
Print the currently-bound actor.

## Access control

### `bole acl path protect <glob>` / `unprotect <glob>` / `list`
Manage path-protection rules.

### `bole acl timeline protect <pattern>` / `unprotect <pattern>` / `list`
Manage timeline-protection rules.

### `bole acl can-read-path --actor <name> <path>`
### `bole acl can-write-path --actor <name> <path>`
### `bole acl can-read-timeline --actor <name> <timeline>`
### `bole acl can-write-timeline --actor <name> <timeline>`
Test whether an actor is permitted an operation (prints `allowed`/`denied`;
`{"allowed": bool}` in JSON).

## Secrets

Keys are **64 hex characters** (32 bytes). Supply via `--key-env <VAR>` (default
`BOLE_KEY`) or `--key-file <path>`. Key material is never stored in the
repository.

### `bole secret put <name> (--from-stdin | --from-file <path>)`
Encrypt and store a secret under a name. `put` errors if the name exists.

| Flag | Default | Meaning |
|------|---------|---------|
| `--from-stdin` | — | read plaintext from stdin |
| `--from-file <path>` | — | read plaintext from a file |
| `--key-env <VAR>` | `BOLE_KEY` | env var holding the hex key |
| `--key-file <path>` | — | file holding the hex key |

### `bole secret reveal <name>`
Decrypt and print a secret (same key flags).

### `bole secret rotate <name> (--from-stdin | --from-file <path>)`
Replace an existing secret's value (same flags as `put`; errors if absent).

### `bole secret list`
List secret names.

## Environment overlays

Overlays are immutable; each edit stores a new overlay object and repoints the
name at it.

### `bole env create <name>`
Create an empty overlay.

### `bole env set <name> <var> <value>`
Set a plaintext variable.

### `bole env set-secret <name> <var> <secret-name>`
Point a variable at a named secret.

### `bole env show <name>`
Show the overlay; secret-backed values are shown as `<secret>`.

### `bole env list`
List overlay names.

## Git export

### `bole git export --to <path>`
One-way projection of the repository to a bare Git repo at `path` (created if
absent; must be a bare repo if it exists). Filtered by the bound actor.

## Plumbing

Low-level access for debugging and scripting.

### Objects
```
bole object list                 # all object ids
bole object get <id>             # decoded structure
bole object type <id>            # blob | tree | snapshot | secret | env-overlay
bole object cat <id>             # blob raw bytes to stdout
bole object put-blob <file>      # store a file, print its id
```

### Refs
```
bole ref list [<prefix>]
bole ref get <name>              # kind + target
bole ref delete <name>
```

### Store
```
bole store stats                 # object/ref/acl counts
bole store fsck                  # verify every object decodes
```

---

# Worked examples

## 1. Solo flow: init, snapshot, iterate

```bash
bole init .
echo 'fn main() {}' > src/main.rs

# First snapshot (no timeline yet) and put it on `main`.
SNAP=$(bole snapshot create --from-workspace -m "initial" --json | jq -r .snapshot)
bole workspace open main --create --from "$SNAP"

# Edit and see pending changes.
echo 'fn main() { println!("hi"); }' > src/main.rs
bole workspace diff            # ~ src/main.rs

# Commit (advances main) and review history.
bole snapshot create --from-workspace -m "say hi"
bole snapshot list            # newest first
```

## 2. Multi-actor flow with ACL enforcement

```bash
# Protect a sensitive area and define a restricted agent.
bole acl path protect "secrets/**"

bole actor create formatter
bole actor grant-path formatter "src/**" write
bole actor grant-timeline formatter "agent/**" write

bole actor create admin
bole actor grant-path admin "**" write
bole actor grant-timeline admin "**" write

# Agent works on its own branch.
bole timeline create agent/fmt --from @main
bole workspace open agent/fmt --as formatter
echo 'fn main() { }' > src/main.rs
bole snapshot create --from-workspace -m "format"   # OK: formatter may write src/** and agent/**

# Agent cannot advance main:
bole acl can-write-timeline --actor formatter main  # denied

# Admin reviews and merges back.
bole actor use admin
bole merge check agent/fmt main
bole merge run   agent/fmt main -m "merge formatter output"
```

## 3. Secrets and environment overlays

```bash
export BOLE_KEY=$(openssl rand -hex 32)    # 64 hex chars

# Store and read a secret.
printf 'postgres://user:pw@db/app' | bole secret put prod/db/url --from-stdin
bole secret reveal prod/db/url
bole secret list

# Build an overlay that mixes plaintext and a secret reference.
bole env create dev
bole env set        dev RUST_LOG debug
bole env set-secret dev DATABASE_URL prod/db/url
bole env show dev                          # DATABASE_URL shown as <secret>

# Rotate the secret; the overlay reference still resolves.
printf 'postgres://user:newpw@db/app' | bole secret rotate prod/db/url --from-stdin
```

## 4. Tags and Git export

```bash
# Tag a release and export to Git for downstream tooling.
bole tag create v1.0 --target @main --message "first release"
bole tag list

bole git export --to /tmp/bole-export.git
git -C /tmp/bole-export.git log --oneline
```

## 5. Scripting with `--json`

```bash
# Drive the CLI from a script using stable JSON output.
head=$(bole timeline show main --json | jq -r .head)
count=$(bole store stats --json | jq .objects)
echo "main @ $head, $count objects"

# Fail a CI check if the work tree has uncommitted changes.
pending=$(bole workspace diff --json | jq '(.added|length)+(.removed|length)+(.modified|length)')
[ "$pending" -eq 0 ] || { echo "uncommitted changes"; exit 1; }
```
