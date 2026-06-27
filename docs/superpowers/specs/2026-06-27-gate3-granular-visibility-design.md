# Gate 3: Granular Visibility

**Project:** bole — a next-generation version control system  
**Language:** Rust (async, tokio)  
**Date:** 2026-06-27  
**Spec ref:** spec.md Gate 3, Test T3

---

## Context

Gates 1, 2, and 5 delivered a content-addressed object store, a mutable reference layer (tags and timelines), and a unified `Repository` entry point with pluggable backends. Gate 3 adds a permission lattice: every path and timeline can be protected by an ACL, and all data access through `Repository` can be filtered to a caller's capability set.

Key architectural decisions made during brainstorming:
- **Pure capability model** — no identity strings; callers hold `HashSet<PathRole>` and `HashSet<TimelineRole>`, which are checked against stored ACLs. No authentication layer.
- **Two-axis roles** — `PathRole { glob, permission }` and `TimelineRole { pattern, permission }`, where permission is `Read | Write`. Glob matching uses `**`-style patterns spanning path separators.
- **Enforcement at the `Repository` layer** — `ObjectStore` and `RefStore` remain unaware of permissions. Filtered methods on `Repository` compute filtered views at read time.
- **`AclStore` as persisted third field** — ACL entries are stored in a new `AclStore` (same pluggable backend pattern as `RefStore`) and exposed as `repo.acls`.

---

## Core Types

```rust
// src/acl/mod.rs

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission { Read, Write }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PathRole {
    pub glob: String,        // e.g. "secrets/**", "src/**"
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRole {
    pub pattern: String,     // e.g. "leslie/private/**", "main"
    pub permission: Permission,
}

/// The caller's complete capability set. Constructed externally and passed
/// into filtered methods. An empty Accessor has no capabilities.
#[derive(Debug, Clone, Default)]
pub struct Accessor {
    pub path_roles: HashSet<PathRole>,
    pub timeline_roles: HashSet<TimelineRole>,
}

impl Accessor {
    pub fn new() -> Self { Self::default() }
    pub fn with_path_role(mut self, role: PathRole) -> Self;
    pub fn with_timeline_role(mut self, role: TimelineRole) -> Self;
    pub fn can_read_path(&self, path: &str) -> bool;
    pub fn can_write_path(&self, path: &str) -> bool;
    pub fn can_read_timeline(&self, name: &str) -> bool;
    pub fn can_write_timeline(&self, name: &str) -> bool;
}

/// A protected path pattern. Any path matching `glob` requires a caller
/// PathRole whose glob also matches the path and includes the required permission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathAcl {
    pub glob: String,
}

/// A protected timeline pattern. Any ref name matching `pattern` requires a
/// caller TimelineRole whose pattern also matches and includes the required permission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineAcl {
    pub pattern: String,
}
```

`PathAcl` and `TimelineAcl` are the persisted side — they declare which paths/timelines are protected. `PathRole` and `TimelineRole` are the caller's capability side. The check: path `secrets/prod.key` is protected if any stored `PathAcl` whose glob matches it exists; the caller can read it if any of their `PathRole` entries has a glob that also matches `secrets/prod.key` and permission `Read`.

---

## Glob Matching

Lives in `src/acl/glob.rs`:

```rust
/// Returns true if `path` matches `pattern`.
/// `*` matches any sequence of non-separator characters.
/// `**` matches any sequence of characters including `/`.
pub fn glob_matches(pattern: &str, path: &str) -> bool;
```

Examples:
- `"secrets/**"` matches `"secrets/prod.key"`, `"secrets/a/b/c"`
- `"secrets/**"` does not match `"src/main.rs"`
- `"*.rs"` matches `"main.rs"` but not `"src/main.rs"`
- `"src/**"` matches `"src/main.rs"`, `"src/a/b.rs"`

Implementation: iterative matching with `**` consuming full remaining path segments. No external crate — hand-rolled to keep the dependency count flat.

---

## AclBackend Trait

Sync trait — ACL entries are tiny:

```rust
// src/acl/backend.rs

pub trait AclBackend: Send + Sync {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>>;
    fn set_path_acl(&self, acl: &PathAcl) -> Result<()>;
    fn delete_path_acl(&self, glob: &str) -> Result<()>;
    fn list_path_acls(&self) -> Result<Vec<PathAcl>>;

    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>>;
    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()>;
    fn delete_timeline_acl(&self, pattern: &str) -> Result<()>;
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>>;
}
```

### MemoryAclBackend

`Arc<RwLock<(HashMap<String, PathAcl>, HashMap<String, TimelineAcl>)>>`. Clone-safe. No persistence.

### DiskAclBackend

Two subdirectories under `<root>/acls/`:
- `<root>/acls/paths/<glob-as-sanitized-filename>` — postcard-encoded `PathAcl`
- `<root>/acls/timelines/<pattern-as-sanitized-filename>` — postcard-encoded `TimelineAcl`

Atomic writes use the leading-dot temp file scheme established in `DiskRefBackend` (write to `.<name>.tmp`, rename). `DiskAclBackend::open(root: impl AsRef<Path>) -> Result<Self>` creates directories on first use.

Glob strings may contain `/` and `*` which are not valid in filenames on some platforms. Sanitize by replacing `/` with `%2F` and `*` with `%2A` for on-disk names. Reverse on read.

---

## AclStore

```rust
// src/acl/mod.rs

pub struct AclStore {
    backend: Box<dyn AclBackend>,
}

impl AclStore {
    pub fn new(backend: impl AclBackend + 'static) -> Self;

    pub fn set_path_acl(&self, acl: PathAcl) -> Result<()>;
    pub fn remove_path_acl(&self, glob: &str) -> Result<()>;
    pub fn list_path_acls(&self) -> Result<Vec<PathAcl>>;

    pub fn set_timeline_acl(&self, acl: TimelineAcl) -> Result<()>;
    pub fn remove_timeline_acl(&self, pattern: &str) -> Result<()>;
    pub fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>>;

    /// Returns true if `path` is protected by any stored PathAcl.
    pub fn path_is_protected(&self, path: &str) -> Result<bool>;

    /// Returns true if `name` is protected by any stored TimelineAcl.
    pub fn timeline_is_protected(&self, name: &str) -> Result<bool>;
}
```

---

## Repository Extension

`Repository` gains a third public field and four new methods:

```rust
pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
    pub acls: AclStore,       // new
}

impl Repository {
    pub fn memory() -> Self;                               // + MemoryAclBackend
    pub async fn disk(root: impl AsRef<Path>) -> Result<Self>;  // + DiskAclBackend

    /// Returns the snapshot with tree entries filtered to only paths
    /// the accessor can read. Walks the tree recursively — a protected
    /// path segment removes the whole subtree. Returns None if the
    /// snapshot does not exist.
    pub async fn get_snapshot_filtered(
        &self,
        id: ObjectId,
        accessor: &Accessor,
    ) -> Result<Option<FilteredSnapshot>>;

    /// Lists ref names under `prefix`, omitting timelines/tags the
    /// accessor cannot read.
    pub fn list_refs_filtered(
        &self,
        prefix: &str,
        accessor: &Accessor,
    ) -> Result<Vec<RefName>>;

    /// Checks whether merging `source`'s head into `dest` would expose
    /// protected paths to callers of `dest` who lack the required roles.
    pub async fn check_merge(
        &self,
        source: &RefName,
        dest: &RefName,
        accessor: &Accessor,
    ) -> Result<MergeCheck>;
}
```

### FilteredSnapshot

Not stored in the object store — computed on read:

```rust
pub struct FilteredSnapshot {
    pub id: ObjectId,
    pub author: String,
    pub created_at: u64,
    pub message: String,
    pub parents: Vec<ObjectId>,
    pub visible_paths: BTreeMap<String, ObjectId>,  // path → blob id, filtered
}
```

`visible_paths` is flat (all leaf paths with their blob IDs). The full-path key is built by joining tree segment names with `/` during recursive walk. Only `EntryKind::Blob` entries that pass the ACL check appear.

### MergeCheck

```rust
pub enum MergeCheck {
    /// No protected paths would be exposed; merge may proceed.
    Allowed,
    /// Merge would expose these protected paths to unauthorized readers of
    /// dest. Caller has Write on dest, so explicit approval could unblock it.
    RequiresApproval(Vec<PathAcl>),
    /// Caller lacks Write permission on dest — merge cannot proceed.
    Rejected(Vec<PathAcl>),
}
```

`check_merge` algorithm:
1. Resolve `source`'s head snapshot, walk all leaf paths.
2. For each path, call `acls.path_is_protected(path)?`.
3. If protected: check whether `dest`'s timeline ACL (if any) requires the same or stronger role. If not, the path would leak.
4. Collect all leaking `PathAcl` entries.
5. If no leaks: `Allowed`.
6. If leaks and `accessor.can_write_timeline(dest.as_str())`: `RequiresApproval(leaking)`.
7. Otherwise: `Rejected(leaking)`.

---

## Error Extension

One new variant in `src/error.rs`:

```rust
#[error("access denied: {0}")]
AccessDenied(String),
```

Not returned by the filtered methods (they silently omit inaccessible data). Used if a caller attempts a direct write operation (e.g., `create_tag`) on a timeline they lack `Write` for — future gate enforcement surface.

---

## Crate Structure Changes

```
src/
├── lib.rs              # add: pub mod acl; re-export Accessor, PathRole, TimelineRole,
│                       #      Permission, PathAcl, TimelineAcl, AclStore,
│                       #      FilteredSnapshot, MergeCheck
├── acl/
│   ├── mod.rs          # AclStore, PathAcl, TimelineAcl, Accessor, PathRole,
│   │                   # TimelineRole, Permission, FilteredSnapshot, MergeCheck
│   ├── backend.rs      # AclBackend trait
│   ├── memory.rs       # MemoryAclBackend
│   ├── disk.rs         # DiskAclBackend
│   └── glob.rs         # glob_matches helper
├── repo/mod.rs         # add acls field, get_snapshot_filtered, list_refs_filtered,
│                       # check_merge
└── error.rs            # add AccessDenied variant

tests/
└── acl.rs              # T3 integration tests
```

---

## Testing Approach

### Unit tests (in-module)

**`glob.rs`:**
- `"secrets/**"` matches `"secrets/prod.key"`, `"secrets/a/b"` — ✓
- `"secrets/**"` does not match `"src/main.rs"` — ✓
- `"src/**"` matches `"src/main.rs"`, `"src/a/b.rs"` — ✓
- `"*.rs"` matches `"main.rs"`, does not match `"src/main.rs"` — ✓
- Exact match: `"README.md"` matches `"README.md"` only — ✓

**`AclBackend` contract suite** (shared fn, run against both `MemoryAclBackend` and `DiskAclBackend`):
- set/get/delete path ACL
- set/get/delete timeline ACL
- list returns all entries
- `DiskAclBackend` persists across reopen

**`Accessor`:**
- `can_read_path` — empty accessor → false for protected path
- `can_read_path` — correct glob role → true
- `can_read_path` — wrong glob → false

### T3 integration tests (`tests/acl.rs`, all `#[tokio::test]` where async)

**`t3_path_filtering`:**
- Set `PathAcl { glob: "secrets/**" }` and `PathAcl { glob: "notes/**" }`.
- Build snapshot with paths: `src/app.rs`, `secrets/prod.key`, `notes/private.md`.
- Empty `Accessor` → `visible_paths` contains only `src/app.rs`.
- `Accessor` with `PathRole("secrets/**", Read)` → `src/app.rs` + `secrets/prod.key`.
- `Accessor` with both roles → all three paths.

**`t3_timeline_filtering`:**
- Set `TimelineAcl { pattern: "leslie/private/**" }`.
- Create tags: `"main"` and `"leslie/private/exp-foo"`.
- `list_refs_filtered("")` with empty accessor → returns `"main"` only.
- With `TimelineRole("leslie/private/**", Read)` → returns both.

**`t3_merge_check`:**
- Source timeline head snapshot contains `secrets/prod.key`.
- `PathAcl { glob: "secrets/**" }` in AclStore.
- Dest is a public timeline with no path ACL requirement.
- `check_merge` with `TimelineRole("dest", Write)` → `RequiresApproval([PathAcl { glob: "secrets/**" }])`.
- `check_merge` with empty accessor (no write on dest) → `Rejected([PathAcl { glob: "secrets/**" }])`.
- `check_merge` with no protected source paths → `Allowed`.

---

## Key Dependencies

No new crates required. All dependencies from Gates 1, 2, and 5 apply. `glob_matches` is hand-rolled.

| Crate | Gate 3 use |
|-------|-----------|
| `serde` + `postcard` | serialize `PathAcl`, `TimelineAcl` for disk storage |
| `thiserror` | new `AccessDenied` error variant |
| `std::sync::RwLock` | `MemoryAclBackend` interior mutability |
| `std::fs` | `DiskAclBackend` (sync I/O, same as `DiskRefBackend`) |
| `tokio` | async tree walk in `get_snapshot_filtered` |

---

## Out of Scope (Gate 3)

- Enforcement on write paths (`create_tag`, `advance_head`) — Gate 6
- Capability token issuance and signing (crypto) — Gate 6
- Merge execution (applying the merge) — Gate 6
- Audit logging of access decisions — non-functional requirement, future
- Secret encryption — Gate 4
- Git export ACL filtering — Gate 7
