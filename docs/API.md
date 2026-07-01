# bole API Reference

This document describes every public type, method, and function in the `bole` crate, along with usage examples. It is intended to be read top-to-bottom once, then used as a reference.

---

## Table of Contents

1. [Core concepts](#core-concepts)
2. [ObjectId](#objectid)
3. [Object types](#object-types)
   - [Blob](#blob)
   - [Tree and TreeEntry](#tree-and-treeentry)
   - [Snapshot](#snapshot)
   - [Secret](#secret)
   - [EnvOverlay](#envoverlay)
4. [ObjectStore](#objectstore)
5. [Storage backends](#storage-backends)
   - [MemoryBackend](#memorybackend)
   - [DiskBackend](#diskbackend)
6. [Repository](#repository)
7. [References](#references)
   - [RefName](#refname)
   - [Ref, Timeline, Tag](#ref-timeline-tag)
   - [TimelinePolicy](#timelinepolicy)
8. [Access control](#access-control)
   - [Permission](#permission)
   - [PathRole and TimelineRole](#pathrole-and-timerole)
   - [Accessor](#accessor)
9. [Repository methods](#repository-methods)
   - [advance\_timeline](#advance_timeline)
   - [get\_snapshot\_filtered](#get_snapshot_filtered)
   - [explain\_path](#explain_path)
   - [list\_refs\_filtered](#list_refs_filtered)
   - [check\_merge](#check_merge)
   - [merge\_timelines](#merge_timelines)
   - [compute\_workspace\_view](#compute_workspace_view)
10. [Utility functions](#utility-functions)
    - [copy\_objects / copy\_refs](#copy_objects--copy_refs)
    - [materialize](#materialize)
    - [project\_to\_git](#project_to_git)
11. [In-memory workspaces](#in-memory-workspaces)
12. [Extended API](#extended-api) — secrets, packs/GC, ref transactions, sync, policy authority, git import
13. [Error handling](#error-handling)
14. [Complete example](#complete-example)

---

## Core concepts

bole's object model is designed to express what Git's commit DAG cannot: **who is
allowed to see each file and timeline**, and **under what conditions an operation
is permitted**. The five primitives below are the mechanism; the access model —
actors, labels, and policy — is the reason for the design.

bole's object model has five primitives:

| Primitive | What it is |
|-----------|-----------|
| **Blob** | Raw bytes — a file, a generated artifact, any opaque content |
| **Tree** | A named set of `path → ObjectId` entries (like a directory) |
| **Snapshot** | An immutable project state: a root Tree + metadata + parent links |
| **Secret** | An encrypted blob — separate lifecycle and visibility from plain blobs |
| **EnvOverlay** | A named set of environment variables — typed config bundles |

All five are stored in the same content-addressed `ObjectStore`. An `ObjectId` is a 32-byte BLAKE3 hash of the object's serialized form. Identical content always produces the same `ObjectId`; nothing is ever rewritten in place.

**Timelines** (like Git branches) are mutable named pointers to a Snapshot. **Tags** are named pointers that can point at either a Snapshot or a Timeline head. Timelines and Tags live in a `RefStore`, separate from the object store.

**ACLs** control which paths and timelines an `Accessor` can read or write. An Accessor is a capability token — you build one for each actor in your system and pass it to every operation.

### Access model

Every read and write through the `Repository` API is mediated by an `Accessor`.
An `Accessor` binds a **label lattice**, a rule set, and an actor's scoped
**clearances**, and answers `can_read` / `can_write` for a resource's effective
label — plus the convenience `can_read_path` / `can_write_path` /
`can_read_timeline` / `can_write_timeline` / `can_read_secret` against a resource
name. Glob path/timeline ACLs are the degenerate two-point lattice
(`public ⊑ protected`); a real bounded lattice with multiple levels is expressed
by the same types. A `PolicyHook` registry gates `advance` and `merge` for rules
labels cannot express (e.g. "N approvals before merging into `release/**`").

`Accessor::privileged()` grants **read-only** access to everything and is
appropriate for tests, migrations, and trusted read paths; build an `Accessor`
with explicit `Clearance`s (or `from_parts`) for write operations.

---

## ObjectId

```rust
pub struct ObjectId([u8; 32]);
```

A content address. Displayed as 64 lowercase hex characters.

### Constructors

```rust
// Hash arbitrary bytes — this is the normal way to get an id from content.
// Used internally by put_blob etc.; you rarely call this directly.
ObjectId::from_content(data: &[u8]) -> ObjectId

// Wrap a known raw hash (e.g. when reading back from storage).
ObjectId::new(bytes: [u8; 32]) -> ObjectId

// Parse a 64-char hex string (e.g. from CLI input or a config file).
// Returns ParseObjectIdError if the string is not exactly 64 lowercase hex chars.
"a3f1...".parse::<ObjectId>() -> Result<ObjectId, ParseObjectIdError>
```

### Methods

```rust
id.as_bytes() -> &[u8; 32]   // raw bytes
id.to_string()               // 64 lowercase hex chars (via Display)
```

`ObjectId` implements `Hash`, `Eq`, `Ord`, `Copy`, `Serialize`, `Deserialize`.

---

## Object types

### Blob

```rust
pub struct Blob {
    pub data: Bytes,
}
```

Raw bytes. Use `ObjectStore::put_blob` / `get` to store and retrieve blobs. There is no metadata; the content is the identity.

### Tree and TreeEntry

```rust
pub struct Tree {
    pub entries: BTreeMap<String, TreeEntry>,
}

pub struct TreeEntry {
    pub id: ObjectId,    // points to a Blob or a nested Tree
    pub kind: EntryKind,
}

pub enum EntryKind {
    Blob,
    Tree,
}
```

A Tree maps logical path components to child objects. Path components are strings — they do not have to be filesystem names. Trees are typically nested: a root Tree may contain sub-Trees (directories) and Blobs (files). A snapshot's tree is a pure file hierarchy: it contains only blobs and subtrees. Secrets and env overlays are **not** tree entries — they are standalone objects referenced out of band (see [Secret](#secret) and [EnvOverlay](#envoverlay)).

When building a tree for a multi-level path like `src/main.rs`, you create the leaf Tree (`src/`) with one entry `"main.rs" → blob_id`, then create a root Tree with one entry `"src" → src_tree_id`.

`put_tree` accepts a flat `BTreeMap<String, TreeEntry>`. For nested structures, build the deepest level first and work upward.

### Snapshot

```rust
pub struct Snapshot {
    pub root: ObjectId,          // root Tree of this snapshot
    pub parents: Vec<ObjectId>,  // parent Snapshot ids (empty for root, 1 for linear, 2 for merge)
    pub author: String,
    pub created_at: u64,         // Unix timestamp (seconds)
    pub message: String,
}
```

The only durable state primitive. A Snapshot is immutable once stored. Changing anything — adding a file, merging — produces a new Snapshot with a new `ObjectId`.

`parents` forms the history DAG. An empty `parents` vec is a root commit. Two parents indicate a merge.

### Secret

```rust
pub struct Secret {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}
```

An encrypted value stored in the object graph. Secrets use ChaCha20-Poly1305 with a random 96-bit nonce per write. Because the nonce is random, two `put_secret` calls with identical plaintext produce **different** `ObjectId`s — there is no equality leakage through the content-addressed store.

You never construct `Secret` directly. Use `ObjectStore::put_secret` / `get_secret`.

### EnvOverlay

```rust
pub struct EnvOverlay {
    pub entries: BTreeMap<String, EnvValue>,
}

pub enum EnvValue {
    Plain(String),
    Secret(ObjectId),  // points to a Secret object in the same store
}
```

A named set of environment variables. `Plain` values are stored in plaintext in the overlay. `Secret` values are pointers to separately-encrypted `Secret` objects — the overlay itself does not contain the plaintext.

You never construct `EnvOverlay` directly. Use `ObjectStore::put_overlay` / `get_overlay`.

---

## ObjectStore

```rust
pub struct ObjectStore { /* ... */ }
```

The content-addressed storage layer. All five object types share one store.

### Constructors

```rust
// Memory-backed store — useful in tests and for agents that never touch disk.
ObjectStore::new(MemoryBackend::new())

// Disk-backed store — see DiskBackend::open below.
ObjectStore::new(backend)
```

`Repository::memory()` and `Repository::disk()` wrap this for you. Use `ObjectStore` directly only when you need a store without the full `Repository` overhead.

### Storing objects

```rust
// Returns the ObjectId of the stored blob.
// Second call with identical bytes returns the same id (deduplication).
store.put_blob(data: Bytes) -> Result<ObjectId>

// Stores a Tree. Build leaves first, then parents.
store.put_tree(entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId>

// Stores a Snapshot. Fill in root, parents, author, created_at, message.
store.put_snapshot(snap: Snapshot) -> Result<ObjectId>

// Encrypts plaintext with key (ChaCha20-Poly1305) and stores the ciphertext.
// key is a 32-byte symmetric key — derive it with something like HKDF or Argon2.
store.put_secret(plaintext: &[u8], key: &[u8; 32]) -> Result<ObjectId>

// Stores an EnvOverlay.
store.put_overlay(overlay: EnvOverlay) -> Result<ObjectId>
```

### Retrieving objects

```rust
// Returns the object behind any ObjectId, or None if not present.
// The returned Object enum lets you match on the type.
store.get(id: &ObjectId) -> Result<Option<Object>>

// Decrypts and returns the secret plaintext, or None if the id is not found.
// Returns Err(Error::DecryptionFailed) if the key is wrong or ciphertext corrupted.
store.get_secret(id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>>

// Returns the EnvOverlay for the given id.
store.get_overlay(id: &ObjectId) -> Result<Option<EnvOverlay>>

// Returns all ObjectIds currently in the store.
// Order is not guaranteed.
store.list() -> Result<Vec<ObjectId>>
```

### The Object enum

```rust
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
    Secret(Secret),
    EnvOverlay(EnvOverlay),
}
```

`store.get()` returns `Option<Object>`. Match on it to extract the inner type.

---

## Storage backends

### MemoryBackend

```rust
MemoryBackend::new() -> MemoryBackend
```

Stores objects in a `HashMap<ObjectId, Bytes>` in the current process. Fast, no I/O, no persistence. Use this in tests, CI, and for agents operating on ephemeral repos.

### DiskBackend

```rust
DiskBackend::open(root: impl AsRef<Path>) -> Result<DiskBackend>  // async
```

Stores objects as zstd-compressed files under `<root>/objects/<2-hex>/<62-hex>`. Creates the directory if it does not exist.

Storage layout:
```
<root>/objects/
├── a3/
│   └── f1c8...   (62-hex filename, zstd-compressed Object)
├── b7/
│   └── 02dd...
└── ...
```

Object writes are atomic (write to `.tmp`, then `rename`). Reads decompress on the fly. The 256 shards (`00`–`ff`) keep directory sizes manageable for large repos.

---

## Repository

```rust
pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
    pub acls: AclStore,
}
```

The top-level handle. Combines an object store, a reference store, and an ACL store into one unit.

### Constructors

```rust
// In-memory repo — MemoryBackend for all three stores.
Repository::memory() -> Repository

// Disk-backed repo — DiskBackend for objects and refs, DiskAclBackend for ACLs.
Repository::disk(root: impl AsRef<Path>) -> Result<Repository>  // async
```

### Direct field access

`repo.objects`, `repo.refs`, and `repo.acls` are public. Use them directly for low-level operations (e.g. `repo.objects.put_blob(...)`, `repo.refs.create_timeline(...)`).

---

## References

### RefName

```rust
pub struct RefName(/* ... */);
```

A validated reference name. Names follow the same rules as Git ref names (no `..`, no leading `/`, etc.).

```rust
RefName::new("main") -> Result<RefName>
RefName::new("leslie/experiment-1") -> Result<RefName>
RefName::new("releases/v2.0") -> Result<RefName>

name.as_str() -> &str   // the validated string
name.prefix() -> &str   // everything before the last `/`, or "" if no slash
```

### Ref, Timeline, Tag

```rust
pub enum Ref {
    Timeline(Timeline),
    Tag(Tag),
}

pub struct Timeline {
    pub head: ObjectId,           // current head Snapshot
    pub policy: TimelinePolicy,
    pub created_at: u64,          // Unix timestamp (seconds) when created
    pub kind: String,             // lifecycle category: "persistent" | "ephemeral" | custom
    pub expires_at: Option<u64>,  // optional Unix timestamp after which pruning is allowed
}

pub struct Tag {
    pub target: ObjectId,   // Snapshot this tag points at
    pub created_at: u64,
    pub message: String,
}
```

Access refs via `repo.refs`:

```rust
repo.refs.create_timeline(name, head, policy, now, kind, expires_at) -> Result<()>
repo.refs.get_timeline(name: &RefName) -> Result<Option<Timeline>>
repo.refs.get(name: &RefName) -> Result<Option<Ref>>
repo.refs.list(prefix: &str) -> Result<Vec<RefName>>   // "" = all refs
repo.refs.delete_ref(name: &RefName) -> Result<()>
repo.refs.create_tag(name, target, message: Option<String>, now: u64) -> Result<()>
repo.refs.get_tag(name: &RefName) -> Result<Option<Tag>>
repo.refs.move_tag(name: &RefName, target: ObjectId) -> Result<()>
repo.refs.advance_head(name: &RefName, new_head: ObjectId) -> Result<()>  // raw, policy-free
```

### TimelinePolicy

```rust
pub enum TimelinePolicy {
    FastForwardOnly,  // new head must be a descendant of the current head
    Append,           // new head must be a descendant of the current head
    Unrestricted,     // head may be set to any snapshot
}
```

The policy is **enforced** by `Repository::advance_timeline`: for `FastForwardOnly`
and `Append`, an advance is rejected with `Error::PolicyViolation` unless the
current head is an ancestor of the new head (i.e. a true fast-forward; a no-op
re-set and any descendant qualify). `Unrestricted` accepts any snapshot. The
low-level `repo.refs.advance_head` primitive does **not** enforce policy — it is
the unchecked setter; enforcement lives only in the `Repository` layer.

> Note: `Append` currently applies the same descendant rule as
> `FastForwardOnly`.

---

## Access control

ACLs are stored in `repo.acls` (`AclStore`). An `Accessor` is a capability token that you attach to each actor (user, agent) in your system. Pass it to every privileged operation.

### Permission

```rust
pub enum Permission {
    Read,
    Write,
}
```

### PathRole and TimelineRole

```rust
pub struct PathRole {
    pub glob: String,        // glob pattern, e.g. "src/**" or "secrets/**"
    pub permission: Permission,
}

pub struct TimelineRole {
    pub pattern: String,     // glob pattern, e.g. "main" or "leslie/**"
    pub permission: Permission,
}
```

A `PathRole` grants read or write access to paths matching `glob`. A `TimelineRole` grants read or write access to timelines matching `pattern`.

**Glob syntax** (used for both roles and ACLs):

- `*` matches any run of characters within a single path segment (does not cross `/`). E.g. `src/*.rs` matches `src/a.rs` but not `src/a/b.rs`; `*` may match zero characters.
- `**` matches zero or more whole path segments, including in the middle of a pattern: `secrets/**` matches `secrets/a/b`, `**/key` matches `key` and `a/b/key`, `a/**/z` matches `a/z` and `a/x/y/z`.
- A trailing `**` matches descendants only: `src/**` matches `src/main.rs` but **not** the bare `src`.
- Matching is **case-sensitive** and literal otherwise (`secret` does not match `secrets`).

### PathAcl and TimelineAcl

```rust
pub struct PathAcl {
    pub glob: String,        // the protected glob
}

pub struct TimelineAcl {
    pub pattern: String,     // the protected timeline pattern
}
```

ACLs record which globs are protected. They are stored in `repo.acls`:

```rust
repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() })        // mark glob as protected
repo.acls.path_is_protected(path: &str) -> Result<bool>
repo.acls.remove_path_acl(glob: &str) -> Result<()>
repo.acls.list_path_acls() -> Result<Vec<PathAcl>>

repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/**".into() })
repo.acls.timeline_is_protected(name: &str) -> Result<bool>
repo.acls.remove_timeline_acl(pattern: &str) -> Result<()>
repo.acls.list_timeline_acls() -> Result<Vec<TimelineAcl>>
```

### Accessor

```rust
pub struct Accessor { /* ... */ }
```

A capability token. Build one per actor using the builder methods.

```rust
// READ-ONLY on everything — for internal operations that must bypass per-user
// ACL checks (e.g. walking a full tree). It does NOT grant write; use explicit
// Write roles to advance timelines or merge.
Accessor::privileged() -> Accessor

// Empty — no permissions. Start here and add roles.
Accessor::new() -> Accessor

// Grant read or write access to paths matching a glob.
accessor.with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })

// Grant read or write access to timelines matching a pattern.
accessor.with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write })
```

#### Checking permissions

```rust
accessor.can_read_path(path: &str) -> bool
accessor.can_write_path(path: &str) -> bool
accessor.can_read_timeline(name: &str) -> bool
accessor.can_write_timeline(name: &str) -> bool
```

These are checked internally by Repository methods; you can also call them directly when building access control logic.

#### Example: agent with restricted access

```rust
// Agent can read and write src/**, read docs/**, cannot touch secrets/**
let agent = Accessor::new()
    .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })
    .with_path_role(PathRole { glob: "docs/**".into(), permission: Permission::Read })
    .with_timeline_role(TimelineRole { pattern: "agent/**".into(), permission: Permission::Write });
```

---

## Repository methods

### advance\_timeline

```rust
repo.advance_timeline(
    name: &RefName,
    new_head: ObjectId,
    accessor: &Accessor,
) -> Result<()>
```

Moves a timeline's head to `new_head`. The `new_head` must be a `Snapshot` object. The accessor must have **write** permission on the timeline and **write** permission on every path in the snapshot's tree.

It enforces the timeline's [`TimelinePolicy`](#timelinepolicy): under `FastForwardOnly` or `Append`, the new head must be a descendant of the current head.

Returns `Err(Error::AccessDenied)` if the accessor lacks the required permissions, or `Err(Error::PolicyViolation)` if the move violates the policy.

> `Accessor::privileged()` grants read-only access, so it cannot advance a
> timeline. Build a write-capable accessor (see [Accessor](#accessor)).

```rust
// A write-capable accessor (privileged() is read-only and would be denied).
let writer = Accessor::new()
    .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
    .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write });

let new_snap = repo.objects.put_snapshot(Snapshot {
    root: new_tree, parents: vec![old_snap], author: "alice".into(),
    created_at: 1_700_000_001, message: "update".into(),
}).await?;
repo.advance_timeline(&RefName::new("main")?, new_snap, &writer).await?;
```

### get\_snapshot\_filtered

```rust
repo.get_snapshot_filtered(
    id: ObjectId,
    accessor: &Accessor,
) -> Result<Option<FilteredSnapshot>>
```

Returns a `FilteredSnapshot` containing only the paths the accessor can read. Paths outside the accessor's `PathRole`s are silently excluded — the caller never learns they exist.

```rust
pub struct FilteredSnapshot {
    pub id: ObjectId,
    pub author: String,
    pub created_at: u64,
    pub message: String,
    pub parents: Vec<ObjectId>,
    pub visible_paths: BTreeMap<String, ObjectId>,  // path → blob id (filtered)
}
```

```rust
let reader = Accessor::new()
    .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Read });

if let Some(view) = repo.get_snapshot_filtered(snap_id, &reader).await? {
    for (path, blob_id) in &view.visible_paths {
        println!("{path}: {blob_id}");
    }
}
```

### explain\_path

Where `get_snapshot_filtered` tells you *what* an actor sees, `explain_path`
tells you *why*. It returns an `AccessExplanation`: whether the path is present
in the snapshot, its effective label and the rules that set it, and a `Decision`
for both read and write — each with a verdict, a human-readable `reason`, and the
per-clearance `ClearanceEval` trace (scope match, capability, dominance) with the
deciding clearance flagged. The read verdict applies the same public/bottom
short-circuit as the filtered tree walk; the write verdict mirrors
`advance_timeline`'s per-path check.

```rust
let exp = repo.explain_path(&accessor, snap_id, "secrets/prod.key").await?;
println!("{}", exp.read.reason);   // e.g. "denied: no in-scope read clearance dominates label `protected`"
for c in &exp.read.clearances {
    if c.decisive { println!("granted by clearance with ceiling {}", c.ceiling.0); }
}
```

### list\_refs\_filtered

```rust
repo.list_refs_filtered(
    prefix: &str,
    accessor: &Accessor,
) -> Result<Vec<RefName>>
```

Lists refs whose names the accessor can read. `prefix` can be `""` for all refs or `"feature/"` to scope to a namespace.

### check\_merge

```rust
repo.check_merge(
    source: &RefName,
    dest: &RefName,
    accessor: &Accessor,
) -> Result<MergeCheck>
```

Checks whether merging `source` into `dest` would leak protected paths into `dest`. Does not perform the merge.

```rust
pub enum MergeCheck {
    Allowed,                        // no protected paths would leak
    RequiresApproval(Vec<PathAcl>), // accessor has write on dest but protected paths would leak
    Rejected(Vec<PathAcl>),         // accessor lacks write on dest; blocked
}
```

Returns `Err` if the source ref does not exist or its head is not a Snapshot.

```rust
match repo.check_merge(&source, &dest, &accessor).await? {
    MergeCheck::Allowed => { /* proceed */ }
    MergeCheck::RequiresApproval(leaking) => {
        println!("requires approval for: {:?}", leaking);
    }
    MergeCheck::Rejected(blocked) => {
        println!("rejected, would expose: {:?}", blocked);
    }
}
```

### merge\_timelines

```rust
repo.merge_timelines(
    source: &RefName,
    target: &RefName,
    accessor: &Accessor,
) -> Result<MergeResult>
```

Performs a three-way merge of `source` into `target`. Finds the lowest common ancestor, diffs both sides, and produces a merged snapshot or a conflict set.

```rust
pub struct MergeResult {
    pub merged: BTreeMap<String, ObjectId>,  // conflict-free merged paths
    pub conflicts: BTreeMap<String, (ObjectId, ObjectId)>,  // (ours, theirs) for conflicts
}
```

On success, the merged snapshot is **not** automatically stored or advanced. You store it and advance the timeline yourself:

```rust
let result = repo.merge_timelines(&feature, &main, &accessor).await?;
if result.conflicts.is_empty() {
    let merged_tree = repo.objects.put_tree(result.merged).await?;
    let merged_snap = repo.objects.put_snapshot(Snapshot {
        root: merged_tree,
        parents: vec![source_head, dest_head],
        author: "merger".into(),
        created_at: now,
        message: "merge feature into main".into(),
    }).await?;
    repo.advance_timeline(&main, merged_snap, &accessor).await?;
}
```

### compute\_workspace\_view

```rust
repo.compute_workspace_view(
    snap_id: ObjectId,
    accessor: &Accessor,
) -> Result<WorkspaceView>
```

Computes the effective workspace for a snapshot: the filtered file tree plus any env overlays visible to the accessor. Used to mount a snapshot in an agent's working context.

```rust
pub struct WorkspaceView {
    pub files: BTreeMap<String, ObjectId>,
    pub env: BTreeMap<String, EnvValue>,
}
```

---

## Utility functions

### copy\_objects / copy\_refs

```rust
copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()>  // async
copy_refs(from: &RefStore, to: &RefStore) -> Result<()>
```

Copies all objects (or refs) from one store to another. Used to clone repos or migrate between backends.

```rust
let mem_repo = Repository::memory();
let disk_repo = Repository::disk("/path/to/persist").await?;
copy_objects(&mem_repo.objects, &disk_repo.objects).await?;
copy_refs(&mem_repo.refs, &disk_repo.refs)?;
```

### materialize

```rust
materialize(
    repo: &Repository,
    snap_id: ObjectId,
    dest: &Path,
    accessor: &Accessor,
) -> Result<()>  // async
```

Writes the filtered snapshot tree to a directory on disk. Only paths the accessor can read are written. Existing files under `dest` are not removed — only the snapshot's paths are written or overwritten.

```rust
materialize(&repo, snap_id, Path::new("/tmp/workspace"), &Accessor::privileged()).await?;
```

### project\_to\_git

```rust
project_to_git(
    repo: &Repository,
    target_path: &Path,
    accessor: &Accessor,
) -> Result<()>  // async
```

Exports the bole repo to a bare Git repository at `target_path`. The export is filtered by `accessor` — paths and timelines the accessor cannot read are excluded.

- If `target_path` does not exist, a new bare repo is created.
- If `target_path` is an existing bare Git repo, it is opened and updated (idempotent).
- If `target_path` exists but is not a Git repo, returns `Err(Error::GitProjection)`.

Timelines become Git branches. Tags become Git tags. `HEAD` is set to the first projected timeline. The conversion is one-way; this is an export tool, not a round-trip sync.

```rust
project_to_git(&repo, Path::new("/tmp/export.git"), &Accessor::privileged()).await?;

// Now usable as a normal Git repo:
// git -C /tmp/export.git log --oneline
```

---

## In-memory workspaces

`EphemeralWorkspace` is a pure in-RAM working tree — for agents and tools that
edit files as buffers and produce snapshots without touching the filesystem. It
works on any `Repository`, but pairs naturally with `Repository::memory()`.

```rust
// Construct (empty, or seeded from a snapshot)
repo.ephemeral_workspace() -> EphemeralWorkspace<'_>
repo.ephemeral_workspace_from(snapshot: ObjectId) -> Result<EphemeralWorkspace<'_>>

// Edit
ws.read(path: &str) -> Option<&[u8]>
ws.write(path: impl Into<String>, bytes: impl Into<Bytes>)
ws.remove(path: &str) -> bool
ws.paths() -> impl Iterator<Item = &str>

// Inspect / persist
ws.diff() -> Result<PathDiff>                                   // vs the base snapshot
ws.commit(author, message, created_at: u64) -> Result<ObjectId> // new snapshot; parent = base
ws.base() -> Option<ObjectId>
```

`commit` stores the files as blobs and a snapshot whose parent is the current
base, then advances the workspace's base to the new snapshot. It does **not**
move any timeline — publish a snapshot by calling
[`Repository::advance_timeline`](#advance_timeline) with it.

```rust
let repo = Repository::memory();
let mut ws = repo.ephemeral_workspace();
ws.write("src/main.rs", &b"fn main() {}"[..]);
let snap = ws.commit("agent", "scaffold", 0).await?;

let mut ws2 = repo.ephemeral_workspace_from(snap).await?;
ws2.write("src/main.rs", &b"fn main() { run(); }"[..]);
let d = ws2.diff().await?;          // d.modified == ["src/main.rs"]
let snap2 = ws2.commit("agent", "wire up", 1).await?;
```

`PathDiff { added, removed, modified }` is also produced by the standalone
primitives the CLI shares: `build_tree(objects, &map)`,
`snapshot_paths(objects, snapshot)`, and `diff_paths(&base, &target)`.

> Note: `diff` and `commit` store the current files' blobs in the object store
> (content-addressed, so this is idempotent). The MVP seeds the full snapshot;
> ACL-filtered seeding may come later.

---

## Extended API

The sections above cover the original surface. These modules were added by the
roadmap workstreams; each is summarised here with its key entry points.

### Envelope secrets and key providers (`bole::crypto`, `bole::SecretV2`)

Secrets are envelope-encrypted: a random per-secret **data key** encrypts the
value; the data key is **wrapped** under a **master key** provided by a
`KeyProvider`.

- `KeyProvider` (trait) — `active_key_ref()`, `wrap_dk(dk, aad)`, `unwrap_dk(wrapped, aad)`.
- `LocalKeyProvider::new(master_key, ref_prefix)` — a master key held in memory.
- `ProviderChain` — the read-side resolver: active provider first, then fallbacks,
  plus legacy raw v1 keys. `with_provider` / `push_provider` / `push_legacy_key`.
- `SecretV2::encrypt_envelope(plaintext, provider, aad)` / `decrypt(chain)` /
  `rewrap(old_chain, new_provider)` (master-key rotation, value untouched).
- `SecretAad::v2(label)` binds `{version, label}` into the AEAD.
- `ObjectStore::put_secret_enveloped` / `get_secret_resolved` and
  `Repository::resolve_overlay(overlay, chain, accessor, skip_unauthorized)` /
  `rekey(ids, old_chain, new_provider)`.
- KMS slot (feature `kms`): `crypto::kms::{KmsClient, KmsKeyProvider, LocalKmsClient}`.

### Storage: packs, GC, and cheap counts (`bole::store`)

`Repository::disk` uses a `PackedDiskBackend` — loose objects first, then immutable
packs. A loose-only repo behaves exactly as before.

- `ObjectStore::count()` — distinct object count (cheap on packs).
- `ObjectStore::compact()` — consolidate loose objects into a pack.
- `Repository::gc(extra_roots, grace_secs, now)` — mark-sweep from ref roots plus
  `extra_roots` (registry-rooted objects), honouring a grace window.
- `store::pack` — the self-verifying pack format (also the sync wire payload).

### Atomic ref transactions (`bole::RefTransaction`)

`Repository::transaction()` (or `RefStore::transaction()`) returns a builder:
`create_tag` / `move_tag` / `create_timeline` / `advance_head` / `delete_ref` /
`set`, plus CAS preconditions `expect` and `advance_head_if`. `commit()` applies
all-or-nothing (a write-ahead journal on disk; idempotent replay on `open`).

### Distributed sync (`bole::sync`)

- In-process core on `Repository`: `fetch(remote, from, accessor)`,
  `push(remote, to, timelines, accessor)`, `clone_from(from, accessor)`.
- `sync::negotiate::missing_closure` — the have/want missing-object walk.
- `sync::wire` — the `Message` protocol codec; `sync::transport::{Conn, InProcessConn, TcpConn}`;
  `sync::session::{serve, client_fetch, client_push}` — the transport-agnostic state machine.
- `sync::http::{http_fetch, http_push, serve_http_once}` — a minimal HTTP transport.
- `sync::authn::{Principal, ActorMap, accessor_for, RefSigner, verify_ref_op}` — map a
  connection principal to a WS1 `Accessor`; sign/verify ref updates.

### Policy authority and signed approvals (`bole::acl`)

- `acl::authority::{TrustStore, TrustAnchor, PolicySigner, verify_chain, reconcile}` —
  Ed25519-signed `PolicyRoot` chains verified to a trusted root (fail-closed),
  highest-rooted-wins conflict resolution.
- `acl::attestation::{Approver, ApproverRegistry, AttestationSigner, Attestation,
  verify_attestation, count_valid_approvals, SignedApprovalHook}` — signed,
  head-bound merge approvals.

### Git import (`bole::repo::git_import`)

`git_import(repo, source, identity_map_dir, ImportOptions)` imports branches/tags
from a local Git repo, with an `IdentityMap` sidecar for incremental round-trips.
`git_projection::project_to_git` (export) gains `project_to_git_mapped` and
annotated-tag export.

---

## Error handling

All fallible operations return `bole::Result<T>`, which is `std::result::Result<T, bole::Error>`.

```rust
pub enum Error {
    Codec(String),           // serialization/deserialization failure
    Storage(String),         // backend-level storage failure
    Io(std::io::Error),      // filesystem or network I/O error
    InvalidRefName(String),  // RefName::new given a string that violates naming rules
    WrongRefKind(String),    // operation targeted the wrong ref type
    AccessDenied(String),    // ACL check failed for the given accessor
    DecryptionFailed,        // wrong key or corrupted ciphertext in get_secret
    SecretNotUtf8,           // decrypted secret bytes are not valid UTF-8
    GitProjection(String),   // project_to_git failure
    PolicyViolation(String), // advance rejected by the timeline's TimelinePolicy
}
```

Use `?` to propagate errors. Match on `Error` variants when you need to distinguish failure modes:

```rust
match repo.objects.get_secret(&id, &wrong_key).await {
    Ok(Some(bytes)) => { /* decrypted */ }
    Ok(None) => { /* id not in store */ }
    Err(Error::DecryptionFailed) => { /* wrong key */ }
    Err(e) => return Err(e),
}
```

---

## Complete example

This example creates an in-memory repo with two files (one under a protected path), applies ACLs, snapshots the state, merges an agent branch, and exports to Git.

```rust
use bole::{
    Accessor, Error, MergeCheck, PathAcl, PathRole, Permission, Repository,
    Snapshot, TimelineRole, TreeEntry, EntryKind,
};
use bole::refs::{RefName, TimelinePolicy};
use bytes::Bytes;
use std::collections::BTreeMap;
use std::path::Path;

#[tokio::main]
async fn main() -> bole::Result<()> {
    let repo = Repository::memory();

    // A write-capable "owner" identity. (Accessor::privileged() is read-only.)
    let owner = Accessor::new()
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read });

    // ── Build initial snapshot ─────────────────────────────────────────────
    // Both entries are blobs; "secrets/db.key" is just a file under a path we
    // will protect with an ACL. (Encrypted Secret objects are separate — see
    // put_secret / EnvOverlay — and are not stored inside the snapshot tree.)

    let src_blob = repo.objects.put_blob(Bytes::from("fn main() {}")).await?;
    let key_blob = repo.objects.put_blob(Bytes::from("postgres://prod:s3cr3t@db/app")).await?;

    let mut entries = BTreeMap::new();
    entries.insert("src/main.rs".into(), TreeEntry { id: src_blob, kind: EntryKind::Blob });
    entries.insert("secrets/db.key".into(), TreeEntry { id: key_blob, kind: EntryKind::Blob });

    let root_tree = repo.objects.put_tree(entries).await?;
    let snap1 = repo.objects.put_snapshot(Snapshot {
        root: root_tree, parents: vec![],
        author: "alice".into(), created_at: 1_700_000_000,
        message: "initial commit".into(),
    }).await?;

    let main_ref = RefName::new("main")?;
    repo.refs.create_timeline(
        main_ref.clone(), snap1, TimelinePolicy::Unrestricted,
        1_700_000_000, "persistent".into(), None,
    )?;

    // ── Apply ACLs ─────────────────────────────────────────────────────────

    // Mark secrets/** as protected (merging it into an unprotected timeline requires approval)
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() })?;

    // ── Agent with limited access ──────────────────────────────────────────

    let agent = Accessor::new()
        .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })
        .with_timeline_role(TimelineRole {
            pattern: "agent/**".into(), permission: Permission::Write,
        });

    // Agent creates its own branch
    let agent_ref = RefName::new("agent/formatter")?;
    repo.refs.create_timeline(
        agent_ref.clone(), snap1, TimelinePolicy::Unrestricted,
        1_700_000_000, "ephemeral".into(), None,
    )?;

    // Agent updates src/main.rs
    let formatted = repo.objects.put_blob(Bytes::from("fn main() { }\n")).await?;
    let mut agent_entries = BTreeMap::new();
    agent_entries.insert("src/main.rs".into(), TreeEntry { id: formatted, kind: EntryKind::Blob });
    let agent_tree = repo.objects.put_tree(agent_entries).await?;
    let snap2 = repo.objects.put_snapshot(Snapshot {
        root: agent_tree, parents: vec![snap1],
        author: "agent/formatter".into(), created_at: 1_700_000_001,
        message: "format src/main.rs".into(),
    }).await?;
    repo.advance_timeline(&agent_ref, snap2, &agent).await?;

    // ── Check merge safety before merging ─────────────────────────────────

    // `owner` can write `main`, so a protected-path leak yields RequiresApproval
    // (an accessor without write on `main` would get Rejected instead).
    match repo.check_merge(&agent_ref, &main_ref, &owner).await? {
        MergeCheck::Allowed => println!("safe to merge"),
        MergeCheck::RequiresApproval(paths) => println!("needs approval for {:?}", paths),
        MergeCheck::Rejected(paths) => println!("rejected, cannot expose {:?}", paths),
    }

    // ── Merge and advance main ─────────────────────────────────────────────

    let merge = repo.merge_timelines(&agent_ref, &main_ref, &owner).await?;
    if merge.conflicts.is_empty() {
        let merged_tree = repo.objects.put_tree(merge.merged).await?;
        let merged_snap = repo.objects.put_snapshot(Snapshot {
            root: merged_tree, parents: vec![snap1, snap2],
            author: "alice".into(), created_at: 1_700_000_002,
            message: "merge agent/formatter".into(),
        }).await?;
        repo.advance_timeline(&main_ref, merged_snap, &owner).await?;
    }

    // ── Filtered view (agent cannot see secrets/**) ────────────────────────

    let view = repo.get_snapshot_filtered(
        repo.refs.get_timeline(&main_ref)?.unwrap().head,
        &agent,
    ).await?.unwrap();
    assert!(view.visible_paths.contains_key("src/main.rs"));
    assert!(!view.visible_paths.contains_key("secrets/db.key"));

    // ── Export to Git ──────────────────────────────────────────────────────

    bole::repo::git_projection::project_to_git(
        &repo, Path::new("/tmp/bole-export.git"), &owner,
    ).await?;

    Ok(())
}
```
