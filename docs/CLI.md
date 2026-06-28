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

- A **repository** is a `.bole/` directory. The directory containing it is the
  **work tree**. Commands discover the repository by walking up from the current
  directory, just like Git.
- A **snapshot** is an immutable file tree plus metadata. It is the only durable
  state.
- A **timeline** is a movable named pointer to a snapshot (like a branch). A
  **tag** is a fixed pointer.
- An **actor** is a named set of path/timeline grants. The bound actor is the
  identity used for access-controlled operations; with none bound the CLI has
  full access.

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

## CLI-local state

Beyond the library stores, the CLI keeps small JSON files under `.bole/`:

| File | Contents |
|------|----------|
| `cli-state.json` | current bound timeline and actor |
| `actors.json` | named actors and their grants |
| `secrets.json` | secret name → object id |
| `envs.json` | overlay name → object id |

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
```

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
```

### Secrets

Keys are 64 hex characters (32 bytes), supplied via `--key-env <VAR>` (default
`BOLE_KEY`) or `--key-file <path>`. Key material is never stored in the
repository.

```bash
bole secret put <name> --from-stdin | --from-file <path>
bole secret reveal <name>
bole secret rotate <name> --from-stdin | --from-file <path>
bole secret list
```

### Environment overlays

```bash
bole env create <name>
bole env set <name> <var> <value>
bole env set-secret <name> <var> <secret-name>
bole env show <name>         # secret-backed values shown as <secret>
bole env list
```

### Git export

```bash
bole git export --to <path>  # one-way projection to a bare Git repo
```

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

bole store stats
bole store fsck
```

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
