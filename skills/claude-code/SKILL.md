---
name: using-bole
description: Use when version-controlling a project with the bole CLI — initializing a repo, creating snapshots from the work tree, managing timelines and tags, opening/diffing a workspace, merging, ACLs/actors, secrets, env overlays, or exporting to Git. Triggers on the `bole` command and `.bole/` repositories.
---

# Using the bole CLI

`bole` is a content-addressed version-control tool. It is **not** Git — the
nouns are different and map to bole's model:

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

| Concept | Meaning |
|---------|---------|
| Repository | a `.bole/` directory; its parent is the **work tree** |
| Snapshot | an immutable file tree + metadata — the only durable state |
| Timeline | a movable named pointer to a snapshot (≈ branch) |
| Tag | a fixed named pointer to a snapshot |
| Actor | a named bundle of path/timeline grants; the bound actor is the identity used for access checks |
| Secret / Env overlay | encrypted value / named variable bundle, addressed by a CLI-local name |

Commands discover the repo by walking up from the current directory (like Git).

## Critical facts

- **Every `bole` command is a separate process.** There is no in-memory/session
  mode; all state lives in `.bole/`. Don't expect anything to persist except
  through the repo.
- **Add `--json` for any output you need to parse.** It is the stable contract;
  human text is not. `--quiet` suppresses non-error output.
- **Snapshots are created from the work tree**, not staged files. There is no
  "add"/staging step.
- **Timeline policy is enforced**: `ff`/`append` timelines reject an advance to a
  non-descendant; `unrestricted` accepts any snapshot.
- **Secrets need a key**: a 64-hex (32-byte) key via `$BOLE_KEY` or
  `--key-file`. It is never stored in the repo.

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

- **Lifecycle**: `init`, `status`, `repo info`
- **History**: `snapshot create|show|list|parents|diff`, `timeline create|list|show|advance|delete` (alias `branch`/`branches`), `tag create|list|show|delete`
- **Work tree**: `workspace open|show|diff|materialize|clear|add|list|remove` (add/list/remove = linked worktrees sharing one store)
- **Merge**: `merge check <src> <dst>` (dry run), `merge run <src> <dst>` (advances dst when clean; reports conflicts otherwise)
- **Access**: `actor create|grant-path|grant-timeline|use|show|list`, `acl path|timeline protect|unprotect|list`, `acl can-{read,write}-{path,timeline}`
- **Config**: `secret put|reveal|rotate|list`, `env create|set|set-secret|show|list`
- **Export**: `git export --to <path>` (one-way projection to a bare Git repo)
- **Plumbing**: `object`, `ref`, `store`

## Reference syntax (anywhere a snapshot is expected)

`@` = bound timeline head · `@<name>` = timeline head · `@tag:<name>` = tag
target · 64 hex chars = object id · `<name>` = timeline head or tag target.

## Glob syntax (paths and timeline patterns)

`*` matches within one segment (not across `/`); `**` matches zero or more whole
segments, including mid-pattern (`a/**/z`); trailing `**` is descendants-only
(`src/**` ≠ bare `src`); matching is case-sensitive.

## When unsure

Run `bole <command> --help`, or read `docs/CLI.md` (full reference) and
`bole-cli/README.md` (flags + worked examples) in the bole repo.
