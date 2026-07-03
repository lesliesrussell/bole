<!-- bole-yfc -->
# bole CLI Reference

`bole` is the command-line interface to the [bole](../README.md) version-control
library. It is a thin wrapper: every command maps onto the library's
`Repository`, `ObjectStore`, `RefStore`, and `AclStore` APIs.

The binary lives in the `bole-cli` workspace member. Build and install it with:

```bash
cargo build --release -p bole-cli
# binary at target/release/bole
```

## Mental model

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

bole puts access control *inside* the repository, not outside it. What Git cannot
express natively: an actor that can only see `src/**` and produce snapshots on
`agent/**`, while `main` is protected and requires an explicit merge by an actor
with broader grants. bole expresses this in the repository itself, without a
hosting platform. An automated agent and a human developer are the same concept —
just different grant sets.

- A **repository** is a `.bole/` directory. The directory containing it is the
  **work tree**. Commands discover the repository by walking up from the current
  directory, just like Git.
- A **snapshot** is an immutable file tree plus metadata. It is the only durable
  state.
- A **timeline** is a movable named pointer to a snapshot (like a branch) with an
  advancement policy (`ff` / `append` / `unrestricted`). A **tag** is a fixed
  pointer.
- An **actor** is a named set of path/timeline grants evaluated against the label
  lattice. The bound actor is the identity used for access-controlled operations;
  `Accessor` enforces its grants at the API boundary, with none bound the CLI has
  full access.
- A **secret** is an encrypted object in the same content-addressed store as
  source files, readable only by cleared actors.

## Global flags

| Flag | Effect |
|------|--------|
| `--json` | Emit machine-readable JSON instead of human text. |
| `--quiet` | Suppress non-error output. |

## Reference syntax

Commands that take a snapshot accept any of:

| Form | Meaning |
|------|---------|
| `@` | head of the currently-bound timeline |
| `@<name>` | head of timeline `<name>` |
| `@tag:<name>` | target of tag `<name>` |
| 64 hex chars | that object id verbatim |
| `<name>` | head of timeline `<name>` (or target if a tag) |

## Glob syntax

Path globs (`acl path`, `actor grant-path`) and timeline patterns (`acl
timeline`, `actor grant-timeline`) use the same matcher:

- `*` matches within a single path segment (does not cross `/`); it may match zero characters.
- `**` matches zero or more whole segments, including mid-pattern: `secrets/**`, `**/key`, `a/**/z` all work.
- A trailing `**` matches descendants only: `src/**` covers `src/main.rs` but not bare `src`.
- Matching is case-sensitive.

## CLI-local state

Beyond the library stores, the CLI keeps small JSON files under `.bole/`:

| File | Contents |
|------|----------|
| `cli-state.json` | current bound timeline and actor |
| `actors.json` | named actors and their grants |
| `secrets.json` | secret name → object id |
| `envs.json` | overlay name → object id |
| `policy-hooks.json` | configured policy hooks (signed-approval requirements) |

---

## Commands

### Repository lifecycle

```bash
bole init [path]            # create .bole/ (default: current directory)
bole status                 # work tree, binding, ref count
bole repo info              # paths, backend, object/ref counts, binding
```

### Timelines

```bash
bole timeline list
bole timeline create <name> --from <snap> [--policy ff|append|unrestricted] \
    [--kind persistent] [--expires-at <unix>]
bole timeline show <name>
bole timeline advance <name> --to <snap>
bole timeline delete <name>
# aliases:
bole branch ...             # = timeline ...
bole branches              # = timeline list
```

The policy is enforced on `advance` (and on the auto-advance done by
`snapshot create`): under `ff` or `append` the new head must be a descendant of
the current head, otherwise the advance is rejected. `unrestricted` allows any
snapshot.

### Tags

```bash
bole tag create <name> --target <snap> [--message <msg>]
bole tag list
bole tag show <name>
bole tag delete <name>
```

### Snapshots

```bash
bole snapshot create --from-workspace -m <msg> [--author <a>] [--no-advance]
bole snapshot show <snap>
bole snapshot list [--timeline <name>] [--limit <n>]
bole snapshot parents <snap>
bole snapshot diff <a> <b>
```

`snapshot create --from-workspace` builds a tree from the work tree and, when a
timeline is bound, advances it to the new snapshot (unless `--no-advance`).

### Workspace

```bash
bole workspace open <timeline> [--create --from <snap>] [--as <actor>]
bole workspace show          # binding + pending changes
bole workspace diff          # work tree vs bound head
bole workspace materialize --snapshot <snap> --to <dir>
bole workspace clear         # unbind

# Linked worktrees — many directories sharing one .bole/ store, each bound
# to a different timeline (like `git worktree`, minus the ref machinery):
bole workspace add <path> --timeline <name> [--as <actor>]   # timeline must exist
bole workspace list [--check]  # primary + linked: path, timeline, head, status
bole workspace remove <path>   # unregister; never deletes your files

# Hardening — reconcile a registry that has gone stale (moved/deleted dirs):
bole workspace prune [--dry-run] [--include-recoverable]     # drop unverifiable entries
bole workspace repair [--dry-run]                            # R1: fix a moved store
bole workspace repair --moved-to <new-path> <id>            # R2: a moved worktree dir
bole workspace repair --adopt <path>                        # R3: adopt an orphaned pointer
```

A linked worktree is a directory containing a `.bole` **file** that points at
the primary store; its binding lives under `<store>/worktrees/<id>/`. Commands
run from inside it resolve the shared store but its own timeline binding, so the
primary and each linked worktree can sit on different timelines at once.

If you delete or move a linked directory (or the primary store) outside of bole,
the registry can go stale. `workspace list` annotates stale entries
(`[STALE: missing-directory]`, `[STALE: wrong-store]`, …) and `list --check`
exits non-zero if any are stale. `workspace prune` drops entries that can no
longer be verified (removing only bookkeeping — the metadata dir and a bad
`.bole` pointer — never your other files), and `workspace repair` reconciles the
recoverable cases: a moved primary store (`repair`), a moved worktree directory
(`repair --moved-to`), or an orphaned pointer with no registry entry
(`repair --adopt`).

### Merge

```bash
bole merge check <source> <target>          # dry run (ACL check)
bole merge run <source> <target> [-m <msg>] # merge; advances target if clean
```

A clean merge stores a snapshot with both heads as parents and advances the
target. Conflicts are reported and the target is left unchanged.

### Actors

```bash
bole actor create <name>
bole actor grant-path <name> <glob> <read|write>
bole actor grant-timeline <name> <pattern> <read|write>
bole actor show <name>
bole actor list
bole actor use <name>        # bind the CLI to act as this actor
bole actor current
```

### Access control

```bash
bole acl path protect <glob>
bole acl path unprotect <glob>
bole acl path list
bole acl timeline protect <pattern>
bole acl timeline unprotect <pattern>
bole acl timeline list
bole acl can-read-path --actor <name> <path>
bole acl can-write-path --actor <name> <path>
bole acl can-read-timeline --actor <name> <timeline>
bole acl can-write-timeline --actor <name> <timeline>
bole acl explain-path --actor <name> [--snapshot <snap>] <path>
```

`can-*` answer a bare yes/no. `explain-path` returns the full decision trace for a
path at a snapshot (default: bound timeline head): whether the path is present,
its effective label, the protection rules that set that label, and — for both
read and write — the verdict, a reason, and every clearance the actor holds with
the deciding one flagged. This is the answer to "why is this path hidden from
this actor?". Add `--json` for the machine-readable trace.

### Signed approvals

Gate advances/merges into protected timelines on N distinct **signed** approvals
of the exact resulting head. Approvers are Ed25519 keys registered in a
content-addressed registry; each approval is a head-bound attestation.

```bash
bole policy require-approval <pattern> [--needed <n>]   # require n signed approvals for <pattern>
bole policy unrequire <pattern>                         # drop the requirement
bole policy list                                        # show configured policy hooks

bole approver add <key-id> --public-key <64hex>         # register a raw public key
bole approver add <key-id> --seed <64hex>               # ...or derive it from a seed
bole approver list

bole approve <timeline> <snapshot> --key-id <id> \      # sign an approval as <id>
    [--key-env BOLE_APPROVER_KEY] [--key-file <path>]   #   seed from env (default) or file
```

Policy hooks are stored in `<store>/policy-hooks.json` and loaded on every
invocation, so `timeline advance` / `merge run` / `snapshot create` (which
advances) into a matching timeline are refused until `count` distinct valid
approvals of that exact head exist. `approve` refuses to sign as an unregistered
`key-id`. Because approvals live in the repo's mutable ref namespace, this is
enforced **locally**; a replicated push into an approval-gated timeline is refused
fail-closed rather than enforced remotely (see the
[threat model](THREAT_MODEL.md)).

### Secrets

Keys are 64 hex characters (32 bytes), supplied via `--key-env <VAR>` (default
`BOLE_KEY`) or `--key-file <path>`. Key material is never stored in the
repository.

Secrets are stored as envelope-encrypted objects: a random per-secret data key
encrypts the value, and that data key is wrapped by the master key resolved from
the flags above. `secret rekey` rotates the master key by re-wrapping data keys —
the value ciphertext is never touched.

```bash
bole secret put <name> --from-stdin | --from-file <path>
bole secret reveal <name>
bole secret rotate <name> --from-stdin | --from-file <path>   # new value
bole secret rekey [--all | <name>...] \                       # rotate the master key
    --from-key-env <VAR> [--from-key-file <path>] \
    --to-key-env <VAR>   [--to-key-file <path>]
bole secret list
bole secret grant-actor <name> \                              # give another actor read access
    [--key-env <VAR>] [--key-file <path>] \                   #   your key (must already read)
    --recipient-key-env <VAR> [--recipient-key-file <path>]   #   the recipient's key
bole secret revoke-actor <name> \                             # drop an actor's read access
    --recipient-key-env <VAR> [--recipient-key-file <path>]
```

`grant-actor` moves a secret from a single shared key toward *per-actor* keys: the
value's data key is wrapped separately for each recipient, so each actor decrypts
with only their own key. The first `grant-actor` on a plain secret upgrades it to
multi-recipient (keeping the granter as a reader); subsequent grants add a wrap
without re-encrypting the value. `revoke-actor` drops a recipient's wrap
(identified by their key fingerprint) — this is *forward* revocation, so pair it
with `secret rotate` to defeat a reader who already extracted the value. The last
remaining recipient cannot be revoked.

### Environment overlays

```bash
bole env create <name>
bole env set <name> <var> <value>
bole env set-secret <name> <var> <secret-name>
bole env show <name>         # secret-backed values shown as <secret>
bole env resolve <name> [--reveal] [--skip-unauthorized]   # concrete env; redacts by default
bole env list
```

`env resolve` redacts secret-backed values by default (safe to paste); `--reveal`
decrypts them, which is access-checked — an actor not cleared for a secret's
label is refused (or the var is omitted with `--skip-unauthorized`).

### Run a command with an overlay

```bash
bole run --env <name> [--clean] [--skip-unauthorized] [key flags] -- <cmd> [args...]
```

Resolves the overlay (access-checked) and executes `<cmd>` with its variables
injected. Defaults to inheriting the parent environment; `--clean` starts from an
empty one. Secrets live only in the child's environment block; bole's own output
never prints them.

### Git export / import

```bash
bole git export --to <path>          # one-way projection to a bare Git repo
bole git import <path> \             # import branches + tags from a local Git repo
    [--branch <name>]... \           #   only these branches (default: all)
    [--timeline-policy ff|append|unrestricted] \
    [--label-ruleset <file>] \       #   apply WS1 path protection rules on import
    [--dry-run] [--force]            #   --force allows a non-fast-forward re-import
```

Import keeps an identity map under `.bole/git-map/` so re-importing after upstream
pushes only translates new commits and advances the affected timelines.

**Export visibility.** `git export` projects every timeline the bound actor may
read, and within each snapshot only the paths that actor may read. Visibility
follows the label model: an **unprotected** timeline is *public* — exported for
any actor — while a timeline **protected** by `acl timeline protect` is exported
only for an actor holding a matching `grant-timeline`. With no actor bound (`actor
current` shows none) the CLI has full access and exports everything. So a bound
actor with only path grants still sees public timelines; if an export looks empty,
check whether the bound actor is cleared for the timelines you expect
(`acl can-read-timeline --actor <a> <name>`, and `acl timeline list` for what's
protected).

### Plumbing

```bash
bole object list
bole object get <id>
bole object type <id>
bole object cat <id>         # blob bytes to stdout
bole object put-blob <file>

bole ref list [<prefix>]
bole ref get <name>
bole ref delete <name>

bole store stats                       # object/ref counts (cheap on packs)
bole store fsck                        # verify every object decodes
bole store repack                      # consolidate loose objects into a pack
bole store gc [--grace-secs <n>]       # collect unreachable objects (refs + registries as roots)
```

`store repack` folds loose objects into an immutable pack; `store gc` mark-sweeps
from the ref store plus the secret/env registries, protecting objects written
within the grace window (default 2h).

---

## Worked example

```bash
# Initialise and make the first snapshot.
bole init .
echo 'fn main() {}' > src/main.rs
bole snapshot create --from-workspace -m "initial"   # prints <snap>

# Put it on a timeline and bind the work tree to it.
bole workspace open main --create --from <snap>

# Define a restricted agent and act as it.
bole actor create formatter
bole actor grant-path formatter "src/**" write
bole actor grant-timeline formatter "agent/**" write
bole timeline create agent/fmt --from main
bole workspace open agent/fmt --as formatter

# Make changes and commit (advances agent/fmt).
echo 'fn main() { }' > src/main.rs
bole snapshot create --from-workspace -m "format"

# Review and merge back as a privileged identity.
bole actor use admin           # an actor granted ** / **
bole merge check agent/fmt main
bole merge run agent/fmt main -m "merge formatter output"

# Export to Git.
bole git export --to /tmp/export.git
```
