# WS7 — Linked-Worktree Hardening + Repositioning

- **Bead:** `bole-3hj`
- **Depends on:** `bole-fo2` (WS1), `bole-1kz` (WS2), `bole-9mz` (WS3)
- **Status:** design spec (not an implementation plan)
- **Conforms to:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md).
  Shared vocabulary (Label, LabelLattice, Clearance, Accessor, PolicyHook,
  Workspace trait, DiskWorkspace) is defined in the foundations doc and WS1–3
  specs and is not re-derived here.

---

## 1. Goal

WS7 has two deliverables that are logically separate but shipped together as the
capstone workstream.

**Part A — Linked-worktree hardening.** The linked-worktree feature (bole-hrk)
introduced in `workspace add/list/remove` has no defences against registry
staleness. If a user `rm -rf`s, `mv`s, or otherwise displaces a linked worktree
directory, `worktrees.json` accumulates phantom entries with no automatic
cleanup. WS7 adds `bole workspace prune`, `bole workspace repair`, consistency
annotations in `workspace list`, and move-detection for both worktree directories
and the primary store. No behavior change to existing commands; new commands only.

**Part B — Repositioning.** The current docs — README.md, docs/CLI.md,
docs/API.md, bole-cli/README.md, and the three agent skills — lead with
"content-addressed VCS, snapshots, timelines," which reads as "Git but nicer"
and buries the actual differentiator: that bole embeds multi-actor identity,
label-gated visibility, and policy-controlled operations into the object model
itself, not into a separate service. WS7 redesigns the doc narrative to lead with
that differentiator, keeps the snapshot/timeline/secret model as the
implementation story, and maintains strict honesty about what is realized today
versus what lands with WS1–5.

Non-goals: WS7 does not change how registrations are persisted (still JSON), does
not redesign the `Workspace` trait (WS2), does not change the policy evaluation
model (WS1), and does not add new secret or env commands (WS3).

---

## 2. Part A — Linked-Worktree Hardening

### 2.1 Failure modes in the current implementation

The current model stores each linked worktree as a `(id, Entry { path })` pair
in `<store>/worktrees.json`. A `.bole` pointer file at `<worktree>/.bole`
carries `{ store, id }` (the `Pointer` struct). The registry and pointer are
written atomically during `workspace add`, but no subsequent operation verifies
their continued consistency. The following failure modes are not handled:

| Case | What happened | Registry state | Pointer state |
|------|---------------|----------------|---------------|
| **Miss-dir** | User `rm -rf`'d the worktree directory | Path gone | Gone with it |
| **Miss-ptr** | User deleted only the `.bole` file | Path exists, pointer gone | Gone |
| **Bad-ptr** | Pointer is corrupt JSON or wrong schema | Path exists | Unreadable |
| **Wrong-store** | Primary store was moved; pointer has old absolute path | Intact | `store` stale |
| **Wrong-id** | User copied a linked worktree; pointer has a different id | Stale | Mismatched |
| **Orphan-meta** | Metadata dir exists but registry entry was manually removed | Absent | May exist |
| **Orphan-ptr** | Valid pointer but no registry entry (dir moved + registry entry was pruned) | Absent | Valid |

### 2.2 Staleness classification

All new commands share a common staleness classifier applied to each registry
entry. The classifier returns one of:

```
Ok
MissingDirectory              # entry.path does not exist as a directory
MissingPointer                # directory exists but <path>/.bole is absent or not a file
BadPointer(reason)            # .bole exists but is not valid JSON / not a Pointer struct
WrongStore { found, expected }# pointer.store != current repo_dir
WrongId { found, expected }   # pointer.id != registry key
Recoverable(RecoveryKind)     # inconsistent but a repair rule applies
```

The classifier is a pure function: `classify(repo_dir, registry_id, entry) -> Status`. It is called by `list`, `prune`, and `repair`, so the classification logic lives in exactly one place.

**Recoverable** is a subtype only relevant to `repair`. A recoverable entry is
one where the pointer's `id` matches the registry key but the `store` path is
wrong (implying the store was moved). This case is repairable automatically
because the id still links the pointer to the right metadata; only the store path
needs updating.

### 2.3 `bole workspace prune`

#### Purpose

Drop registry entries whose linked worktree can no longer be located or verified,
and clean up the corresponding metadata directories under
`<store>/worktrees/<id>/`. Does **not** touch the user's working files.

#### Prunable cases

An entry is prunable if its status is `MissingDirectory`, `MissingPointer`,
`BadPointer`, `WrongId`, or `WrongStore` when the store was NOT simply moved
(i.e., `pointer.id` also does not match, making the entry truly disconnected
from any recoverable state). The `Recoverable` status (store moved, id matches)
is not pruned by default; it is left for `repair`.

#### Command signature

```
bole workspace prune [--dry-run] [--include-recoverable] [--json]
```

| Flag | Meaning |
|------|---------|
| `--dry-run` | Print what would be pruned; do not modify anything |
| `--include-recoverable` | Also prune entries classified as `Recoverable` (store-path mismatch but id matches); use with caution |
| `--json` | Emit a JSON array of pruned entries |

#### Reconciliation rules

For each prunable entry `(id, entry)`:

1. Remove the metadata directory `<store>/worktrees/<id>/` and its contents (idempotent if already absent).
2. Remove the registry entry from `worktrees.json`.
3. If the worktree directory still exists and contains a `.bole` pointer file that was determined to be `BadPointer` or `WrongId`, remove **only** the `.bole` pointer file. The rest of the user's files in the directory are never touched.
4. Write the updated `worktrees.json`.
5. Report each pruned entry: `id`, `path`, `status`, `action`.

#### Output (human-readable)

```
pruned worktree feature/foo  (path: /home/user/projects/foo) — directory missing
pruned worktree old-exp      (path: /home/user/projects/exp) — pointer unreadable
2 entries pruned, 1 entry clean.
```

#### Output (JSON)

```json
[
  { "id": "feature-foo", "path": "/home/user/projects/foo",
    "status": "missing-directory", "pruned": true },
  { "id": "old-exp", "path": "/home/user/projects/exp",
    "status": "bad-pointer", "pruned": true },
  { "id": "dev", "path": "/home/user/projects/dev",
    "status": "ok", "pruned": false }
]
```

#### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Completed; zero or more entries pruned |
| 1 | No entries to prune (also 0 is acceptable; see Open question OQ1) |
| 2 | Unexpected I/O error |

### 2.4 `bole workspace repair`

#### Purpose

Reconcile pointer↔registry inconsistencies that are recoverable without data
loss. Three sub-cases are handled:

**Sub-case R1 — Store moved.** The primary `.bole/` store was relocated to a new
absolute path. All registered linked worktrees have pointer files whose `store`
field contains the old path. `repair` rewrites those pointer files to use the new
path.

**Sub-case R2 — Directory moved.** A linked worktree directory was moved to a new
location. The registry entry still has the old path. The user supplies the new
path explicitly.

**Sub-case R3 — Orphaned pointer (adopt).** A directory has a valid `.bole`
pointer pointing at this store, but there is no matching registry entry. This
happens when: (a) the registry entry was pruned but the directory survived; (b) a
directory was manually constructed. `repair --adopt` registers it.

#### Command signature

```
bole workspace repair [--dry-run] [--json]
bole workspace repair --moved-to <new-path>  <id>
bole workspace repair --adopt <path>
```

| Variant | Meaning |
|---------|---------|
| `repair` (no args) | Auto-detect and fix all R1 (store-moved) entries; report R2/R3 if found but leave them for explicit commands |
| `repair --moved-to <new-path> <id>` | Fix one R2 (directory moved) entry |
| `repair --adopt <path>` | Handle one R3 (orphaned pointer) entry |

#### R1 — Store moved: reconciliation rules

1. For each registry entry `(id, entry)`:
   - Read `<entry.path>/.bole`. If it does not exist or is unreadable, skip (let `prune` handle it).
   - Parse as `Pointer`. If `pointer.id == id` and `pointer.store != current repo_dir`: this is a store-moved case.
2. For each such entry: rewrite the pointer file with `{ store: current_repo_dir, id }`.
3. Report entries repaired.

This is safe to run idempotently. If the pointer already has the correct `store`, it is a no-op for that entry.

#### R2 — Directory moved: reconciliation rules

Invoked as `bole workspace repair --moved-to <new-path> <id>`:

1. Look up `id` in the registry. If not found: error ("no such worktree id; did you mean --adopt?").
2. Canonicalize `<new-path>` to an absolute path.
3. Read `<new-path>/.bole`:
   - Must exist and parse as a valid `Pointer`.
   - `pointer.store` must equal `current repo_dir` (within path normalization).
   - `pointer.id` must equal `id`.
   - If any check fails: error with the specific mismatch; do not modify anything.
4. Update the registry entry's `path` to the canonical new path.
5. Write `worktrees.json`.
6. Report: "repaired worktree `<id>`: path updated from `<old>` to `<new>`."

#### R3 — Orphaned pointer: reconciliation rules

Invoked as `bole workspace repair --adopt <path>`:

1. Canonicalize `<path>`.
2. Read `<path>/.bole`:
   - Must exist, parse as `Pointer`, and `pointer.store == current repo_dir`. Error if not.
3. Let `id = pointer.id`.
4. If `id` is already in the registry:
   - If `registry[id].path == canonical path`: already consistent; nothing to do (not an error).
   - If `registry[id].path != canonical path`: error ("id `<id>` is already registered at `<other-path>`; run `workspace remove` first or use `--moved-to` to update the path").
5. Add `registry[id] = { path: canonical path }`.
6. Ensure metadata dir `<store>/worktrees/<id>/` exists. If absent, create it with an empty `state.json` (`CliState::default()`).
7. Write `worktrees.json`.
8. Report: "adopted worktree `<id>` at `<path>`."

#### `--dry-run` for repair

All three sub-cases support `--dry-run`: print what would change without writing anything.

### 2.5 `workspace list` — consistency annotations

`workspace list` gains a lightweight consistency check on every run. The check
applies only to linked worktrees (the primary is always present by definition).

**Human output change:**

```
/home/user/projects/main         main     a3f1b2c4d5e6  (primary)
/home/user/projects/feature-x   feat/x   9d2e6a1b3c7f  (linked)
/home/user/projects/old-feature  feat/old  -            (linked) [STALE: missing-directory]
/home/user/projects/moved-store  feat/ms   -            (linked) [STALE: wrong-store]
```

**JSON output change (additive field):**

```json
[
  { "path": "...", "timeline": "feat/old", "head": null, "linked": true,
    "status": "missing-directory" },
  ...
]
```

The `status` field is always present: `"ok"` for clean entries; the classifier
string for stale ones. This is an additive field so existing consumers that
ignore unknown fields are not affected.

**Exit code for `workspace list`:**

By default, `list` exits 0 regardless of stale entries (non-breaking change for
scripts already using it). With `--check`, it exits 1 if any linked worktree is
stale. This follows the principle: informational commands are non-intrusive; the
caller opts into strict exit codes.

```
bole workspace list --check
```

### 2.6 Moved-directory and moved-store detection matrix

| Scenario | Detected by | Fix command |
|----------|-------------|-------------|
| `rm -rf worktree/` | `prune` (MissingDirectory) | `prune` |
| `rm worktree/.bole` | `prune` (MissingPointer) | `prune` |
| `mv worktree/ newloc/` | `list` annotates old entry stale; pointer at newloc is "orphaned" | `repair --moved-to newloc <id>` or `repair --adopt newloc` |
| `mv .bole/ newstore/` | `list` annotates all linked entries as WrongStore (via pointer check) | `repair` (auto, R1) |
| `cp -r worktree/ clone/` | `list` shows `clone/.bole` as orphaned (not in registry) | `repair --adopt clone/` |
| Corrupt `.bole` file | `prune` (BadPointer) | `prune` (drops entry); re-add manually if needed |
| Manual `rm worktrees.json` | `list` shows no linked entries; pointer files still exist | `repair --adopt <each path>` |

### 2.7 Interaction with WS2 `Workspace` / `DiskWorkspace`

The classifier accesses pointer files via standard `std::fs` calls, not through
the `DiskWorkspace` abstraction. This is correct: the classifier runs before a
`RepoContext` is fully established (it is used during `list`, before any timeline
binding is asserted). The `DiskWorkspace` abstraction covers the working content
of a worktree, not its registration metadata.

The `Registry`, `Entry`, `Pointer`, and per-worktree `CliState` types remain in
`worktrees.rs` and `context.rs` unchanged. The classifier is a new pure function
added to `worktrees.rs`.

### 2.8 Data model additions

No new persistent state is added. The classifier is entirely derived from the
existing `worktrees.json` and `.bole` pointer files. The only structural change
to on-disk data during `repair` is rewriting pointer file JSON (same schema) and
potentially creating missing metadata directories with empty `state.json`.

### 2.9 Backward compatibility (Part A)

Existing behavior of `workspace add`, `workspace list`, `workspace remove`, and
`workspace show` is unchanged. New commands (`prune`, `repair`) and new flags
(`list --check`) are purely additive. The new `status` field in `list --json` is
an additive key; existing JSON consumers unaffected by unknown keys are not
broken.

Existing `worktrees.json` schemas are unchanged. No migration required.

---

## 3. Part B — Repositioning

### 3.1 The critique — why the current docs underperform

The current README opens with:

> "A next-generation version control library crate for Rust, designed for
> fine-grained visibility, pluggable storage, typed secrets, multi-actor
> workflows, and backward-compatible Git export."

The five clauses are all implementation-internal nouns. A reader unfamiliar with
bole leaves the sentence not knowing what problem bole solves that Git does not.
The features table then enumerates mechanisms (content-addressing, timelines,
ACLs, secrets) with no context for why they matter.

The critique is not that these things are wrong — they are correct — but that
they are not the story. The story is what only bole can express natively:

1. **Access-controlled history**: the restriction "this actor can only see files
   matching `src/**`" is stored in the same object graph as the files and
   enforced at the library API boundary, not by a convention or a separate
   service.
2. **Approval-gated operations**: a `PolicyHook` can block a `merge` or
   `advance` until a named approver with a required clearance has signed off —
   encoded in a content-addressed policy object, transferable and verifiable.
3. **Agent-safe workflows**: an automated actor's capability bounds are declared
   in the repository, so an agent that escapes its allowed paths cannot
   accidentally write to protected timelines or read secrets above its clearance.
4. **First-class encrypted secrets in the history graph**: secrets are
   content-addressed objects in the same store as snapshots, with access gated by
   the same label model, decryptable only by cleared actors.

Git cannot express any of these things. Git has a commit DAG and filesystem
semantics; access control, approval workflows, and secret management all live
outside it.

### 3.2 Honesty constraint

WS1 (real label lattice, `LabelLattice`, content-addressed policy objects,
`PolicyHook` trait) is roadmap. Today's access model is a two-point lattice
(`public ⊑ protected`) expressed as glob rules and `PathRole`/`TimelineRole`
grants. WS3 envelope encryption and `KeyProvider` are roadmap. Today's secret
model is single-key ChaCha20-Poly1305.

The repositioned docs must:

- Lead with what the model *will express* when WS1–3 land, clearly framed as the
  design direction.
- Describe what *exists today* in precise terms without inflating it.
- Include a "Status" or "Roadmap" section in README.md that maps each
  capability to its realization state.
- Never present roadmap items as shipped.

The rule: any capability described in a present-tense claim ("bole enforces
X") must be realized today. Future items use "bole is designed to / the WS1
roadmap adds / upcoming in WS1."

### 3.3 New positioning statement

The following one-paragraph statement replaces the current README.md opening
paragraph. It is also the single canonical framing that all other doc sections
should echo in abbreviated form.

---

**Positioning statement (README.md, first paragraph after the heading):**

> bole is a version control library for multi-actor, access-controlled
> workflows. Unlike Git — where access control lives outside the repository in a
> hosting platform or filesystem permissions — bole encodes actor identities,
> visibility labels, and operation policies as first-class objects in the same
> content-addressed store as your files and history. Today that means named
> actors with glob-scoped path and timeline grants, ACL-filtered snapshot views,
> policy-controlled timeline advancement, and encrypted secrets stored alongside
> source files with access gated by the same rules. On the roadmap (WS1–3): a
> real label lattice, approval-gated merge hooks, and envelope-encrypted secrets
> with KMS integration — making bole the foundation for agent-safe workflows
> where every actor's capability is declared, enforced, and auditable without a
> separate service.

---

### 3.4 Document-by-document spec

#### 3.4.1 README.md

**H1 heading:** change from `# bole` to `# bole — access-controlled version
control for multi-actor workflows`

**Opening paragraph:** replace with the positioning statement from §3.3.

**"What bole is" section:** Reorder the five bullet items. Lead with the
access/identity items; place the storage-mechanism items after.

New order and framing:

1. **Actors and access** — named actors carry labeled grants (path globs,
   timeline patterns); the CLI binds an actor for all subsequent operations.
   Access-controlled views of snapshots filter what an actor can see. *(Today:
   two-point glob ACL. Roadmap WS1: real label lattice.)*
2. **Timelines with policy** — named movable pointers with configurable
   advancement policies (`ff`, `append`, `unrestricted`) enforced at the API
   boundary. *(Roadmap WS1: `PolicyHook` for approval-gated merge.)*
3. **Secrets and env overlays** — encrypted values and environment bundles
   stored as content-addressed objects, access-gated by the actor model, never
   committed as plaintext. *(Today: single-key ChaCha20-Poly1305. Roadmap WS3:
   envelope encryption with KMS.)*
4. **Snapshots** — the only durable state: immutable typed file trees plus
   metadata. Every operation produces a new snapshot; nothing is rewritten.
5. **Tags** — fixed named pointers to a snapshot.

**Features table:** Rename the "Gate" column to "Capability" and rewrite
descriptions to describe user value, not mechanism.

| Capability | Description |
|-----------|-------------|
| Content-addressed store | Immutable snapshots; identical content is stored once; BLAKE3-verified integrity |
| Timelines and tags | Named history views with configurable advancement policy |
| Granular ACLs | Path and timeline access rules; ACL-filtered snapshot views for each actor |
| Secrets and env overlays | Encrypted typed objects in the same store; env bundles mixing plain and secret values |
| Pluggable storage | In-memory (agents, tests) and disk-backed (CLI) backends behind one interface |
| Multi-actor workflows | Named actors, capability grants, agent-safe timelines |
| Git projection | One-way export of an ACL-filtered view to a bare Git repo |
| Performance | Criterion benchmarks; zstd-compressed disk objects; pack format (roadmap WS4) |

**Quick start section:** Add a multi-actor example before the solo example. The
solo example moves to example 2. The new example 1 shows the differentiator:
actor creation, ACL protection, and `merge check` with access enforcement. Use
the worked example from §3.5 (abbreviated).

**Architecture section:** Add a paragraph below the component tree:

> The access model sits across all three stores: `AclStore` holds path and
> timeline rules; `Accessor` evaluates them against an actor's grants at runtime.
> On the roadmap (WS1), rules become content-addressed `LabelLattice` objects and
> `Accessor` evaluates the full partial-order dominance relation; a `PolicyHook`
> trait gates `advance` and `merge` operations that labels alone cannot express.

**New "Status and roadmap" section** (before License):

```markdown
## Status and roadmap

| Capability | Today | Roadmap workstream |
|-----------|-------|-------------------|
| Content-addressed object store | Realized | — |
| Timelines, tags, ff/append/unrestricted policy | Realized | — |
| Glob ACLs (path + timeline), actor grants | Realized | — |
| Linked worktrees | Realized | WS7 (hardening) |
| Secrets (single-key ChaCha20-Poly1305) | Realized | — |
| Env overlays | Realized | — |
| Git projection | Realized | — |
| Real label lattice + clearance model | Design spec | WS1 (`bole-fo2`) |
| PolicyHook (approval-gated merge/advance) | Design spec | WS1 (`bole-fo2`) |
| Workspace trait unification | Design spec | WS2 (`bole-1kz`) |
| Envelope encryption + KMS integration | Design spec | WS3 (`bole-9mz`) |
| Pack format + GC | Design spec | WS4 (`bole-81z`) |
| Distributed sync | Design spec | WS5 (`bole-cy6`) |
| Git import | Design spec | WS6 (`bole-mtq`) |
```

#### 3.4.2 docs/API.md — intro section

**"Core concepts" section:** Add a framing paragraph before the five-primitive
table:

> bole's object model is designed to express what Git's commit DAG cannot:
> **who is allowed to see each file and timeline**, and **under what conditions
> an operation is permitted**. The five primitives below are the mechanism; the
> access model — actors, labels, and policy — is the reason for the design.

The five-primitive table itself is unchanged (it is already accurate).

Add a new "Access model" subsection immediately after the table, before the
ObjectStore section:

```
### Access model

Every read and write through the Repository API is mediated by an `Accessor`.
An `Accessor` holds an actor's path and timeline grants and answers
`can_read_path` / `can_write_path` / `can_read_timeline` / `can_write_timeline`
against a resource name.

Today's grant model: `PathRole { glob, permission }` and `TimelineRole {
pattern, permission }` — a two-point protection lattice expressed as glob
rules. On the roadmap (WS1): a real `LabelLattice`, `Clearance` objects, and
a `PolicyHook` trait that gates `advance` and `merge` for rules labels cannot
express (e.g., "two approvals required before merging into `release/**`").

`Accessor::privileged()` bypasses all checks and is appropriate only for
tests, migrations, and trusted administrative operations.
```

#### 3.4.3 docs/CLI.md — mental model section

**Current mental model line:**
> Actors open workspaces on timelines, produce snapshots, and advance timelines subject to ACL and policy.

This line is good. Expand it into a short section that makes the "why" explicit.

**Replacement mental model section:**

```markdown
## Mental model

> Actors open workspaces on timelines, produce snapshots, and advance timelines
> subject to ACL and policy.

bole's model puts access control *inside* the repository, not outside it:

- **An actor** is a named set of capability grants (path globs, timeline
  patterns). When a CLI session binds an actor, every subsequent operation is
  evaluated against that actor's grants. An automated agent and a human developer
  are the same concept — just different grant sets.
- **ACL rules** (path and timeline) record what each actor may see or modify.
  `Accessor` evaluates them at the API boundary; no external enforcement is
  needed.
- **Timeline policy** controls how history may advance: `ff` and `append`
  reject non-descendant snapshots; `unrestricted` accepts any (for merge targets,
  agent workspaces, etc.).
- **Secrets** are encrypted objects in the same content-addressed store as
  source files, visible only to actors cleared to read them.

What Git cannot express natively: an actor that can only see `src/**` and
produce snapshots on `agent/**`, while `main` is protected and requires an
explicit merge by an actor with broader grants. bole expresses this in the
repository itself, without a hosting platform.
```

**Add "Linked worktrees" mental model paragraph** (under the Workspace section,
before the `add/list/remove` command reference):

```markdown
### Linked worktrees

`workspace add <path> --timeline <name>` creates a linked worktree: a directory
that shares the primary store but tracks its own timeline independently. The
pointer file at `<path>/.bole` is the link. If you delete or move a linked
directory outside of bole, the registry may go stale; run `bole workspace prune`
to clean up orphaned entries and `bole workspace repair` to reconcile moved
directories or a moved store.
```

#### 3.4.4 bole-cli/README.md

**H1 heading:** change from `# bole-cli` to `# bole-cli — the bole CLI`

**Opening paragraph:** replace the current first paragraph with:

> The command-line interface to [bole](../README.md), a version control library
> for multi-actor, access-controlled workflows. `bole-cli` is a thin wrapper:
> every command maps directly onto the library's `Repository`, `ObjectStore`,
> `RefStore`, and `AclStore` APIs. Access control — which actors can see which
> files and timelines — is enforced by the library at the API boundary, not by
> the CLI. The crate produces a single binary named **`bole`**.

**Mental model table:** keep the table as-is, but add a new row:

| **Worktree** | a directory with a `.bole/` store (primary) or a `.bole` pointer file (linked); many directories can share one store, each on a different timeline |

**Add worked-example reference:** Add a note at the end of the mental model
section: "See §3.5 of the WS7 design spec for the canonical multi-actor +
approval-gate example that Git cannot express."

(In the actual README, this note is replaced with inline example content from
§3.5.)

#### 3.4.5 skills/claude-code/SKILL.md

**`description` frontmatter:** Change from:

> Use when version-controlling a project with the bole CLI — initializing a
> repo, creating snapshots from the work tree, managing timelines and tags,
> opening/diffing a workspace, merging, ACLs/actors, secrets, env overlays, or
> exporting to Git. Triggers on the `bole` command and `.bole/` repositories.

To:

> Use when version-controlling a project with the bole CLI. bole is
> access-controlled, multi-actor version control: actor grants, ACL rules, and
> policy-gated operations are in the repository model, not a hosting platform.
> Triggers on the `bole` command, `.bole/` repositories, or any task involving
> actors, timelines, secrets, env overlays, or linked worktrees.

**Opening paragraph:** Add after "bole is a content-addressed version-control
tool. It is NOT Git":

> More precisely: bole puts access control inside the repository. Named actors
> carry path and timeline grants; the CLI binds an actor before any
> access-controlled operation; secrets are encrypted objects in the same store as
> source files. Automated agents and human developers are the same concept — just
> different grant sets.

**Critical facts section:** add:

> - **`--as <actor>` binds capability for all workspace operations.** An agent
>   scoped to `src/**` write cannot see or touch `secrets/**`, enforced at the
>   API level.
> - **`workspace prune` / `workspace repair`** clean up stale or moved linked
>   worktree registrations; run them after moving directories or the store.

#### 3.4.6 skills/codex/AGENTS.md and skills/hermes/SKILL.md

Both files have identical content and `description`. Apply the same changes as
§3.4.5 (description frontmatter + opening paragraph addendum + critical facts).
The two files are kept in sync; they serve different agent runtimes (Codex and
Hermes) but must present the same model.

### 3.5 Differentiator worked example

This is the canonical example to feature prominently in README.md (Quick Start,
example 1) and summarize in the CLI reference. It illustrates what Git cannot
express without a hosting platform.

**Scenario:** A code-formatting agent (`formatter`) is allowed to modify source
files on a dedicated timeline, but cannot touch secrets, cannot read `internal/**`
docs, and cannot advance `main` directly. A human `admin` reviews and merges.

**Step-by-step (bole CLI, today's model):**

```bash
bole init .

# Protect sensitive paths and the main timeline.
bole acl path protect "secrets/**"
bole acl path protect "internal/**"
bole acl timeline protect "main"

# Define the formatter agent: write access to src only, write access to agent timelines.
bole actor create formatter
bole actor grant-path      formatter "src/**"    write
bole actor grant-timeline  formatter "agent/**"  write

# Define the admin: full access.
bole actor create admin
bole actor grant-path      admin "**" write
bole actor grant-timeline  admin "**" write

# Create and populate main.
SNAP=$(bole snapshot create --from-workspace -m "initial" --json | jq -r .snapshot)
bole workspace open main --create --from "$SNAP" --as admin

# The agent opens a dedicated timeline and makes changes.
bole timeline create agent/fmt --from @main
bole workspace add ./fmt-workspace --timeline agent/fmt --as formatter
# (In ./fmt-workspace, the formatter sees only src/**; secrets/** is invisible.)
cd ./fmt-workspace
bole snapshot create --from-workspace -m "format: apply rustfmt" --as formatter
cd ..

# Formatter cannot touch main:
bole acl can-write-timeline --actor formatter main    # → denied

# Admin reviews and merges.
bole actor use admin
bole merge check agent/fmt main                        # → allowed (no protected paths modified)
bole merge run   agent/fmt main -m "merge formatter output"

# Audit: the snapshot DAG records who did what and from which actor.
bole snapshot list --timeline main --json | jq '.[].author'
```

**What Git cannot express here without external infrastructure:**
- The `formatter` actor's path restrictions are enforced at the API boundary;
  the agent cannot even read `secrets/**` from its workspace view.
- The `main` timeline protection is stored in the repository; no hosting platform
  is required to block a direct push.
- The merge record includes actor identity as part of the repository history.
- In the WS1 roadmap: the `merge check` step can invoke a `PolicyHook` that
  requires `admin`'s explicit approval before `merge run` is permitted.

**Roadmap extension (WS1 — not yet implemented):**

```bash
# WS1 will add: a PolicyHook that requires two approvals before merge into main.
# Until WS1 lands, bole's timeline policy (ff/append/unrestricted) controls
# advancement; explicit approval gating is a roadmap item.
bole timeline set-policy main --hook "require-approval:admin,security-lead"
bole merge check agent/fmt main   # → requires-approval (lists needed approvers)
```

Document this in the worked example as clearly labeled roadmap.

### 3.6 Roadmap / status section

The `## Status and roadmap` table specified in §3.4.1 is the canonical record.
Each new workstream spec that lands a capability updates the corresponding row's
"Today" column from "Design spec" to "Realized" and removes the roadmap
workstream reference. The table is owned by README.md; other docs link to it
rather than duplicating it.

Protocol for keeping docs honest as workstreams land:
- On WS1 merge: update `Accessor` paragraph in API.md from "today's two-point
  lattice" to "real label lattice"; update CLI.md mental model to remove roadmap
  caveat on PolicyHook; update the positioning statement to remove "(roadmap WS1)"
  qualifiers.
- On WS3 merge: update the secrets description from "single-key" to "envelope
  encryption with KMS integration"; remove "(Today: single-key)" caveat from all
  docs.
- On WS5 merge: add "Distributed sync" row as realized.

Each workstream spec names which doc sections it is responsible for updating on
completion.

---

## 4. Backward compatibility

**Part A:** No existing command behavior changes. `prune` and `repair` are new
subcommands. `list --check` is a new flag that adds exit-code semantics to an
existing command; without the flag the exit code behavior is unchanged. The `status`
field in `list --json` is additive; consumers that ignore unknown JSON keys are
unaffected.

**Part B:** Doc-only changes. No CLI behavior, no library API, no on-disk format
changes. The three skill files are docs consumed by agent runtimes; updating
them improves guidance without breaking existing invocations.

---

## 5. Testing strategy

### 5.1 Part A — integration tests

Tests live under `bole-cli/tests/workspace_hardening.rs` (new file). Each test
uses a `tempdir` fixture that creates a primary store and one or more linked
worktrees, then manipulates the filesystem to simulate the failure modes, and
asserts the expected command output and exit codes.

**Required test cases:**

| Test | Setup | Command | Assert |
|------|-------|---------|--------|
| `prune_missing_directory` | Add linked worktree; `rm -rf` its directory | `workspace prune` | Entry removed from registry; metadata dir removed; output names the pruned id |
| `prune_missing_pointer` | Add linked worktree; delete only `.bole` pointer file | `workspace prune` | Entry removed; metadata removed |
| `prune_bad_pointer_json` | Add linked worktree; overwrite `.bole` with garbage | `workspace prune` | Entry removed; metadata removed; pointer file removed |
| `prune_dry_run` | Add linked worktree; `rm -rf` its directory | `workspace prune --dry-run` | Nothing modified; output shows what would be pruned |
| `prune_clean_repo` | Add linked worktree; do not modify anything | `workspace prune` | No entries pruned; exit 0 |
| `prune_leaves_user_files` | Add linked worktree with extra files; overwrite `.bole` with garbage | `workspace prune` | Extra files untouched; only `.bole` and metadata removed |
| `list_annotates_stale` | Add linked worktree; `rm -rf` its directory | `workspace list` | Output contains `[STALE: missing-directory]`; exit 0 |
| `list_check_exits_1_on_stale` | Add linked worktree; `rm -rf` its directory | `workspace list --check` | Exit 1 |
| `list_check_exits_0_on_clean` | Add linked worktree; leave intact | `workspace list --check` | Exit 0 |
| `list_json_status_field` | Add linked worktree; `rm -rf` its directory | `workspace list --json` | JSON contains `"status": "missing-directory"` |
| `repair_store_moved` | Add two linked worktrees; `mv .bole/ store2/`; update `repo_dir` | `workspace repair` | Both pointer files rewritten with new store path; registry unchanged |
| `repair_store_moved_dry_run` | Same setup | `workspace repair --dry-run` | No files modified; output describes what would change |
| `repair_directory_moved` | Add linked worktree; `mv worktree/ newloc/` | `workspace repair --moved-to newloc/ <id>` | Registry entry path updated; metadata intact |
| `repair_directory_moved_wrong_id` | Same setup but pointer's id was manually changed | `workspace repair --moved-to newloc/ <id>` | Error: pointer id mismatch; registry unchanged |
| `repair_adopt_orphaned` | Add linked worktree; manually remove its registry entry | `workspace repair --adopt <path>` | Entry re-added; metadata dir created if missing |
| `repair_adopt_already_registered` | Add linked worktree; no modification | `workspace repair --adopt <path>` | "Already consistent" message; exit 0 |
| `repair_adopt_wrong_store` | Create directory with `.bole` pointing to a different store | `workspace repair --adopt <path>` | Error: store mismatch; nothing modified |
| `prune_then_repair` | Add linked worktree; `mv` directory; observe stale entry | `workspace prune` then `workspace repair --adopt <new-path>` | Prune removes stale entry; adopt re-registers at new path |

### 5.2 Part B — verification

Part B is docs-only. Verification consists of:

1. **Link check:** all cross-references within updated docs resolve to existing
   files (CI: `markdown-link-check` or equivalent).
2. **Roadmap table audit:** CI script (`scripts/check-roadmap-honesty.sh`) that
   reads the `## Status and roadmap` table and fails if any row marked "Realized"
   references a bead that has no corresponding `bole-cli/tests/` or `src/`
   evidence (grep-based heuristic; not a hard requirement, but a nudge).
3. **Worked example is runnable:** The shell script in §3.5 is extracted into
   `tests/example_multi_actor.sh` and run in CI against a real `bole` binary.
   This ensures the positioning story stays honest as the codebase evolves.

---

## 6. Open questions

**OQ1 — `prune` exit code when nothing was pruned.** The spec says exit 0 "if
zero or more entries pruned" but notes exit 1 may indicate "nothing to prune."
Some callers (scripts that run `prune && notify`) would prefer exit 0 on "nothing
done" and exit 1 on "entries pruned." Others want exit 0 always for idempotent
use. **Decision needed:** adopt `0 = completed (including 0 entries)`, `1 = I/O
error` (simplest, analogous to `git worktree prune`), or `0 = clean, 1 = pruned
entries` (useful for scripting)?

**OQ2 — `repair` without args: should it auto-fix R1 (store moved) silently or
prompt?** The spec says auto-fix all R1 entries. This is convenient but
surprising if the user runs `repair` not knowing the store was moved. **Option:**
require `--yes` to confirm automatic pointer rewrites, or always require an
explicit flag (`--store-moved`) to prevent unintended silent mutation.

**OQ3 — `list --check` exit code semantics in CI.** Should "some stale entries"
return exit 1 (non-zero means "problem") or a separate exit 2 to distinguish from
other errors? **Recommendation:** 1 for "stale entries detected" (matches
`git status --short` convention where non-empty output exits 0 but `git diff
--exit-code` exits 1 on differences). Needs a decision before implementation to
avoid breaking scripts.

**OQ4 — `repair --adopt` when metadata is missing: what `state.json` to write?**
If a linked worktree was pruned and re-adopted, it has no `state.json`. The spec
says write `CliState::default()` (no timeline, no actor). This means the
worktree is unbound after adoption. **Alternative:** scan the pointer file for
any recoverable state, or prompt the user to re-bind. **Recommendation:** write
default (unbound) state; user runs `workspace open` to re-bind.

**OQ5 — Scanning for orphaned metadata dirs (`--scan-orphans`).** The spec
mentions this as a sub-case of `repair` but does not fully specify it. Should
`workspace repair` automatically scan `<store>/worktrees/` for directories not
referenced in the registry? Or is this deferred? **Recommendation:** defer to a
follow-up bead; orphaned metadata is not harmful (just wastes a few bytes), and
auto-scanning reduces the blast radius from accidental invocations.

**OQ6 — How does the roadmap table in README.md stay in sync?** The spec says
each workstream updates it on merge. Is this enforced by a CI check (verify that
bead-closed workstreams are marked "Realized") or purely by convention?
**Recommendation:** convention first; add CI gate when two or more workstreams
have landed and diverged.

**OQ7 — Skill file parity enforcement.** `skills/claude-code/SKILL.md`,
`skills/codex/AGENTS.md`, and `skills/hermes/SKILL.md` must be kept in sync. Is
there a CI check (e.g., assert their `description` frontmatter is identical) or
is this manual? **Recommendation:** add a trivial CI step that diffs the three
files and errors on divergence in the `description` and `Critical facts` sections.

**OQ8 — Worked example in tests: how to handle WS1 roadmap callouts?** The
integration test in `tests/example_multi_actor.sh` must skip the `PolicyHook`
section (labeled as roadmap). Should roadmap commands be commented out in the
test script, or should the script detect that WS1 is unimplemented and skip via
a `bole feature-flags` query? **Recommendation:** comment out with a clearly
labeled `# ROADMAP WS1` marker; a grep CI check ensures these markers are
removed when WS1 is merged.
