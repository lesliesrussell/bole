# WS6 — Git Import / Round-Trip

- **Bead:** `bole-mtq`
- **Depends on:** WS1 `bole-fo2` (label assignment for imported paths)
- **Status:** design spec (not an implementation plan)
- **Conforms to:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md).
  All shared vocabulary (Label, LabelLattice, label rule, Clearance, Accessor, PolicyHook,
  content-addressed policy object, atomic refs) is defined there and is not re-derived here.
  WS1 vocabulary (two-point lattice `public ⊑ protected`, label rules, `PathAcl` glob) is
  used directly below.

---

## 1. Goal

Add `git_import` (library) and `bole git import` (CLI) as the symmetric inverse of the
existing `project_to_git` / `bole git export`. Together the two functions form a
**round-trip bridge**: a git repository can be brought into bole, and a bole repository
can be projected back to git, without duplicating objects or losing structural identity.

The primary use cases are:

1. **Onboarding** — import an existing git codebase into bole to start using bole's
   policy model, secrets handling, and workspace features.
2. **Interop** — keep a shadow git remote for tooling (CI, code review, editors) that
   cannot speak the bole protocol. Export to git for those consumers, import back if
   commits land there.
3. **Incremental sync** — re-import after upstream pushes without reimporting the full
   history.

Non-goals for this workstream: the distribution/sync protocol (WS5), the workspace
Workspace trait unification (WS2), and CLI ergonomics beyond a minimal surface (WS7).

---

## 2. Architecture

```
 git repo (local path or fetched URL)
        │
        ▼
 ┌──────────────────────────────────────────────────┐
 │  git_import  (src/repo/git_import.rs)            │
 │                                                  │
 │  Pass 1: open git repo via gix                   │
 │  Pass 2: load IdentityMap from sidecar           │
 │  Pass 3: collect branches & tags; topo-sort      │
 │  Pass 4: write blobs → bole Blobs                │
 │  Pass 5: write trees → bole Trees                │
 │  Pass 6: write commits → bole Snapshots          │
 │  Pass 7: write branch → bole Timelines           │
 │  Pass 8: write tags → bole Tags                  │
 │  Pass 9: apply label rules (WS1)                 │
 │  Pass 10: persist updated IdentityMap            │
 └──────────────────────────────────────────────────┘
        │
        ▼
 bole Repository  (ObjectStore + RefStore)
```

The existing `project_to_git` (`src/repo/git_projection.rs`) is the inverse: it reads
from bole and writes to git. WS6 adds the other direction. The two paths share the
`IdentityMap` sidecar (§4) so that each direction can detect and skip already-translated
objects.

---

## 3. Object Mapping

### 3.1 Mapping table (import direction: git → bole)

| git object / concept | bole object / concept | Notes |
|---|---|---|
| blob | `Blob { data }` | Content copied verbatim |
| tree entry (mode 100644 / 100755) | `TreeEntry { id: blob_id, kind: Blob }` | Executable bit **lost** |
| tree entry (mode 40000) | `TreeEntry { id: tree_id, kind: Tree }` | Full recursion |
| tree entry (mode 120000) | `TreeEntry { id: blob_id, kind: Blob }` | Symlink stored as blob of target path; **documented loss** |
| tree entry (mode 160000) | **skipped** | Submodules out of scope (§7) |
| commit → blob/tree closure | `Blob` + `Tree` hierarchy | Identical content → same `ObjectId` via BLAKE3 |
| commit | `Snapshot { root, parents, author, created_at, message }` | See §3.2 |
| branch | `Timeline { head, policy, created_at, kind, expires_at }` | See §3.3 |
| lightweight tag | `Tag { target, created_at, message: None }` | `created_at` = tagged commit's `author_time` |
| annotated tag | `Tag { target, created_at, message: Some(tag_message) }` | See §3.4 |

### 3.2 git commit → bole Snapshot

A git commit carries two identities and two timestamps: **author** (who wrote the patch,
when they wrote it) and **committer** (who applied the patch, when they applied it).

Bole `Snapshot` has one `author: String` and one `created_at: u64`. The mapping is:

| git field | bole field | v1 behaviour |
|---|---|---|
| `author.name + author.email` | `Snapshot.author` | Stored as `"Name <email>"` verbatim |
| `author_time` (seconds, UTC) | `Snapshot.created_at` | UTC Unix seconds; timezone offset **lost** |
| `committer.name + committer.email` | — | **Lost in v1** (see Open question 1) |
| `committer_time` | — | **Lost in v1** |
| `message` | `Snapshot.message` | Full message including trailers |
| `tree sha` | `Snapshot.root` | Translated via Pass 5 |
| `parent shas` | `Snapshot.parents` | All parents preserved; merge commits become multi-parent Snapshots |
| GPG/SSH signature (commit header) | — | **Stripped in v1** |

The existing export (`project_to_git`) collapses `author` and `committer` to the same
identity string. Import must therefore accept commits whose `author == committer` as
unambiguously roundtrip-clean.

### 3.3 git branch → bole Timeline

| git field | bole field | v1 behaviour |
|---|---|---|
| branch name | `RefName` | Sanitized (see §3.6); slashes preserved |
| branch head sha | `Timeline.head` | Translated `ObjectId` via identity map |
| — | `Timeline.policy` | `Unrestricted` by default; overridable via `--timeline-policy` flag |
| — | `Timeline.created_at` | Import timestamp (`now`) |
| — | `Timeline.kind` | `"persistent"` |
| — | `Timeline.expires_at` | `None` |

### 3.4 git tag → bole Tag

**Lightweight tag:** a ref pointing directly to a commit sha.

| git | bole | notes |
|---|---|---|
| tag name | `RefName` | |
| pointed commit | `Tag.target` | Translated `ObjectId` |
| — | `Tag.created_at` | `created_at` of the referenced Snapshot |
| — | `Tag.message` | `None` |

**Annotated tag:** a git tag object with its own sha, tagger identity, timestamp, and
message.

| git | bole | notes |
|---|---|---|
| tag name | `RefName` | |
| `tag.target` commit | `Tag.target` | Translated `ObjectId` |
| `tag.tagger_time` (seconds) | `Tag.created_at` | UTC; timezone offset lost |
| `tag.tagger` identity | — | **Lost in v1** (see Open question 2) |
| `tag.message` | `Tag.message` | `Some(message)` |
| tag GPG signature | — | **Stripped in v1** |

### 3.5 Mapping in the export direction (bole → git)

This is the existing `project_to_git`. WS6 does **not** change the mapping rules; it
only adds identity-map persistence to the export path (§4.3). The existing mapping is:

| bole | git | existing behaviour |
|---|---|---|
| `Snapshot.author` | `author` identity | `"name <bole@local> timestamp +0000"` |
| — | `committer` identity | Same as author (collapsed) |
| `Snapshot.created_at` | `author_time = committer_time` | UTC seconds, `+0000` offset |
| `Snapshot.message` | commit message | Verbatim |
| `Snapshot.parents` | parent shas | All parents |
| `Tag.message == None` | lightweight tag ref | |
| `Tag.message == Some(_)` | NOT annotated — still written as lightweight | **Open question 3** |

The last row is a known asymmetry: the current exporter does not write git annotated tag
objects. WS6 does not fix this; it is listed as an open question for a follow-up spec.

### 3.6 Ref name sanitization

`RefName::new` rejects names whose segments start with `.`, contain null bytes, or have
consecutive slashes. Git allows a superset of these characters in branch names. The
sanitizer runs before `RefName::new`:

1. Replace any segment beginning with `.` → prefix with `_` (e.g. `.hidden` → `_hidden`).
2. Strip null bytes.
3. Collapse consecutive slashes.
4. Reject names that are empty after sanitization and log a warning; skip that ref.

The sanitizer is deterministic: the same git name always yields the same bole name. If
the sanitized name collides with an existing ref, the import aborts with a descriptive
error.

---

## 4. Identity Map

### 4.1 Purpose

The identity map is a persisted bidirectional table:

```
git ObjectId (20-byte SHA-1 or 32-byte SHA-256) ↔ bole ObjectId (32-byte BLAKE3)
```

It makes round-trips stable:

- **Export idempotency**: if a bole `ObjectId` is already in the map, `project_to_git`
  reuses the existing git OID instead of rewriting the object.
- **Import idempotency**: if a git OID is already in the map, `git_import` reuses the
  bole `ObjectId` instead of duplicating the object.
- **Incremental import**: on a second import after upstream pushes, only git commits
  absent from the map are translated. Timeline heads are advanced to the new tip.

### 4.2 Storage location

The identity map is stored as a **sidecar file** in the bole repository directory:

```
<bole-repo-root>/.bole/git-map/<fingerprint>.postcard
```

where `<fingerprint>` is the lowercase hex SHA-256 of the canonical source path or URL
(UTF-8 bytes). One sidecar per source: importing two different git repos into one bole
repo produces two separate sidecars.

**Format**: postcard-encoded `IdentityMap` struct:

```rust
struct IdentityMap {
    /// git SHA-1 (20 bytes) or SHA-256 (32 bytes) → bole BLAKE3 (32 bytes)
    git_to_bole: HashMap<Vec<u8>, [u8; 32]>,
    /// bole BLAKE3 (32 bytes) → git bytes
    bole_to_git: HashMap<[u8; 32], Vec<u8>>,
}
```

Postcard is consistent with the rest of bole's serialization layer, compact, and
already a dependency. The file is not part of the object store and is not transferred
in packs (WS4). It is local operational metadata, analogous to git's `ORIG_HEAD`.

### 4.3 Export must start persisting the map

**Decision: YES.** The current `project_to_git` builds an in-memory `id_map` that is
discarded at the end of the function. WS6 requires that it be persisted.

Rationale:
1. Without a persisted export map, an import following an export cannot detect that
   bole objects were the source — it would reimport everything, producing duplicate
   snapshots with different `ObjectId`s.
2. Incremental export (only writing new objects on re-export) requires the map to
   seed the initial state.
3. The cost is one postcard write (~microseconds) at export completion.

**API change**: `project_to_git` gains an optional `identity_map_dir: Option<&Path>`
parameter. When `None` (existing callers), behavior is unchanged and no sidecar is
written. When `Some(dir)`, the map is loaded from `dir/git-map/<fingerprint>.postcard`,
used to seed `id_map`, and written back after all passes complete. This is a
backward-compatible extension (adding an optional parameter with `None` default).

### 4.4 Interaction with atomic refs (WS4)

The sidecar is written after all bole objects and refs are committed. If the import
crashes after writing objects but before updating the sidecar, the next incremental
import will re-translate those objects (they are content-addressed, so re-translation
is idempotent — it produces the same `ObjectId`). The sidecar is then updated. This is
safe at the cost of redundant work; no corruption can occur.

---

## 5. Label Assignment for Imported Paths (WS1 tie-in)

All bole objects are content-addressed and carry no labels themselves. Labels are
assigned by label rules stored in the `LabelLattice` (WS1). On import, the importer
must decide what label rules to install for the imported paths.

**Default behaviour**: all imported paths receive label `public` — the bottom of the
two-point lattice `public ⊑ protected`. This matches the current pre-WS1 behaviour
where paths have no protection by default, and preserves backward compatibility.

**Optional ruleset application**: the `--label-ruleset <file>` flag accepts a
label-rule file in the format WS1 defines for `PathAcl` glob rules. The importer
applies those rules to the `PathAcl` store after all objects and refs are written.
This gives operators the ability to mark paths `protected` (or future lattice levels)
at import time rather than requiring a separate `bole acl set` pass.

The importer does not infer labels from git's file permissions or `.gitattributes`;
that would be WS7 ergonomic polish.

---

## 6. Incremental Import

On a second `bole git import` from the same source:

1. Load the existing `IdentityMap` from the sidecar (§4.2).
2. Walk the git ref graph. For each git OID encountered, check `git_to_bole`:
   - **Present**: skip object translation; use the existing bole `ObjectId`.
   - **Absent**: translate normally.
3. After processing all commits, for each imported branch: if the bole timeline
   already exists, advance its head to the new tip (using `advance_head`). The
   timeline's `policy` is not changed by an incremental import.
4. For new branches with no existing timeline, create a new timeline (as on first import).
5. For tags: if the bole tag already exists and points to the same target, skip. If the
   target changed (force-pushed tag), log a warning and do not update (tags are
   immutable in bole without an explicit `move_tag`).
6. Write the updated sidecar.

The importer does not track deleted git branches. If a git branch was deleted since the
last import, the corresponding bole timeline remains. A future `--prune` flag (WS7) can
handle this.

---

## 7. Out of Scope for v1

The following are explicitly excluded. Each may be addressed in a follow-up bead.

| Feature | Reason for exclusion |
|---|---|
| **Git submodules** (mode 160000 gitlinks) | Require recursive repo resolution; distinct design |
| **Git LFS** | LFS pointer files are imported as plain blobs; a warning is emitted. Full LFS hydration requires an LFS server protocol |
| **git notes** (`refs/notes/**`) | No bole analogue; excluded without loss of history |
| **git reflogs** | Local operational metadata; not part of the commit graph |
| **git worktrees** | Workspace model is WS2's territory |
| **GPG/SSH signatures** on commits or tags | Stripped on import; no bole signing model yet |
| **File mode bits** (executable, symlink type beyond blob storage) | bole Tree has no mode concept |
| **Partial clones / sparse checkouts** | Full tree import only |
| **git bundles** | Local path or URL only in v1 |
| **Push-to-import** | One-way pull; no serve protocol |
| **Remote tracking branches** (`refs/remotes/**`) | Local branches only |
| **`FETCH_HEAD`, `MERGE_HEAD`, loose state refs** | Not part of the history graph |
| **Annotated tag export** (existing gap in `project_to_git`) | Out of WS6 scope; listed in Open questions |

---

## 8. Public API Surface

### 8.1 Library (`src/repo/git_import.rs`)

```rust
/// Import all branches and tags from `source` into `repo`.
///
/// `source` must be a path to a bare or non-bare git repository that gix can open.
/// Remote URLs are not accepted directly — the caller is responsible for fetching
/// via `gix::clone` before calling this function.
///
/// `identity_map_dir` is the directory under which the sidecar map is stored;
/// pass `repo.path().join(".bole")` for the conventional location.
///
/// `opts` controls label assignment, timeline policy, and branch filtering.
pub async fn git_import(
    repo: &Repository,
    source: &Path,
    identity_map_dir: &Path,
    opts: ImportOptions,
) -> Result<ImportSummary>;

pub struct ImportOptions {
    /// Branches to import. Empty = all branches.
    pub branches: Vec<String>,
    /// Policy applied to newly created timelines.
    pub timeline_policy: TimelinePolicy,
    /// Optional path to a label-rule file (WS1 PathAcl glob format).
    pub label_ruleset: Option<PathBuf>,
    /// If true, translate objects but do not write to the repo or update the sidecar.
    pub dry_run: bool,
}

pub struct ImportSummary {
    pub blobs_written: usize,
    pub trees_written: usize,
    pub snapshots_written: usize,
    pub timelines_created: usize,
    pub timelines_advanced: usize,
    pub tags_created: usize,
    pub skipped_via_identity_map: usize,
}
```

`git_import` is the single entry point. Internal passes (object translation,
topo-sort, label application) are private functions within `git_import.rs`, mirroring
the structure of `git_projection.rs`.

### 8.2 CLI (`bole-cli`, `bole git import`)

```
bole git import <path>
    [--branch <name>]...           # import only named branches (repeatable); default: all
    [--timeline-policy <policy>]   # ff | append | unrestricted (default: unrestricted)
    [--label-ruleset <file>]       # apply WS1 label rules on import
    [--dry-run]                    # print plan without writing
```

`<path>` is a local filesystem path to a git repo. Remote URL support (fetch + import)
is a WS7 ergonomic concern; v1 accepts paths only. The operator fetches (`git fetch` or
`gix fetch`) and then runs `bole git import`.

Output on success: one line per timeline/tag created or advanced, plus a summary row.

The existing `bole git export` surface (`project_to_git`) gains no new flags in WS6;
only the internal sidecar write is added.

---

## 9. Backward Compatibility and Migration

### 9.1 `project_to_git` API change

The change is additive: a new optional `identity_map_dir: Option<&Path>` parameter is
appended. All call sites that pass `None` (or use the default) retain current behaviour.
No existing test changes.

### 9.2 Existing 247 passing tests

No existing test is broken. The identity map sidecar is only written when
`identity_map_dir` is `Some`. Existing `project_to_git` tests pass `None` (or the
current no-arg form) and exercise no sidecar path.

### 9.3 Repos created before WS6

Importing into an existing bole repo with no sidecar starts a fresh identity map. The
first import is a full import; subsequent imports are incremental. No migration step is
needed.

---

## 10. Testing Strategy

### 10.1 Unit tests (`src/repo/git_import.rs`)

- `identity_map_roundtrip`: write and read a postcard sidecar; assert both directions
  are preserved.
- `sanitize_ref_name_dot_prefix`: assert `.hidden` → `_hidden` and `feature/.foo` →
  `feature/_foo`.
- `sanitize_ref_name_collision_errors`: two git branches that sanitize to the same
  name produce `Err`.
- `topo_sort_parents_before_children`: port of the existing export test, adapted to
  the import direction.
- `symlink_stored_as_blob`: git tree with mode 120000 entry → bole blob containing
  the link target bytes; no panic.
- `submodule_entry_skipped`: git tree with mode 160000 entry → silently absent from
  bole tree.

### 10.2 Round-trip integration test

The key invariant test lives in `tests/git_roundtrip.rs`:

```
1. Build an in-memory bole repo with known structure
   (3-commit linear history on `main`, one merge commit, one annotated tag).
2. project_to_git → bare git repo at tmp/export.git (with identity_map_dir set).
3. git_import from tmp/export.git into a fresh bole repo (using the same map dir).
4. Assert:
   a. Same number of Snapshots in the new repo.
   b. Same tree content at every Snapshot (walk and compare blob data).
   c. Same Timeline names and heads.
   d. Same Tag names; Tag.message preserved for annotated tag.
   e. skipped_via_identity_map == 0 (fresh import has no prior map).
5. Run project_to_git again on the imported repo.
6. Assert the two git repos are structurally identical:
   each branch ref points to a commit with the same parent structure and tree sha.
```

### 10.3 Incremental import test

```
1. Import a 2-commit git repo into bole.
2. Add a third commit to the git repo (in-process via gix).
3. Re-import.
4. Assert: snapshots_written == 1, skipped_via_identity_map == 2.
5. Assert: timeline head advanced to the new snapshot.
```

### 10.4 Label ruleset test

```
1. Import a git repo with paths ["src/main.rs", "private/secret.rs"].
2. Pass --label-ruleset file containing `private/** → protected`.
3. Assert: Accessor without `protected` clearance cannot read `private/secret.rs`.
4. Assert: Accessor with `protected` clearance can read both.
```

---

## 11. Open Questions

These are design forks requiring a maintainer decision before implementation begins.

**OQ-1 Committer metadata loss.**
Git's committer identity and timestamp are discarded in v1. For repos where `author !=
committer` (e.g. GitHub merges, cherry-picks), this loses information. Options:
(a) store committer as a message trailer on import (`Committer: name <email> ts tz`);
(b) add an optional `committer: Option<String>` field to `Snapshot` (a breaking schema
change); (c) accept the loss and document it. Recommendation: (a) for v1 (non-breaking,
recoverable), with (b) as a future WS3 or WS4 schema evolution.

**OQ-2 Annotated tag tagger identity loss.**
The git tagger (`name <email>` + timestamp) has no bole analogue. Same options as OQ-1.
A tag message trailer `Tagger: ...` is the minimal v1 answer.

**OQ-3 Export of annotated tags.**
`project_to_git` currently writes all bole Tags as lightweight git tag refs (a `Tag`
with `message: Some(...)` is NOT exported as a git annotated tag object). This is an
existing gap. Should WS6 fix it as part of completing the round-trip, or defer to a
follow-up bead? The fix requires `encode_commit`-style `encode_tag` logic in
`git_projection.rs`. Recommendation: fix in this workstream since the identity map
needs to track git tag OIDs, and annotated tags have their own OIDs.

**OQ-4 Timeline policy on re-import head advance.**
When a timeline already exists and its head is advanced during an incremental import,
should the importer enforce the timeline's current policy (e.g. `FastForwardOnly`)? If
the upstream git branch was force-pushed, the new tip may not be a descendant of the
current bole head. Options: (a) always advance with `Unrestricted` override on import
(the importer is privileged); (b) respect the policy and fail with an error, requiring
`--force`; (c) log a warning and skip. Recommendation: (b) with `--force` flag to
override, so the policy model is not silently bypassed.

**OQ-5 Identity map format.**
Postcard is compact and consistent with bole but is binary and requires tooling to
inspect. A JSON sidecar would be human-readable and `jq`-debuggable. Decision: postcard
for v1 (consistent with bole's serialization layer), with a `bole git map dump`
diagnostic subcommand (WS7) to render it as JSON.

**OQ-6 Canonical fingerprint for source URL.**
For local paths, the fingerprint is the SHA-256 of the absolute canonical path. For
URLs (WS7 concern), the same SHA-256 of the URL string after normalization. If the same
repo is cloned to two different local paths, the fingerprints differ and two independent
maps are maintained. Is this the desired behaviour, or should the map be keyed on the
git repo's `HEAD` commit or remote URL? Recommendation: path-based for v1 (simple,
deterministic); URL-based as a WS7 option.

**OQ-7 RefName collision on sanitization.**
If two git branches sanitize to the same bole `RefName`, the import aborts. An
alternative is to apply a numeric suffix (`_1`, `_2`). The abort-on-collision approach
is safer (no silent renaming) but may frustrate users with many `.`-prefixed branches.
Keep abort behaviour; document it.

**OQ-8 Empty commits.**
A git commit that changes no files (same tree SHA as its parent) still produces a
distinct bole Snapshot (different metadata). This is correct — bole Snapshots are
identity-by-content-hash, and the metadata differs. No special handling needed; confirm
this is intentional.
