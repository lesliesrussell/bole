# Gate 3: Granular Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a permission lattice to bole: ACL storage, caller capability sets, filtered snapshot reads, filtered ref listing, and merge-safety checks.

**Architecture:** A new `src/acl/` module holds all ACL types and backends following the same pattern as `src/refs/`. `Repository` gains a third public field `acls: AclStore` and three new filtered methods. ACL checks happen at the `Repository` layer only — `ObjectStore` and `RefStore` remain unaware of permissions.

**Tech Stack:** Rust (edition 2021, stable, tokio async), serde + postcard, thiserror — all already in Cargo.toml. No new dependencies.

## Global Constraints

- `thiserror` only — no `anyhow` anywhere in library code
- Both `MemoryAclBackend` and `DiskAclBackend` always compiled — no feature flags
- Bead required before any code: `bd create`, `bd update <id> --claim`, `git checkout -b <id>`
- Branch name = bead ID exactly
- Tests must pass before merge; after merge: `git branch -d <id>` then `bd close <id>`
- Conservative git profile: no push, no dolt sync without explicit request
- `// <bead-id>` comment on each contiguous block of new code — one per block, not per line
- No new crate dependencies — `glob_matches` is hand-rolled

---

## File Map

| File | Status | Purpose |
|------|--------|---------|
| `src/acl/glob.rs` | Create | `glob_matches(pattern, path) -> bool` helper |
| `src/acl/backend.rs` | Create | `AclBackend` sync trait |
| `src/acl/memory.rs` | Create | `MemoryAclBackend` |
| `src/acl/disk.rs` | Create | `DiskAclBackend` |
| `src/acl/mod.rs` | Create | All public ACL types + `AclStore` |
| `src/error.rs` | Modify | Add `AccessDenied(String)` variant |
| `src/repo/mod.rs` | Modify | Add `acls: AclStore` field, update constructors, add filtered methods |
| `src/lib.rs` | Modify | `pub mod acl` + re-exports |
| `tests/acl.rs` | Create | T3 integration tests |

---

## Task 1: `glob_matches` + `AclBackend` + `MemoryAclBackend`

**Files:**
- Create: `src/acl/glob.rs`
- Create: `src/acl/backend.rs`
- Create: `src/acl/memory.rs`
- Create: `src/acl/mod.rs` (types + `AclStore` + `MemoryAclBackend` wiring)
- Modify: `src/error.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::error::{Error, Result}` (existing)
- Produces:
  - `pub fn glob_matches(pattern: &str, path: &str) -> bool`
  - `pub enum Permission { Read, Write }`
  - `pub struct PathRole { pub glob: String, pub permission: Permission }`
  - `pub struct TimelineRole { pub pattern: String, pub permission: Permission }`
  - `pub struct Accessor { pub path_roles: HashSet<PathRole>, pub timeline_roles: HashSet<TimelineRole> }` with `::new()`, `can_read_path(&str) -> bool`, `can_write_path(&str) -> bool`, `can_read_timeline(&str) -> bool`, `can_write_timeline(&str) -> bool`
  - `pub struct PathAcl { pub glob: String }`
  - `pub struct TimelineAcl { pub pattern: String }`
  - `pub trait AclBackend: Send + Sync` with 8 methods (4 path, 4 timeline)
  - `pub struct MemoryAclBackend` implementing `AclBackend`
  - `pub struct AclStore` with 6 public methods + `path_is_protected` + `timeline_is_protected`
  - `AccessDenied(String)` variant in `Error`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 3 T1: glob, AclBackend, MemoryAclBackend" \
  --description="Hand-rolled glob_matches helper, AclBackend sync trait, MemoryAclBackend (Arc<RwLock<HashMap>>), all ACL types (Permission, PathRole, TimelineRole, Accessor, PathAcl, TimelineAcl), AclStore, AccessDenied error variant, lib.rs wiring." \
  --type=task --priority=2
# Note the printed bead ID, e.g. bole-abc
bd update bole-abc --claim
git checkout -b bole-abc
```

- [ ] **Step 2: Write failing tests for `glob_matches`**

Create `src/acl/glob.rs` with tests first:

```rust
// <bead-id>
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    #[test]
    fn double_star_matches_nested() {
        assert!(glob_matches("secrets/**", "secrets/prod.key"));
        assert!(glob_matches("secrets/**", "secrets/a/b/c"));
        assert!(!glob_matches("secrets/**", "src/main.rs"));
    }

    #[test]
    fn double_star_matches_direct_child() {
        assert!(glob_matches("src/**", "src/main.rs"));
    }

    #[test]
    fn single_star_does_not_span_separator() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(!glob_matches("*.rs", "src/main.rs"));
    }

    #[test]
    fn exact_match() {
        assert!(glob_matches("README.md", "README.md"));
        assert!(!glob_matches("README.md", "readme.md"));
    }

    #[test]
    fn no_pattern_chars_literal() {
        assert!(glob_matches("src/lib.rs", "src/lib.rs"));
        assert!(!glob_matches("src/lib.rs", "src/main.rs"));
    }

    #[test]
    fn star_star_at_root() {
        assert!(glob_matches("**", "anything/nested/deeply"));
        assert!(glob_matches("**", "flat"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test glob 2>&1 | head -20
```

Expected: `todo!()` panics.

- [ ] **Step 4: Implement `glob_matches`**

Replace the `todo!()` in `src/acl/glob.rs` (keep the `#[cfg(test)]` block unchanged):

```rust
// <bead-id>
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    fn matches(pat: &[u8], s: &[u8]) -> bool {
        match (pat, s) {
            ([], []) => true,
            ([], _) => false,
            ([b'*', b'*', rest @ ..], _) => {
                // ** matches zero or more path segments
                if matches(rest, s) { return true; }
                for i in 0..=s.len() {
                    if i == s.len() || s[i] == b'/' {
                        let tail = if i == s.len() { &s[i..] } else { &s[i + 1..] };
                        if matches(rest, tail) { return true; }
                    }
                }
                false
            }
            ([b'*', rest @ ..], _) => {
                // * matches any sequence of non-separator chars
                let mut i = 0;
                loop {
                    if matches(rest, &s[i..]) { return true; }
                    if i == s.len() || s[i] == b'/' { return false; }
                    i += 1;
                }
            }
            ([p, pat_rest @ ..], [c, s_rest @ ..]) if p == c => matches(pat_rest, s_rest),
            _ => false,
        }
    }
    matches(pattern.as_bytes(), path.as_bytes())
}
```

- [ ] **Step 5: Run glob tests**

```bash
cargo test glob 2>&1 | tail -15
```

Expected: all 6 glob tests pass.

- [ ] **Step 6: Write ACL type + AclBackend + MemoryAclBackend failing tests**

Create `src/acl/backend.rs`:

```rust
// <bead-id>
use crate::error::Result;
use crate::acl::{PathAcl, TimelineAcl};

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

Create `src/acl/memory.rs` with tests:

```rust
// <bead-id>
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
use crate::error::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct MemoryAclBackend {
    path_acls: Arc<RwLock<HashMap<String, PathAcl>>>,
    timeline_acls: Arc<RwLock<HashMap<String, TimelineAcl>>>,
}

impl MemoryAclBackend {
    pub fn new() -> Self { Self::default() }
}

impl AclBackend for MemoryAclBackend {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>> {
        Ok(self.path_acls.read().unwrap().get(glob).cloned())
    }
    fn set_path_acl(&self, acl: &PathAcl) -> Result<()> {
        self.path_acls.write().unwrap().insert(acl.glob.clone(), acl.clone());
        Ok(())
    }
    fn delete_path_acl(&self, glob: &str) -> Result<()> {
        self.path_acls.write().unwrap().remove(glob);
        Ok(())
    }
    fn list_path_acls(&self) -> Result<Vec<PathAcl>> {
        Ok(self.path_acls.read().unwrap().values().cloned().collect())
    }
    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>> {
        Ok(self.timeline_acls.read().unwrap().get(pattern).cloned())
    }
    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()> {
        self.timeline_acls.write().unwrap().insert(acl.pattern.clone(), acl.clone());
        Ok(())
    }
    fn delete_timeline_acl(&self, pattern: &str) -> Result<()> {
        self.timeline_acls.write().unwrap().remove(pattern);
        Ok(())
    }
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> {
        Ok(self.timeline_acls.read().unwrap().values().cloned().collect())
    }
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::MemoryAclBackend;
    use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};

    #[test]
    fn path_acl_set_get_delete() {
        let b = MemoryAclBackend::new();
        let acl = PathAcl { glob: "secrets/**".into() };
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("secrets/**").unwrap(), Some(acl));
        b.delete_path_acl("secrets/**").unwrap();
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
    }

    #[test]
    fn timeline_acl_set_get_delete() {
        let b = MemoryAclBackend::new();
        let acl = TimelineAcl { pattern: "leslie/private/**".into() };
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
        b.set_timeline_acl(&acl).unwrap();
        assert_eq!(b.get_timeline_acl("leslie/private/**").unwrap(), Some(acl));
        b.delete_timeline_acl("leslie/private/**").unwrap();
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
    }

    #[test]
    fn list_returns_all_entries() {
        let b = MemoryAclBackend::new();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        b.set_path_acl(&PathAcl { glob: "notes/**".into() }).unwrap();
        let mut list = b.list_path_acls().unwrap();
        list.sort_by(|a, b| a.glob.cmp(&b.glob));
        assert_eq!(list[0].glob, "notes/**");
        assert_eq!(list[1].glob, "secrets/**");
    }
}
```

Create `src/acl/mod.rs` with all types, `AclStore`, and `Accessor`:

```rust
// <bead-id>
pub mod backend;
pub mod disk;
pub mod glob;
pub mod memory;

use crate::error::Result;
use backend::AclBackend;
use glob::glob_matches;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission { Read, Write }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PathRole {
    pub glob: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRole {
    pub pattern: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathAcl {
    pub glob: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineAcl {
    pub pattern: String,
}

// <bead-id>
#[derive(Debug, Clone, Default)]
pub struct Accessor {
    pub path_roles: HashSet<PathRole>,
    pub timeline_roles: HashSet<TimelineRole>,
}

impl Accessor {
    pub fn new() -> Self { Self::default() }

    pub fn with_path_role(mut self, role: PathRole) -> Self {
        self.path_roles.insert(role);
        self
    }

    pub fn with_timeline_role(mut self, role: TimelineRole) -> Self {
        self.timeline_roles.insert(role);
        self
    }

    pub fn can_read_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.glob, path)
        )
    }

    pub fn can_write_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.glob, path)
        )
    }

    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.pattern, name)
        )
    }

    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.pattern, name)
        )
    }
}

// <bead-id>
pub struct AclStore {
    backend: Box<dyn AclBackend>,
}

impl AclStore {
    pub fn new(backend: impl AclBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    pub fn set_path_acl(&self, acl: PathAcl) -> Result<()> { self.backend.set_path_acl(&acl) }
    pub fn remove_path_acl(&self, glob: &str) -> Result<()> { self.backend.delete_path_acl(glob) }
    pub fn list_path_acls(&self) -> Result<Vec<PathAcl>> { self.backend.list_path_acls() }

    pub fn set_timeline_acl(&self, acl: TimelineAcl) -> Result<()> { self.backend.set_timeline_acl(&acl) }
    pub fn remove_timeline_acl(&self, pattern: &str) -> Result<()> { self.backend.delete_timeline_acl(pattern) }
    pub fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> { self.backend.list_timeline_acls() }

    pub fn path_is_protected(&self, path: &str) -> Result<bool> {
        Ok(self.backend.list_path_acls()?.iter().any(|a| glob_matches(&a.glob, path)))
    }

    pub fn timeline_is_protected(&self, name: &str) -> Result<bool> {
        Ok(self.backend.list_timeline_acls()?.iter().any(|a| glob_matches(&a.pattern, name)))
    }
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::{Accessor, PathRole, Permission, TimelineRole};

    #[test]
    fn empty_accessor_cannot_read_anything() {
        let a = Accessor::new();
        assert!(!a.can_read_path("secrets/prod.key"));
        assert!(!a.can_read_timeline("leslie/private/exp"));
    }

    #[test]
    fn matching_role_grants_read() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
        assert!(a.can_read_path("secrets/prod.key"));
        assert!(a.can_read_path("secrets/a/b"));
        assert!(!a.can_read_path("src/main.rs"));
    }

    #[test]
    fn write_role_does_not_grant_read() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Write });
        assert!(!a.can_read_path("secrets/prod.key"));
        assert!(a.can_write_path("secrets/prod.key"));
    }

    #[test]
    fn timeline_role_matching() {
        let a = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "leslie/private/**".into(), permission: Permission::Read });
        assert!(a.can_read_timeline("leslie/private/exp-foo"));
        assert!(!a.can_read_timeline("main"));
    }
}
```

- [ ] **Step 7: Add `AccessDenied` to `src/error.rs`**

In `src/error.rs`, add the new variant (keep the `// bole-49r` and `// bole-s5y` comments):

```rust
// bole-49r
// bole-s5y
// <bead-id>
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("codec error: {0}")] Codec(String),
    #[error("storage error: {0}")] Storage(String),
    #[error("io error: {0}")] Io(#[from] std::io::Error),
    #[error("invalid ref name: {0}")] InvalidRefName(String),
    #[error("wrong ref kind: {0}")] WrongRefKind(String),
    #[error("access denied: {0}")] AccessDenied(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 8: Add `disk` stub so module compiles**

Create `src/acl/disk.rs` as a compilable stub (Task 2 will implement it):

```rust
// <bead-id>
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
use crate::error::Result;
use std::path::{Path, PathBuf};

pub struct DiskAclBackend {
    root: PathBuf,
}

impl DiskAclBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }
}

impl AclBackend for DiskAclBackend {
    fn get_path_acl(&self, _glob: &str) -> Result<Option<PathAcl>> { todo!() }
    fn set_path_acl(&self, _acl: &PathAcl) -> Result<()> { todo!() }
    fn delete_path_acl(&self, _glob: &str) -> Result<()> { todo!() }
    fn list_path_acls(&self) -> Result<Vec<PathAcl>> { todo!() }
    fn get_timeline_acl(&self, _pattern: &str) -> Result<Option<TimelineAcl>> { todo!() }
    fn set_timeline_acl(&self, _acl: &TimelineAcl) -> Result<()> { todo!() }
    fn delete_timeline_acl(&self, _pattern: &str) -> Result<()> { todo!() }
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> { todo!() }
}
```

- [ ] **Step 9: Wire `pub mod acl` in `src/lib.rs`**

Add to `src/lib.rs` after `pub mod repo;`:

```rust
// <bead-id>
pub mod acl;
pub use acl::{
    Accessor, AclStore, PathAcl, PathRole, Permission, TimelineAcl, TimelineRole,
};
pub use error::AccessDenied;
```

Wait — `AccessDenied` is an enum *variant*, not a type. Do not re-export it separately. Instead update the existing `pub use error::{Error, Result};` line to just keep as-is (callers use `Error::AccessDenied`). Remove the `pub use error::AccessDenied;` line above. The correct lib.rs addition is:

```rust
// <bead-id>
pub mod acl;
pub use acl::{
    Accessor, AclStore, PathAcl, PathRole, Permission, TimelineAcl, TimelineRole,
};
```

- [ ] **Step 10: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass plus the new glob and ACL type tests. If `disk` module causes `todo!()` panics, verify no tests call `DiskAclBackend` methods directly yet.

- [ ] **Step 11: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -30
```

Expected: no warnings.

- [ ] **Step 12: Commit**

```bash
git add src/acl/ src/error.rs src/lib.rs
git commit -m "feat(acl): add glob_matches, AclBackend, MemoryAclBackend, AclStore, Accessor"
```

- [ ] **Step 13: Merge and close**

```bash
git checkout master && git merge bole-abc
git branch -d bole-abc
bd close bole-abc
```

---

## Task 2: `DiskAclBackend`

**Files:**
- Modify: `src/acl/disk.rs` (replace stub with full implementation)

**Interfaces:**
- Consumes:
  - `AclBackend` trait from Task 1
  - `PathAcl { pub glob: String }` from Task 1
  - `TimelineAcl { pub pattern: String }` from Task 1
  - `crate::error::{Error, Result}` (existing)
  - postcard codec: use `postcard::to_allocvec` and `postcard::from_bytes`
- Produces:
  - `pub struct DiskAclBackend` fully implementing `AclBackend`
  - `DiskAclBackend::open(root: impl AsRef<Path>) -> Result<Self>` (sync)

**Filename sanitization:** glob strings contain `/` and `*` which are invalid in filenames on some systems. Replace `/` with `%2F` and `*` with `%2A` when writing. Reverse on read.

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 3 T2: DiskAclBackend" \
  --description="Full DiskAclBackend implementation: files under <root>/acls/paths/ and <root>/acls/timelines/, postcard encoding, glob-to-filename sanitization (%2F/%2A), atomic leading-dot temp writes, persist across reopen." \
  --type=task --priority=2
bd update bole-def --claim
git checkout -b bole-def
```

- [ ] **Step 2: Write failing tests**

Add to the end of `src/acl/disk.rs` (after the stub impl):

```rust
// <bead-id>
#[cfg(test)]
mod tests {
    use super::DiskAclBackend;
    use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
    use tempfile::TempDir;

    #[test]
    fn path_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "secrets/**".into() };
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("secrets/**").unwrap(), Some(acl));
        b.delete_path_acl("secrets/**").unwrap();
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
    }

    #[test]
    fn timeline_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = TimelineAcl { pattern: "leslie/private/**".into() };
        b.set_timeline_acl(&acl).unwrap();
        assert_eq!(b.get_timeline_acl("leslie/private/**").unwrap(), Some(acl));
        b.delete_timeline_acl("leslie/private/**").unwrap();
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let b = DiskAclBackend::open(dir.path()).unwrap();
            b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
            b.set_timeline_acl(&TimelineAcl { pattern: "private/**".into() }).unwrap();
        }
        let b2 = DiskAclBackend::open(dir.path()).unwrap();
        assert!(b2.get_path_acl("secrets/**").unwrap().is_some());
        assert!(b2.get_timeline_acl("private/**").unwrap().is_some());
    }

    #[test]
    fn list_returns_all() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        b.set_path_acl(&PathAcl { glob: "notes/**".into() }).unwrap();
        let mut list = b.list_path_acls().unwrap();
        list.sort_by(|a, c| a.glob.cmp(&c.glob));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].glob, "notes/**");
        assert_eq!(list[1].glob, "secrets/**");
    }

    #[test]
    fn glob_with_slash_roundtrips() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "a/b/**".into() };
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("a/b/**").unwrap(), Some(acl));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test disk_acl 2>&1 | head -20
```

Expected: `todo!()` panics.

- [ ] **Step 4: Implement `DiskAclBackend`**

Replace the entire `src/acl/disk.rs` with:

```rust
// <bead-id>
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

fn sanitize(s: &str) -> String {
    s.replace('/', "%2F").replace('*', "%2A")
}

fn desanitize(s: &str) -> String {
    s.replace("%2F", "/").replace("%2A", "*")
}

pub struct DiskAclBackend {
    root: PathBuf,
}

impl DiskAclBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("acls").join("paths"))?;
        fs::create_dir_all(root.join("acls").join("timelines"))?;
        Ok(Self { root })
    }

    fn path_acl_file(&self, glob: &str) -> PathBuf {
        self.root.join("acls").join("paths").join(sanitize(glob))
    }

    fn timeline_acl_file(&self, pattern: &str) -> PathBuf {
        self.root.join("acls").join("timelines").join(sanitize(pattern))
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> Result<()> {
        let tmp_name = format!(".{}.tmp",
            path.file_name().unwrap().to_string_lossy());
        let tmp = path.parent().unwrap().join(tmp_name);
        fs::write(&tmp, data)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    fn list_dir<T, F>(&self, dir: PathBuf, decode: F) -> Result<Vec<T>>
    where
        F: Fn(&str, &[u8]) -> Result<T>,
    {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') { continue; }
            let data = fs::read(entry.path())?;
            out.push(decode(&name, &data)?);
        }
        Ok(out)
    }
}

// <bead-id>
impl AclBackend for DiskAclBackend {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>> {
        let path = self.path_acl_file(glob);
        match fs::read(&path) {
            Ok(data) => Ok(Some(postcard::from_bytes(&data)
                .map_err(|e| Error::Codec(e.to_string()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set_path_acl(&self, acl: &PathAcl) -> Result<()> {
        let data = postcard::to_allocvec(acl).map_err(|e| Error::Codec(e.to_string()))?;
        self.atomic_write(&self.path_acl_file(&acl.glob), &data)
    }

    fn delete_path_acl(&self, glob: &str) -> Result<()> {
        match fs::remove_file(self.path_acl_file(glob)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list_path_acls(&self) -> Result<Vec<PathAcl>> {
        self.list_dir(
            self.root.join("acls").join("paths"),
            |_name, data| postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string())),
        )
    }

    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>> {
        let path = self.timeline_acl_file(pattern);
        match fs::read(&path) {
            Ok(data) => Ok(Some(postcard::from_bytes(&data)
                .map_err(|e| Error::Codec(e.to_string()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()> {
        let data = postcard::to_allocvec(acl).map_err(|e| Error::Codec(e.to_string()))?;
        self.atomic_write(&self.timeline_acl_file(&acl.pattern), &data)
    }

    fn delete_timeline_acl(&self, pattern: &str) -> Result<()> {
        match fs::remove_file(self.timeline_acl_file(pattern)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> {
        self.list_dir(
            self.root.join("acls").join("timelines"),
            |_name, data| postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string())),
        )
    }
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::DiskAclBackend;
    use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
    use tempfile::TempDir;

    #[test]
    fn path_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "secrets/**".into() };
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("secrets/**").unwrap(), Some(acl));
        b.delete_path_acl("secrets/**").unwrap();
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
    }

    #[test]
    fn timeline_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = TimelineAcl { pattern: "leslie/private/**".into() };
        b.set_timeline_acl(&acl).unwrap();
        assert_eq!(b.get_timeline_acl("leslie/private/**").unwrap(), Some(acl));
        b.delete_timeline_acl("leslie/private/**").unwrap();
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let b = DiskAclBackend::open(dir.path()).unwrap();
            b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
            b.set_timeline_acl(&TimelineAcl { pattern: "private/**".into() }).unwrap();
        }
        let b2 = DiskAclBackend::open(dir.path()).unwrap();
        assert!(b2.get_path_acl("secrets/**").unwrap().is_some());
        assert!(b2.get_timeline_acl("private/**").unwrap().is_some());
    }

    #[test]
    fn list_returns_all() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        b.set_path_acl(&PathAcl { glob: "notes/**".into() }).unwrap();
        let mut list = b.list_path_acls().unwrap();
        list.sort_by(|a, c| a.glob.cmp(&c.glob));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].glob, "notes/**");
        assert_eq!(list[1].glob, "secrets/**");
    }

    #[test]
    fn glob_with_slash_roundtrips() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "a/b/**".into() };
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("a/b/**").unwrap(), Some(acl));
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass including the new DiskAclBackend tests.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/acl/disk.rs
git commit -m "feat(acl): implement DiskAclBackend with sanitized filenames and atomic writes"
```

- [ ] **Step 8: Merge and close**

```bash
git checkout master && git merge bole-def
git branch -d bole-def
bd close bole-def
```

---

## Task 3: `Repository` filtered methods + `FilteredSnapshot` + `MergeCheck`

**Files:**
- Modify: `src/repo/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes (from Tasks 1+2):
  - `AclStore::new(backend)`, `AclStore::path_is_protected(&str) -> Result<bool>`, `AclStore::timeline_is_protected(&str) -> Result<bool>`, `AclStore::list_path_acls() -> Result<Vec<PathAcl>>`
  - `Accessor::can_read_path(&str) -> bool`, `Accessor::can_read_timeline(&str) -> bool`, `Accessor::can_write_timeline(&str) -> bool`
  - `MemoryAclBackend::new()` from `crate::acl::memory`
  - `DiskAclBackend::open(root) -> Result<Self>` from `crate::acl::disk`
  - `Object::Snapshot(Snapshot)`, `Object::Tree(Tree)`, `Object::Blob(Blob)`
  - `Tree { entries: BTreeMap<String, TreeEntry> }`, `TreeEntry { id: ObjectId, kind: EntryKind }`
  - `EntryKind::Blob | Tree`
  - `RefStore::list(&str) -> Result<Vec<RefName>>`, `RefStore::get_timeline(&RefName) -> Result<Option<Timeline>>`
- Produces:
  - `pub struct FilteredSnapshot { pub id: ObjectId, pub author: String, pub created_at: u64, pub message: String, pub parents: Vec<ObjectId>, pub visible_paths: BTreeMap<String, ObjectId> }`
  - `pub enum MergeCheck { Allowed, RequiresApproval(Vec<PathAcl>), Rejected(Vec<PathAcl>) }`
  - `Repository { pub objects, pub refs, pub acls: AclStore }` (updated)
  - `Repository::memory() -> Self` (updated)
  - `pub async fn Repository::disk(root) -> Result<Self>` (updated)
  - `pub async fn Repository::get_snapshot_filtered(id, accessor) -> Result<Option<FilteredSnapshot>>`
  - `pub fn Repository::list_refs_filtered(prefix, accessor) -> Result<Vec<RefName>>`
  - `pub async fn Repository::check_merge(source, dest, accessor) -> Result<MergeCheck>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 3 T3: Repository filtered methods" \
  --description="Add acls: AclStore to Repository, update memory()/disk() constructors. Add FilteredSnapshot, MergeCheck types. Implement get_snapshot_filtered (recursive tree walk with ACL filtering), list_refs_filtered, and check_merge. Wire new types in lib.rs." \
  --type=task --priority=2
bd update bole-ghi --claim
git checkout -b bole-ghi
```

- [ ] **Step 2: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/repo/mod.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn filtered_snapshot_hides_protected_path() {
        use crate::acl::{Accessor, AclStore, PathAcl, PathRole, Permission};
        use crate::acl::memory::MemoryAclBackend;
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        let blob1 = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
        let blob2 = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        entries.insert("secrets/prod.key".into(), TreeEntry { id: blob2, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();

        // Empty accessor — cannot see secrets
        let empty = Accessor::new();
        let filtered = repo.get_snapshot_filtered(snap_id, &empty).await.unwrap().unwrap();
        assert!(filtered.visible_paths.contains_key("src/app.rs"));
        assert!(!filtered.visible_paths.contains_key("secrets/prod.key"));

        // Accessor with secrets read role — can see both
        let privileged = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
        let filtered2 = repo.get_snapshot_filtered(snap_id, &privileged).await.unwrap().unwrap();
        assert!(filtered2.visible_paths.contains_key("src/app.rs"));
        assert!(filtered2.visible_paths.contains_key("secrets/prod.key"));
    }

    #[test]
    fn list_refs_filtered_hides_protected_timeline() {
        use crate::acl::{Accessor, TimelineAcl, TimelineRole, Permission};
        use crate::refs::{RefName, TimelinePolicy};
        use crate::object::ObjectId;

        let repo = Repository::memory();
        repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();

        let id = ObjectId::new([1u8; 32]);
        repo.refs.create_tag(RefName::new("main").unwrap(), id, None, 1).unwrap();
        repo.refs.create_tag(RefName::new("leslie/private/exp").unwrap(), id, None, 2).unwrap();

        let empty = Accessor::new();
        let visible = repo.list_refs_filtered("", &empty).unwrap();
        let names: Vec<&str> = visible.iter().map(|n| n.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(!names.contains(&"leslie/private/exp"));

        let privileged = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "leslie/private/**".into(), permission: Permission::Read });
        let visible2 = repo.list_refs_filtered("", &privileged).unwrap();
        let names2: Vec<&str> = visible2.iter().map(|n| n.as_str()).collect();
        assert!(names2.contains(&"leslie/private/exp"));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test filtered 2>&1 | head -30
```

Expected: compile errors — `acls` field not on `Repository`, `get_snapshot_filtered` not found.

- [ ] **Step 4: Add `FilteredSnapshot` and `MergeCheck` types + update `Repository`**

Replace `src/repo/mod.rs` entirely with:

```rust
// bole-1vi
pub mod materialize;

use std::collections::BTreeMap;
use std::path::Path;
use crate::acl::disk::DiskAclBackend;
use crate::acl::memory::MemoryAclBackend;
use crate::acl::{Accessor, AclStore, PathAcl};
use crate::error::Result;
use crate::object::{EntryKind, Object, ObjectId};
use crate::refs::{DiskRefBackend, MemoryRefBackend, RefName, RefStore};
use crate::store::{disk::DiskBackend, memory::MemoryBackend, ObjectStore};

// <bead-id>
#[derive(Debug, Clone)]
pub struct FilteredSnapshot {
    pub id: ObjectId,
    pub author: String,
    pub created_at: u64,
    pub message: String,
    pub parents: Vec<ObjectId>,
    pub visible_paths: BTreeMap<String, ObjectId>,
}

// <bead-id>
#[derive(Debug, Clone, PartialEq)]
pub enum MergeCheck {
    Allowed,
    RequiresApproval(Vec<PathAcl>),
    Rejected(Vec<PathAcl>),
}

// bole-1vi
pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
    // <bead-id>
    pub acls: AclStore,
}

// bole-1vi
impl Repository {
    pub fn memory() -> Self {
        Self {
            objects: ObjectStore::new(MemoryBackend::new()),
            refs: RefStore::new(MemoryRefBackend::new()),
            // <bead-id>
            acls: AclStore::new(MemoryAclBackend::new()),
        }
    }

    pub async fn disk(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            objects: ObjectStore::new(DiskBackend::open(root).await?),
            refs: RefStore::new(DiskRefBackend::open(root)?),
            // <bead-id>
            acls: AclStore::new(DiskAclBackend::open(root)?),
        })
    }

    pub async fn copy_to(&self, dest: &Repository) -> Result<()> {
        copy_objects(&self.objects, &dest.objects).await?;
        copy_refs(&self.refs, &dest.refs)?;
        Ok(())
    }

    // <bead-id>
    pub async fn get_snapshot_filtered(
        &self,
        id: ObjectId,
        accessor: &Accessor,
    ) -> Result<Option<FilteredSnapshot>> {
        let snap = match self.objects.get(&id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Ok(None),
        };
        let mut visible_paths = BTreeMap::new();
        walk_tree_filtered(&self.objects, &self.acls, snap.root, "", accessor, &mut visible_paths).await?;
        Ok(Some(FilteredSnapshot {
            id,
            author: snap.author,
            created_at: snap.created_at,
            message: snap.message,
            parents: snap.parents,
            visible_paths,
        }))
    }

    // <bead-id>
    pub fn list_refs_filtered(&self, prefix: &str, accessor: &Accessor) -> Result<Vec<RefName>> {
        let all = self.refs.list(prefix)?;
        let mut out = Vec::new();
        for name in all {
            if self.acls.timeline_is_protected(name.as_str())? {
                if accessor.can_read_timeline(name.as_str()) {
                    out.push(name);
                }
            } else {
                out.push(name);
            }
        }
        Ok(out)
    }

    // <bead-id>
    pub async fn check_merge(
        &self,
        source: &RefName,
        dest: &RefName,
        accessor: &Accessor,
    ) -> Result<MergeCheck> {
        let source_head = match self.refs.get_timeline(source)? {
            Some(tl) => tl.head,
            None => return Ok(MergeCheck::Allowed),
        };
        let mut visible = BTreeMap::new();
        walk_tree_filtered(&self.objects, &self.acls, source_head, "", &Accessor::new(), &mut visible).await?;
        // Find all paths in source that are protected but dest doesn't enforce them
        let mut leaking: Vec<PathAcl> = Vec::new();
        let path_acls = self.acls.list_path_acls()?;
        for acl in &path_acls {
            let any_match = visible.keys().any(|p| crate::acl::glob::glob_matches(&acl.glob, p));
            if any_match && !self.acls.timeline_is_protected(dest.as_str())? {
                if !leaking.iter().any(|l| l.glob == acl.glob) {
                    leaking.push(acl.clone());
                }
            }
        }
        if leaking.is_empty() {
            Ok(MergeCheck::Allowed)
        } else if accessor.can_write_timeline(dest.as_str()) {
            Ok(MergeCheck::RequiresApproval(leaking))
        } else {
            Ok(MergeCheck::Rejected(leaking))
        }
    }
}

// <bead-id>
async fn walk_tree_filtered(
    objects: &ObjectStore,
    acls: &AclStore,
    tree_id: ObjectId,
    prefix: &str,
    accessor: &Accessor,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    let tree = match objects.get(&tree_id).await? {
        Some(Object::Tree(t)) => t,
        _ => return Ok(()),
    };
    for (name, entry) in &tree.entries {
        let full_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        match entry.kind {
            EntryKind::Blob => {
                if acls.path_is_protected(&full_path)? {
                    if accessor.can_read_path(&full_path) {
                        out.insert(full_path, entry.id);
                    }
                } else {
                    out.insert(full_path, entry.id);
                }
            }
            EntryKind::Tree => {
                Box::pin(walk_tree_filtered(objects, acls, entry.id, &full_path, accessor, out)).await?;
            }
        }
    }
    Ok(())
}

// bole-1vi
pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()> {
    for id in from.list().await? {
        if let Some(obj) = from.get(&id).await? {
            to.put(&obj).await?;
        }
    }
    Ok(())
}

// bole-1vi
pub fn copy_refs(from: &RefStore, to: &RefStore) -> Result<()> {
    for name in from.list("")? {
        if let Some(r) = from.get(&name)? {
            to.set_raw(&name, &r)?;
        }
    }
    Ok(())
}

// bole-1vi
#[cfg(test)]
mod tests {
    use super::{copy_objects, copy_refs, Repository};
    use crate::object::ObjectId;
    use crate::refs::{MemoryRefBackend, RefName, RefStore, TimelinePolicy};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use bytes::Bytes;
    use tempfile::TempDir;

    #[tokio::test]
    async fn memory_repo_has_working_stores() {
        let repo = Repository::memory();
        let id = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
        assert!(repo.objects.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn disk_repo_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = {
            let repo = Repository::disk(dir.path()).await.unwrap();
            repo.objects.put_blob(Bytes::from("persist")).await.unwrap()
        };
        let repo2 = Repository::disk(dir.path()).await.unwrap();
        assert!(repo2.objects.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn copy_objects_copies_all_five() {
        let from = ObjectStore::new(MemoryBackend::new());
        let to = ObjectStore::new(MemoryBackend::new());
        let ids = [
            from.put_blob(Bytes::from("a")).await.unwrap(),
            from.put_blob(Bytes::from("b")).await.unwrap(),
            from.put_blob(Bytes::from("c")).await.unwrap(),
            from.put_blob(Bytes::from("d")).await.unwrap(),
            from.put_blob(Bytes::from("e")).await.unwrap(),
        ];
        copy_objects(&from, &to).await.unwrap();
        for id in &ids {
            assert!(to.exists(id).await.unwrap(), "id {id} missing after copy");
        }
    }

    #[test]
    fn copy_refs_copies_tags_and_timelines() {
        let from = RefStore::new(MemoryRefBackend::new());
        let to = RefStore::new(MemoryRefBackend::new());
        let id = ObjectId::new([1u8; 32]);
        from.create_tag(RefName::new("v1").unwrap(), id, None, 1).unwrap();
        from.create_tag(RefName::new("v2").unwrap(), id, None, 2).unwrap();
        from.create_timeline(RefName::new("main").unwrap(), id, TimelinePolicy::Unrestricted, 3).unwrap();
        copy_refs(&from, &to).unwrap();
        assert!(to.get(&RefName::new("v1").unwrap()).unwrap().is_some());
        assert!(to.get(&RefName::new("v2").unwrap()).unwrap().is_some());
        assert!(to.get(&RefName::new("main").unwrap()).unwrap().is_some());
    }

    #[tokio::test]
    async fn copy_to_copies_objects_and_refs() {
        let dir = TempDir::new().unwrap();
        let src = Repository::memory();
        let id = src.objects.put_blob(Bytes::from("data")).await.unwrap();
        let tag_name = RefName::new("v1").unwrap();
        src.refs.create_tag(tag_name.clone(), id, None, 1).unwrap();
        let dest = Repository::disk(dir.path()).await.unwrap();
        src.copy_to(&dest).await.unwrap();
        assert!(dest.objects.exists(&id).await.unwrap());
        assert!(dest.refs.get_tag(&tag_name).unwrap().is_some());
    }

    // <bead-id>
    #[tokio::test]
    async fn filtered_snapshot_hides_protected_path() {
        use crate::acl::{Accessor, PathAcl, PathRole, Permission};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        let blob1 = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
        let blob2 = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        entries.insert("secrets/prod.key".into(), TreeEntry { id: blob2, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();

        let empty = Accessor::new();
        let filtered = repo.get_snapshot_filtered(snap_id, &empty).await.unwrap().unwrap();
        assert!(filtered.visible_paths.contains_key("src/app.rs"));
        assert!(!filtered.visible_paths.contains_key("secrets/prod.key"));

        let privileged = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
        let filtered2 = repo.get_snapshot_filtered(snap_id, &privileged).await.unwrap().unwrap();
        assert!(filtered2.visible_paths.contains_key("src/app.rs"));
        assert!(filtered2.visible_paths.contains_key("secrets/prod.key"));
    }

    #[test]
    fn list_refs_filtered_hides_protected_timeline() {
        use crate::acl::{Accessor, TimelineAcl, TimelineRole, Permission};
        use crate::object::ObjectId;

        let repo = Repository::memory();
        repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();

        let id = ObjectId::new([1u8; 32]);
        repo.refs.create_tag(RefName::new("main").unwrap(), id, None, 1).unwrap();
        repo.refs.create_tag(RefName::new("leslie/private/exp").unwrap(), id, None, 2).unwrap();

        let empty = Accessor::new();
        let visible = repo.list_refs_filtered("", &empty).unwrap();
        let names: Vec<&str> = visible.iter().map(|n| n.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(!names.contains(&"leslie/private/exp"));

        let privileged = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "leslie/private/**".into(), permission: Permission::Read });
        let visible2 = repo.list_refs_filtered("", &privileged).unwrap();
        let names2: Vec<&str> = visible2.iter().map(|n| n.as_str()).collect();
        assert!(names2.contains(&"leslie/private/exp"));
    }
}
```

- [ ] **Step 5: Update `src/lib.rs` re-exports**

Add `FilteredSnapshot` and `MergeCheck` to the repo re-exports in `src/lib.rs`:

```rust
// bole-1vi
pub use repo::{copy_objects, materialize::materialize, FilteredSnapshot, MergeCheck, Repository};
```

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -25
```

Expected: all existing tests plus the two new filtered tests pass. If `check_merge` causes compile errors from the `glob_matches` path import, use the full path `crate::acl::glob::glob_matches`.

- [ ] **Step 7: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean. Common warning: `dead_code` on `AccessDenied` variant — suppress with `#[allow(dead_code)]` on the variant if needed, or leave it (it's part of the public API).

- [ ] **Step 8: Commit**

```bash
git add src/repo/mod.rs src/lib.rs
git commit -m "feat(repo): add AclStore field, FilteredSnapshot, MergeCheck, filtered methods"
```

- [ ] **Step 9: Merge and close**

```bash
git checkout master && git merge bole-ghi
git branch -d bole-ghi
bd close bole-ghi
```

---

## Task 4: T3 Integration Tests

**Files:**
- Create: `tests/acl.rs`

**Interfaces:**
- Consumes all public APIs from Tasks 1–3:
  - `bole::{Accessor, AclStore, PathAcl, PathRole, Permission, TimelineAcl, TimelineRole}`
  - `bole::{FilteredSnapshot, MergeCheck, Repository}`
  - `bole::object::{EntryKind, ObjectId, Snapshot, TreeEntry}`
  - `bole::refs::{RefName, TimelinePolicy}`
  - `bytes::Bytes`
  - `tempfile::TempDir`
  - `std::collections::BTreeMap`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 3 T4: T3 integration tests" \
  --description="Create tests/acl.rs with three T3 spec tests: path filtering (3-path snapshot with ACL, filtered by empty/partial/full accessor), timeline filtering (list_refs_filtered hides protected timelines), merge_check (RequiresApproval and Rejected variants)." \
  --type=task --priority=2
bd update bole-jkl --claim
git checkout -b bole-jkl
```

- [ ] **Step 2: Create `tests/acl.rs`**

```rust
// <bead-id>
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, MergeCheck, PathAcl, PathRole, Permission, Repository, TimelineAcl, TimelineRole};
use bytes::Bytes;
use std::collections::BTreeMap;
use tempfile::TempDir;

/// T3: Snapshot path filtering.
/// Build a snapshot with 3 paths at different ACL levels.
/// Verify visibility depends on accessor's roles.
#[tokio::test]
async fn t3_path_filtering() {
    let repo = Repository::memory();

    // Protect two path namespaces
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();
    repo.acls.set_path_acl(PathAcl { glob: "notes/**".into() }).unwrap();

    // Build snapshot: one public path + two protected paths
    let blob_pub = repo.objects.put_blob(Bytes::from("public code")).await.unwrap();
    let blob_sec = repo.objects.put_blob(Bytes::from("s3cr3t")).await.unwrap();
    let blob_note = repo.objects.put_blob(Bytes::from("private note")).await.unwrap();

    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: blob_pub, kind: EntryKind::Blob });
    entries.insert("secrets/prod.key".into(), TreeEntry { id: blob_sec, kind: EntryKind::Blob });
    entries.insert("notes/private.md".into(), TreeEntry { id: blob_note, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();

    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 1, message: "init".into(),
    }).await.unwrap();

    // Empty accessor: only public path visible
    let empty = Accessor::new();
    let f = repo.get_snapshot_filtered(snap_id, &empty).await.unwrap().unwrap();
    assert_eq!(f.visible_paths.len(), 1);
    assert!(f.visible_paths.contains_key("src/app.rs"));
    assert!(!f.visible_paths.contains_key("secrets/prod.key"));
    assert!(!f.visible_paths.contains_key("notes/private.md"));

    // Accessor with secrets read: public + secrets visible
    let sec_only = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
    let f2 = repo.get_snapshot_filtered(snap_id, &sec_only).await.unwrap().unwrap();
    assert_eq!(f2.visible_paths.len(), 2);
    assert!(f2.visible_paths.contains_key("src/app.rs"));
    assert!(f2.visible_paths.contains_key("secrets/prod.key"));
    assert!(!f2.visible_paths.contains_key("notes/private.md"));

    // Accessor with both roles: all three paths visible
    let full = Accessor::new()
        .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read })
        .with_path_role(PathRole { glob: "notes/**".into(), permission: Permission::Read });
    let f3 = repo.get_snapshot_filtered(snap_id, &full).await.unwrap().unwrap();
    assert_eq!(f3.visible_paths.len(), 3);
    assert!(f3.visible_paths.contains_key("src/app.rs"));
    assert!(f3.visible_paths.contains_key("secrets/prod.key"));
    assert!(f3.visible_paths.contains_key("notes/private.md"));
}

/// T3: Timeline filtering.
/// Protected timelines are hidden from callers without the matching role.
#[tokio::test]
async fn t3_timeline_filtering() {
    let repo = Repository::memory();

    repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();

    let id = bole::object::ObjectId::new([1u8; 32]);
    repo.refs.create_tag(RefName::new("main").unwrap(), id, None, 1).unwrap();
    repo.refs.create_tag(RefName::new("leslie/private/exp-foo").unwrap(), id, None, 2).unwrap();

    // Empty accessor: private timeline hidden
    let empty = Accessor::new();
    let visible = repo.list_refs_filtered("", &empty).unwrap();
    let names: Vec<&str> = visible.iter().map(|n| n.as_str()).collect();
    assert!(names.contains(&"main"), "main should be visible");
    assert!(!names.contains(&"leslie/private/exp-foo"), "private timeline should be hidden");

    // Accessor with the matching timeline role: both visible
    let privileged = Accessor::new()
        .with_timeline_role(TimelineRole {
            pattern: "leslie/private/**".into(),
            permission: Permission::Read,
        });
    let visible2 = repo.list_refs_filtered("", &privileged).unwrap();
    let names2: Vec<&str> = visible2.iter().map(|n| n.as_str()).collect();
    assert!(names2.contains(&"main"));
    assert!(names2.contains(&"leslie/private/exp-foo"));
}

/// T3: Merge check.
/// Merging a timeline whose head contains protected paths into a public
/// timeline should be RequiresApproval (if caller has write) or Rejected.
#[tokio::test]
async fn t3_merge_check() {
    let repo = Repository::memory();

    // Protect secrets/** paths
    repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

    // Build a snapshot with a secret path
    let sec_blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
    let pub_blob = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("secrets/prod.key".into(), TreeEntry { id: sec_blob, kind: EntryKind::Blob });
    entries.insert("src/main.rs".into(), TreeEntry { id: pub_blob, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 1, message: "secret commit".into(),
    }).await.unwrap();

    // Create source timeline pointing at the secret-containing snapshot
    let source = RefName::new("feature/secret-work").unwrap();
    repo.refs.create_timeline(source.clone(), snap_id, TimelinePolicy::Unrestricted, 1).unwrap();

    // Create a public destination timeline
    let dest = RefName::new("main").unwrap();
    let pub_snap = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![], author: "test".into(),
        created_at: 2, message: "public".into(),
    }).await.unwrap();
    repo.refs.create_timeline(dest.clone(), pub_snap, TimelinePolicy::Unrestricted, 2).unwrap();

    // Accessor with write on dest: RequiresApproval
    let writer = Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write });
    let result = repo.check_merge(&source, &dest, &writer).await.unwrap();
    match result {
        MergeCheck::RequiresApproval(acls) => {
            assert!(!acls.is_empty(), "should report which paths would leak");
            assert!(acls.iter().any(|a| a.glob == "secrets/**"));
        }
        other => panic!("expected RequiresApproval, got {:?}", other),
    }

    // Accessor with no write on dest: Rejected
    let reader = Accessor::new();
    let result2 = repo.check_merge(&source, &dest, &reader).await.unwrap();
    match result2 {
        MergeCheck::Rejected(acls) => {
            assert!(!acls.is_empty());
        }
        other => panic!("expected Rejected, got {:?}", other),
    }

    // Clean source with no protected paths: Allowed
    let clean_blob = repo.objects.put_blob(Bytes::from("clean")).await.unwrap();
    let mut clean_entries = BTreeMap::new();
    clean_entries.insert("src/lib.rs".into(), TreeEntry { id: clean_blob, kind: EntryKind::Blob });
    let clean_tree = repo.objects.put_tree(clean_entries).await.unwrap();
    let clean_snap = repo.objects.put_snapshot(Snapshot {
        root: clean_tree, parents: vec![], author: "test".into(),
        created_at: 3, message: "clean".into(),
    }).await.unwrap();
    let clean_source = RefName::new("feature/clean").unwrap();
    repo.refs.create_timeline(clean_source.clone(), clean_snap, TimelinePolicy::Unrestricted, 3).unwrap();
    let result3 = repo.check_merge(&clean_source, &dest, &reader).await.unwrap();
    assert_eq!(result3, MergeCheck::Allowed);
}
```

- [ ] **Step 3: Run tests to verify they compile and pass**

```bash
cargo test --test acl 2>&1 | tail -20
```

Expected: all 3 T3 tests pass. If `check_merge` returns `Allowed` when `RequiresApproval` was expected, debug the `walk_tree_filtered` call inside `check_merge` — it uses `Accessor::new()` (empty) to collect all paths, so it should see the secret paths even without the role.

- [ ] **Step 4: Run the full suite**

```bash
cargo test 2>&1 | tail -15
```

Expected: all tests pass.

- [ ] **Step 5: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tests/acl.rs
git commit -m "test(acl): add T3 integration tests (path filtering, timeline filtering, merge check)"
```

- [ ] **Step 7: Merge and close**

```bash
git checkout master && git merge bole-jkl
git branch -d bole-jkl
bd close bole-jkl
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|-----------------|------|
| `glob_matches` helper | Task 1 |
| `Permission`, `PathRole`, `TimelineRole`, `Accessor` types | Task 1 |
| `PathAcl`, `TimelineAcl` types | Task 1 |
| `AclBackend` trait (8 methods) | Task 1 |
| `MemoryAclBackend` | Task 1 |
| `AclStore` with 6 public methods + `path_is_protected` + `timeline_is_protected` | Task 1 |
| `Error::AccessDenied` variant | Task 1 |
| `DiskAclBackend` with sanitized filenames, atomic writes, persist across reopen | Task 2 |
| `Repository.acls: AclStore` field | Task 3 |
| `Repository::memory()` includes `MemoryAclBackend` | Task 3 |
| `Repository::disk()` includes `DiskAclBackend` | Task 3 |
| `FilteredSnapshot` type | Task 3 |
| `MergeCheck` enum | Task 3 |
| `get_snapshot_filtered` recursive tree walk | Task 3 |
| `list_refs_filtered` timeline visibility filter | Task 3 |
| `check_merge` with `RequiresApproval` / `Rejected` / `Allowed` | Task 3 |
| T3 path filtering integration test | Task 4 |
| T3 timeline filtering integration test | Task 4 |
| T3 merge check integration test | Task 4 |

**Placeholder scan:** None found.

**Type consistency:**
- `PathAcl { pub glob: String }` — used consistently in Tasks 1, 3, 4 ✓
- `TimelineAcl { pub pattern: String }` — used consistently ✓
- `Accessor::can_read_path(&str) -> bool` — matches usage in `walk_tree_filtered` ✓
- `Accessor::can_write_timeline(&str) -> bool` — matches usage in `check_merge` ✓
- `MergeCheck::RequiresApproval(Vec<PathAcl>)` — matches T3 test assertions ✓
- `FilteredSnapshot { visible_paths: BTreeMap<String, ObjectId> }` — matches T3 test `contains_key` calls ✓
- `walk_tree_filtered` is an `async fn` using `Box::pin` for recursion — same pattern as `materialize` ✓
