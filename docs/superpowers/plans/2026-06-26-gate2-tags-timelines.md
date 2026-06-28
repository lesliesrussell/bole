# Gate 2: Tags and Timelines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a composable `RefStore` to the `bole` library crate — Tags (named mutable pointers to Snapshots) and Timelines (head pointer + policy) — backed by pluggable sync backends.

**Architecture:** `RefStore` wraps `Box<dyn RefBackend>` (sync trait). All refs are keyed by `RefName` (validated hierarchical path). `MemoryRefBackend` uses `Arc<RwLock<HashMap>>`. `DiskRefBackend` stores one postcard-encoded file per ref, mirroring the name hierarchy as a directory tree, using atomic write-then-rename. `RefStore` is fully independent of `ObjectStore` — callers compose them.

**Tech Stack:** Rust 2021 edition, serde + postcard (already in Cargo.toml), thiserror, std::sync::RwLock, std::fs (sync — no tokio needed for refs).

## Global Constraints

- Rust edition 2021, stable toolchain
- `RefBackend` is **sync** (not async) — refs are tiny, no async overhead needed
- No `anyhow` in library code — `thiserror` only
- No new crate dependencies — all required crates already in Cargo.toml
- `DiskRefBackend` uses atomic write-then-rename (learned from Gate 1 fix)
- `TimelinePolicy` stored but NOT enforced in Gate 2 — enforcement is Gate 6
- ZERO code written without a bead claimed first
- Branch name must match bead ID exactly
- Each contiguous block of code added for a bead gets a `// <bead-id>` comment
- Tests must pass before merge; delete branch after merge

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/error.rs` | Modify | Add `InvalidRefName` and `WrongRefKind` variants |
| `src/lib.rs` | Modify | Add `pub mod refs` + re-exports |
| `src/refs/mod.rs` | Create | `RefStore` struct + impl + re-exports |
| `src/refs/name.rs` | Create | `RefName` — validated hierarchical path |
| `src/refs/tag.rs` | Create | `Tag` struct |
| `src/refs/timeline.rs` | Create | `Timeline` + `TimelinePolicy` |
| `src/refs/ref_type.rs` | Create | `Ref` enum (`Tag` \| `Timeline`) |
| `src/refs/backend.rs` | Create | `RefBackend` sync trait |
| `src/refs/memory.rs` | Create | `MemoryRefBackend` |
| `src/refs/disk.rs` | Create | `DiskRefBackend` |
| `tests/refs.rs` | Create | Integration tests (spec T2) |

---

### Task 1: Scaffold + Error Variants

**Files:**
- Modify: `src/error.rs`
- Modify: `src/lib.rs`
- Create: `src/refs/mod.rs` (stub)
- Create: `src/refs/name.rs` (stub)
- Create: `src/refs/tag.rs` (stub)
- Create: `src/refs/timeline.rs` (stub)
- Create: `src/refs/ref_type.rs` (stub)
- Create: `src/refs/backend.rs` (stub)
- Create: `src/refs/memory.rs` (stub)
- Create: `src/refs/disk.rs` (stub)

**Interfaces:**
- Produces: `Error::InvalidRefName(String)`, `Error::WrongRefKind(String)` — used by Tasks 2 and 6

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: scaffold + error variants" --description="Add InvalidRefName and WrongRefKind to error.rs. Create src/refs/ module tree with empty stubs. Add pub mod refs to lib.rs. Foundation for all Gate 2 tasks." --type=task --priority=2
```

Note the returned ID (e.g. `bole-1`). Replace `bole-1` below with your actual ID.

```bash
bd update bole-1 --claim
git checkout -b bole-1
```

- [ ] **Step 2: Update src/error.rs**

```rust
// bole-49r
// bole-1
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("codec error: {0}")]
    Codec(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid ref name: {0}")]
    InvalidRefName(String),
    #[error("wrong ref kind: {0}")]
    WrongRefKind(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 3: Create stub files**

`src/refs/mod.rs`:
```rust
// bole-1
pub mod backend;
pub mod disk;
pub mod memory;
pub mod name;
pub mod ref_type;
pub mod tag;
pub mod timeline;

pub use backend::RefBackend;
pub use disk::DiskRefBackend;
pub use memory::MemoryRefBackend;
pub use name::RefName;
pub use ref_type::Ref;
pub use tag::Tag;
pub use timeline::{Timeline, TimelinePolicy};
```

`src/refs/name.rs`:
```rust
// bole-1
```

`src/refs/tag.rs`:
```rust
// bole-1
```

`src/refs/timeline.rs`:
```rust
// bole-1
```

`src/refs/ref_type.rs`:
```rust
// bole-1
```

`src/refs/backend.rs`:
```rust
// bole-1
```

`src/refs/memory.rs`:
```rust
// bole-1
```

`src/refs/disk.rs`:
```rust
// bole-1
```

- [ ] **Step 4: Add refs module to src/lib.rs**

```rust
// bole-49r
// bole-a7c
// bole-1
pub mod error;
pub mod object;
pub mod refs;
pub mod store;

pub(crate) mod codec;

pub use error::{Error, Result};
pub use object::{Blob, EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
pub use store::{
    backend::StorageBackend,
    disk::DiskBackend,
    memory::MemoryBackend,
    ObjectStore,
};
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check
```

Expected: compiles with no errors. Warnings about empty modules are fine.

- [ ] **Step 6: Commit and close**

```bash
git add src/error.rs src/lib.rs src/refs/
git commit -m "feat: scaffold refs module and add error variants"
git checkout master && git merge bole-1 && git branch -d bole-1
bd close bole-1
```

---

### Task 2: RefName

**Files:**
- Modify: `src/refs/name.rs`

**Interfaces:**
- Consumes: `Error::InvalidRefName` from Task 1
- Produces:
  - `RefName::new(s: impl Into<String>) -> Result<RefName>`
  - `RefName::as_str(&self) -> &str`
  - `RefName::prefix(&self) -> &str` — everything before the last `/`, or `""` if no `/`
  - `impl Display for RefName`
  - `#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: RefName" --description="Validated hierarchical ref name. Enforces: non-empty, no leading/trailing slash, no empty segments, no '..' segments. Used as the key for all tags and timelines." --type=task --priority=2
```

Note the returned ID (e.g. `bole-2`). Replace `bole-2` below with your actual ID.

```bash
bd update bole-2 --claim
git checkout -b bole-2
```

- [ ] **Step 2: Write failing tests**

Add to `src/refs/name.rs`:

```rust
// bole-2
#[cfg(test)]
mod tests {
    use super::RefName;

    #[test]
    fn valid_simple() {
        let n = RefName::new("v1").unwrap();
        assert_eq!(n.as_str(), "v1");
    }

    #[test]
    fn valid_hierarchical() {
        let n = RefName::new("experiment/foo").unwrap();
        assert_eq!(n.as_str(), "experiment/foo");
        assert_eq!(n.prefix(), "experiment");
    }

    #[test]
    fn prefix_no_slash() {
        let n = RefName::new("main").unwrap();
        assert_eq!(n.prefix(), "");
    }

    #[test]
    fn rejects_empty() {
        assert!(RefName::new("").is_err());
    }

    #[test]
    fn rejects_leading_slash() {
        assert!(RefName::new("/foo").is_err());
    }

    #[test]
    fn rejects_trailing_slash() {
        assert!(RefName::new("foo/").is_err());
    }

    #[test]
    fn rejects_consecutive_slashes() {
        assert!(RefName::new("a//b").is_err());
    }

    #[test]
    fn rejects_dotdot() {
        assert!(RefName::new("../etc/passwd").is_err());
    }

    #[test]
    fn display() {
        let n = RefName::new("leslie/exp-foo").unwrap();
        assert_eq!(n.to_string(), "leslie/exp-foo");
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test refs::name
```

Expected: compile error — `RefName` not defined.

- [ ] **Step 4: Implement RefName**

Replace `src/refs/name.rs` with:

```rust
// bole-2
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefName(String);

impl RefName {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        if s.is_empty() {
            return Err(Error::InvalidRefName("name must not be empty".into()));
        }
        if s.starts_with('/') {
            return Err(Error::InvalidRefName("name must not start with '/'".into()));
        }
        if s.ends_with('/') {
            return Err(Error::InvalidRefName("name must not end with '/'".into()));
        }
        for segment in s.split('/') {
            if segment.is_empty() {
                return Err(Error::InvalidRefName(
                    "consecutive slashes produce empty segment".into(),
                ));
            }
            if segment == ".." {
                return Err(Error::InvalidRefName("'..' segment not allowed".into()));
            }
            if segment.contains('\0') {
                return Err(Error::InvalidRefName("null byte in segment".into()));
            }
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn prefix(&self) -> &str {
        match self.0.rfind('/') {
            Some(i) => &self.0[..i],
            None => "",
        }
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::RefName;

    #[test]
    fn valid_simple() {
        let n = RefName::new("v1").unwrap();
        assert_eq!(n.as_str(), "v1");
    }

    #[test]
    fn valid_hierarchical() {
        let n = RefName::new("experiment/foo").unwrap();
        assert_eq!(n.as_str(), "experiment/foo");
        assert_eq!(n.prefix(), "experiment");
    }

    #[test]
    fn prefix_no_slash() {
        let n = RefName::new("main").unwrap();
        assert_eq!(n.prefix(), "");
    }

    #[test]
    fn rejects_empty() {
        assert!(RefName::new("").is_err());
    }

    #[test]
    fn rejects_leading_slash() {
        assert!(RefName::new("/foo").is_err());
    }

    #[test]
    fn rejects_trailing_slash() {
        assert!(RefName::new("foo/").is_err());
    }

    #[test]
    fn rejects_consecutive_slashes() {
        assert!(RefName::new("a//b").is_err());
    }

    #[test]
    fn rejects_dotdot() {
        assert!(RefName::new("../etc/passwd").is_err());
    }

    #[test]
    fn display() {
        let n = RefName::new("leslie/exp-foo").unwrap();
        assert_eq!(n.to_string(), "leslie/exp-foo");
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test refs::name
```

Expected: 9 tests pass.

- [ ] **Step 6: Commit and close**

```bash
git add src/refs/name.rs
git commit -m "feat: add RefName with hierarchical path validation"
git checkout master && git merge bole-2 && git branch -d bole-2
bd close bole-2
```

---

### Task 3: Tag, Timeline, Ref Types

**Files:**
- Modify: `src/refs/tag.rs`
- Modify: `src/refs/timeline.rs`
- Modify: `src/refs/ref_type.rs`

**Interfaces:**
- Consumes: `ObjectId` from Gate 1 (`crate::object::ObjectId`)
- Produces:
  - `Tag { target: ObjectId, created_at: u64, message: Option<String> }` with `Debug, Clone, PartialEq, Serialize, Deserialize`
  - `TimelinePolicy` enum: `FastForwardOnly | Append | Unrestricted` with same derives
  - `Timeline { head: ObjectId, policy: TimelinePolicy, created_at: u64 }` with same derives
  - `Ref` enum: `Tag(Tag) | Timeline(Timeline)` with same derives

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: Tag, Timeline, Ref types" --description="Tag, Timeline, TimelinePolicy, and Ref enum — all with serde derives for postcard serialization. These are the values stored in RefBackend." --type=task --priority=2
```

Note the returned ID (e.g. `bole-3`). Replace `bole-3` below with your actual ID.

```bash
bd update bole-3 --claim
git checkout -b bole-3
```

- [ ] **Step 2: Write failing serialization tests in src/refs/ref_type.rs**

```rust
// bole-3
#[cfg(test)]
mod tests {
    use super::Ref;
    use crate::refs::{Tag, Timeline, TimelinePolicy};
    use crate::object::ObjectId;

    #[test]
    fn tag_round_trip() {
        let id = ObjectId::new([1u8; 32]);
        let r = Ref::Tag(Tag { target: id, created_at: 1000, message: Some("v1".into()) });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    #[test]
    fn timeline_round_trip() {
        let id = ObjectId::new([2u8; 32]);
        let r = Ref::Timeline(Timeline {
            head: id,
            policy: TimelinePolicy::Append,
            created_at: 2000,
        });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test refs::ref_type
```

Expected: compile error — types not defined.

- [ ] **Step 4: Write src/refs/tag.rs**

```rust
// bole-3
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub target: ObjectId,
    pub created_at: u64,
    pub message: Option<String>,
}
```

- [ ] **Step 5: Write src/refs/timeline.rs**

```rust
// bole-3
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimelinePolicy {
    FastForwardOnly,
    Append,
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub head: ObjectId,
    pub policy: TimelinePolicy,
    pub created_at: u64,
}
```

- [ ] **Step 6: Write src/refs/ref_type.rs**

```rust
// bole-3
use crate::refs::{Tag, Timeline};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ref {
    Tag(Tag),
    Timeline(Timeline),
}

#[cfg(test)]
mod tests {
    use super::Ref;
    use crate::refs::{Tag, Timeline, TimelinePolicy};
    use crate::object::ObjectId;

    #[test]
    fn tag_round_trip() {
        let id = ObjectId::new([1u8; 32]);
        let r = Ref::Tag(Tag { target: id, created_at: 1000, message: Some("v1".into()) });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    #[test]
    fn timeline_round_trip() {
        let id = ObjectId::new([2u8; 32]);
        let r = Ref::Timeline(Timeline {
            head: id,
            policy: TimelinePolicy::Append,
            created_at: 2000,
        });
        let bytes = postcard::to_allocvec(&r).unwrap();
        let decoded: Ref = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(r, decoded);
    }
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test refs
```

Expected: 11 tests pass (9 RefName + 2 Ref round-trip).

- [ ] **Step 8: Commit and close**

```bash
git add src/refs/tag.rs src/refs/timeline.rs src/refs/ref_type.rs
git commit -m "feat: add Tag, Timeline, TimelinePolicy, and Ref types"
git checkout master && git merge bole-3 && git branch -d bole-3
bd close bole-3
```

---

### Task 4: RefBackend Trait + MemoryRefBackend

**Files:**
- Modify: `src/refs/backend.rs`
- Modify: `src/refs/memory.rs`

**Interfaces:**
- Consumes: `RefName` (Task 2), `Ref` (Task 3), `Result` from `crate::error`
- Produces:
  - `trait RefBackend: Send + Sync` with sync `get`, `set`, `delete`, `list`
  - `MemoryRefBackend::new() -> MemoryRefBackend`
  - `MemoryRefBackend: RefBackend + Clone`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: RefBackend trait + MemoryRefBackend" --description="Sync RefBackend trait (4 methods) and in-memory implementation using Arc<RwLock<HashMap>>. Same pattern as Gate 1 StorageBackend but sync." --type=task --priority=2
```

Note the returned ID (e.g. `bole-4`). Replace `bole-4` below with your actual ID.

```bash
bd update bole-4 --claim
git checkout -b bole-4
```

- [ ] **Step 2: Write failing tests in src/refs/memory.rs**

```rust
// bole-4
#[cfg(test)]
mod tests {
    use super::MemoryRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        let r = b.get(&name("v1")).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let b = MemoryRefBackend::new();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "leslie/a"));
        assert!(names.iter().any(|n| n.as_str() == "leslie/b"));
    }

    #[test]
    fn list_empty_prefix_returns_all() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("a"), &tag(id)).unwrap();
        b.set(&name("b/c"), &tag(id)).unwrap();
        assert_eq!(b.list("").unwrap().len(), 2);
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test refs::memory
```

Expected: compile error — `MemoryRefBackend` not defined.

- [ ] **Step 4: Write src/refs/backend.rs**

```rust
// bole-4
use crate::error::Result;
use crate::refs::{Ref, RefName};

pub trait RefBackend: Send + Sync {
    fn get(&self, name: &RefName) -> Result<Option<Ref>>;
    fn set(&self, name: &RefName, r: &Ref) -> Result<()>;
    fn delete(&self, name: &RefName) -> Result<()>;
    fn list(&self, prefix: &str) -> Result<Vec<RefName>>;
}
```

- [ ] **Step 5: Write src/refs/memory.rs**

```rust
// bole-4
use crate::error::Result;
use crate::refs::{backend::RefBackend, Ref, RefName};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct MemoryRefBackend {
    store: Arc<RwLock<HashMap<String, Ref>>>,
}

impl MemoryRefBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RefBackend for MemoryRefBackend {
    fn get(&self, name: &RefName) -> Result<Option<Ref>> {
        Ok(self.store.read().unwrap().get(name.as_str()).cloned())
    }

    fn set(&self, name: &RefName, r: &Ref) -> Result<()> {
        self.store
            .write()
            .unwrap()
            .insert(name.as_str().to_owned(), r.clone());
        Ok(())
    }

    fn delete(&self, name: &RefName) -> Result<()> {
        self.store.write().unwrap().remove(name.as_str());
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
        let store = self.store.read().unwrap();
        let mut names: Vec<RefName> = store
            .keys()
            .filter(|k| k.starts_with(prefix))
            .map(|k| RefName::new(k.as_str()).unwrap())
            .collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        let r = b.get(&name("v1")).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let b = MemoryRefBackend::new();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "leslie/a"));
        assert!(names.iter().any(|n| n.as_str() == "leslie/b"));
    }

    #[test]
    fn list_empty_prefix_returns_all() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("a"), &tag(id)).unwrap();
        b.set(&name("b/c"), &tag(id)).unwrap();
        assert_eq!(b.list("").unwrap().len(), 2);
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test refs::memory
```

Expected: 5 tests pass.

- [ ] **Step 7: Commit and close**

```bash
git add src/refs/backend.rs src/refs/memory.rs
git commit -m "feat: add RefBackend trait and MemoryRefBackend"
git checkout master && git merge bole-4 && git branch -d bole-4
bd close bole-4
```

---

### Task 5: DiskRefBackend

**Files:**
- Modify: `src/refs/disk.rs`

**Interfaces:**
- Consumes: `RefBackend` trait (Task 4), `Ref`, `RefName`, `Error::Codec`, `Error::Io`
- Produces:
  - `DiskRefBackend::open(root: impl AsRef<Path>) -> Result<DiskRefBackend>`
  - `DiskRefBackend: RefBackend`
  - File layout: `<root>/refs/<name-as-path>` (e.g. `experiment/foo` → `<root>/refs/experiment/foo`)
  - Postcard-encoded `Ref`, atomic write-then-rename, no compression

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: DiskRefBackend" --description="File-per-ref storage mirroring ref name as directory hierarchy. Postcard encoded. Atomic write-then-rename. No compression (refs are tiny). Same RefBackend contract as MemoryRefBackend." --type=task --priority=2
```

Note the returned ID (e.g. `bole-5`). Replace `bole-5` below with your actual ID.

```bash
bd update bole-5 --claim
git checkout -b bole-5
```

- [ ] **Step 2: Write failing tests in src/refs/disk.rs**

```rust
// bole-5
#[cfg(test)]
mod tests {
    use super::DiskRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;
    use tempfile::TempDir;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "leslie/a"));
        assert!(names.iter().any(|n| n.as_str() == "leslie/b"));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = ObjectId::new([1u8; 32]);
        {
            let b = DiskRefBackend::open(dir.path()).unwrap();
            b.set(&name("main"), &tag(id)).unwrap();
        }
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("main")).unwrap().is_some());
    }

    #[test]
    fn hierarchical_name_stored_in_subdirectory() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("experiment/foo"), &tag(id)).unwrap();
        // verify the file exists at the expected path
        assert!(dir.path().join("refs/experiment/foo").exists());
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test refs::disk
```

Expected: compile error — `DiskRefBackend` not defined.

- [ ] **Step 4: Write src/refs/disk.rs**

```rust
// bole-5
use crate::error::{Error, Result};
use crate::refs::{backend::RefBackend, Ref, RefName};
use std::fs;
use std::path::{Path, PathBuf};

pub struct DiskRefBackend {
    root: PathBuf,
}

impl DiskRefBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn ref_path(&self, name: &RefName) -> PathBuf {
        let mut path = self.root.join("refs");
        for segment in name.as_str().split('/') {
            path = path.join(segment);
        }
        path
    }

    fn walk_refs(&self, dir: &Path, root: &Path, prefix: &str, acc: &mut Vec<RefName>) -> Result<()> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.walk_refs(&path, root, prefix, acc)?;
            } else {
                if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                    continue;
                }
                let rel = path.strip_prefix(root).unwrap();
                let name_str: String = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if name_str.starts_with(prefix) {
                    if let Ok(ref_name) = RefName::new(name_str) {
                        acc.push(ref_name);
                    }
                }
            }
        }
        Ok(())
    }
}

impl RefBackend for DiskRefBackend {
    fn get(&self, name: &RefName) -> Result<Option<Ref>> {
        let path = self.ref_path(name);
        match fs::read(&path) {
            Ok(data) => {
                let r = postcard::from_bytes(&data).map_err(|e| Error::Codec(e.to_string()))?;
                Ok(Some(r))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set(&self, name: &RefName, r: &Ref) -> Result<()> {
        let path = self.ref_path(name);
        fs::create_dir_all(path.parent().expect("ref path always has a parent"))?;
        let data = postcard::to_allocvec(r).map_err(|e| Error::Codec(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn delete(&self, name: &RefName) -> Result<()> {
        match fs::remove_file(self.ref_path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
        let refs_root = self.root.join("refs");
        let mut names = Vec::new();
        self.walk_refs(&refs_root, &refs_root, prefix, &mut names)?;
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::DiskRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;
    use tempfile::TempDir;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "leslie/a"));
        assert!(names.iter().any(|n| n.as_str() == "leslie/b"));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = ObjectId::new([1u8; 32]);
        {
            let b = DiskRefBackend::open(dir.path()).unwrap();
            b.set(&name("main"), &tag(id)).unwrap();
        }
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("main")).unwrap().is_some());
    }

    #[test]
    fn hierarchical_name_stored_in_subdirectory() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("experiment/foo"), &tag(id)).unwrap();
        assert!(dir.path().join("refs/experiment/foo").exists());
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test refs::disk
```

Expected: 6 tests pass.

- [ ] **Step 6: Commit and close**

```bash
git add src/refs/disk.rs
git commit -m "feat: add DiskRefBackend with atomic writes and directory hierarchy"
git checkout master && git merge bole-5 && git branch -d bole-5
bd close bole-5
```

---

### Task 6: RefStore

**Files:**
- Modify: `src/refs/mod.rs`

**Interfaces:**
- Consumes: `RefBackend` (Task 4), `Ref`, `Tag`, `Timeline`, `TimelinePolicy`, `RefName` (Tasks 2–3), `ObjectId`, `Error::WrongRefKind`
- Produces:
  - `RefStore::new(backend: impl RefBackend + 'static) -> RefStore`
  - `RefStore::create_tag(&self, name: RefName, target: ObjectId, message: Option<String>, now: u64) -> Result<()>`
  - `RefStore::move_tag(&self, name: &RefName, target: ObjectId) -> Result<()>`
  - `RefStore::get_tag(&self, name: &RefName) -> Result<Option<Tag>>`
  - `RefStore::create_timeline(&self, name: RefName, head: ObjectId, policy: TimelinePolicy, now: u64) -> Result<()>`
  - `RefStore::advance_head(&self, name: &RefName, new_head: ObjectId) -> Result<()>`
  - `RefStore::get_timeline(&self, name: &RefName) -> Result<Option<Timeline>>`
  - `RefStore::get(&self, name: &RefName) -> Result<Option<Ref>>`
  - `RefStore::delete_ref(&self, name: &RefName) -> Result<()>`
  - `RefStore::list(&self, prefix: &str) -> Result<Vec<RefName>>`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: RefStore" --description="Public API layer wrapping Box<dyn RefBackend>. Type-safe methods for tags and timelines. move_tag and advance_head return WrongRefKind if called on the wrong ref type." --type=task --priority=2
```

Note the returned ID (e.g. `bole-6`). Replace `bole-6` below with your actual ID.

```bash
bd update bole-6 --claim
git checkout -b bole-6
```

- [ ] **Step 2: Write failing tests**

Add to `src/refs/mod.rs` (below the existing re-exports):

```rust
// bole-6
#[cfg(test)]
mod tests {
    use super::RefStore;
    use crate::refs::{MemoryRefBackend, RefName, TimelinePolicy};
    use crate::object::ObjectId;

    fn store() -> RefStore { RefStore::new(MemoryRefBackend::new()) }
    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }

    #[test]
    fn create_and_get_tag() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, Some("release".into()), 1000).unwrap();
        let tag = s.get_tag(&name("v1")).unwrap().unwrap();
        assert_eq!(tag.target, id);
        assert_eq!(tag.message.as_deref(), Some("release"));
    }

    #[test]
    fn move_tag_updates_target() {
        let s = store();
        let id1 = ObjectId::new([1u8; 32]);
        let id2 = ObjectId::new([2u8; 32]);
        s.create_tag(name("v1"), id1, None, 1).unwrap();
        s.move_tag(&name("v1"), id2).unwrap();
        assert_eq!(s.get_tag(&name("v1")).unwrap().unwrap().target, id2);
    }

    #[test]
    fn move_tag_on_timeline_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
        assert!(s.move_tag(&name("main"), id).is_err());
    }

    #[test]
    fn create_and_advance_timeline() {
        let s = store();
        let s1 = ObjectId::new([1u8; 32]);
        let s2 = ObjectId::new([2u8; 32]);
        let s3 = ObjectId::new([3u8; 32]);
        s.create_timeline(name("main"), s1, TimelinePolicy::Append, 1).unwrap();
        s.advance_head(&name("main"), s2).unwrap();
        s.advance_head(&name("main"), s3).unwrap();
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, s3);
    }

    #[test]
    fn advance_head_on_tag_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        assert!(s.advance_head(&name("v1"), id).is_err());
    }

    #[test]
    fn delete_ref_works_for_both_kinds() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        s.delete_ref(&name("v1")).unwrap();
        assert!(s.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("leslie/exp-a"), id, None, 1).unwrap();
        s.create_tag(name("leslie/exp-b"), id, None, 1).unwrap();
        s.create_tag(name("v1"), id, None, 1).unwrap();
        let listed = s.list("leslie/").unwrap();
        assert_eq!(listed.len(), 2);
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test refs::mod
```

Expected: compile error — `RefStore` not defined.

- [ ] **Step 4: Write RefStore in src/refs/mod.rs**

Replace `src/refs/mod.rs` with the complete file:

```rust
// bole-1
// bole-6
pub mod backend;
pub mod disk;
pub mod memory;
pub mod name;
pub mod ref_type;
pub mod tag;
pub mod timeline;

pub use backend::RefBackend;
pub use disk::DiskRefBackend;
pub use memory::MemoryRefBackend;
pub use name::RefName;
pub use ref_type::Ref;
pub use tag::Tag;
pub use timeline::{Timeline, TimelinePolicy};

use crate::error::{Error, Result};
use crate::object::ObjectId;

pub struct RefStore {
    backend: Box<dyn RefBackend>,
}

impl RefStore {
    pub fn new(backend: impl RefBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    pub fn create_tag(
        &self,
        name: RefName,
        target: ObjectId,
        message: Option<String>,
        now: u64,
    ) -> Result<()> {
        self.backend.set(&name, &Ref::Tag(Tag { target, created_at: now, message }))
    }

    pub fn move_tag(&self, name: &RefName, target: ObjectId) -> Result<()> {
        match self.backend.get(name)? {
            Some(Ref::Tag(mut tag)) => {
                tag.target = target;
                self.backend.set(name, &Ref::Tag(tag))
            }
            Some(Ref::Timeline(_)) => Err(Error::WrongRefKind(format!(
                "'{}' is a timeline, not a tag",
                name.as_str()
            ))),
            None => Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
        }
    }

    pub fn get_tag(&self, name: &RefName) -> Result<Option<Tag>> {
        match self.backend.get(name)? {
            Some(Ref::Tag(t)) => Ok(Some(t)),
            Some(Ref::Timeline(_)) => Err(Error::WrongRefKind(format!(
                "'{}' is a timeline, not a tag",
                name.as_str()
            ))),
            None => Ok(None),
        }
    }

    pub fn create_timeline(
        &self,
        name: RefName,
        head: ObjectId,
        policy: TimelinePolicy,
        now: u64,
    ) -> Result<()> {
        self.backend.set(&name, &Ref::Timeline(Timeline { head, policy, created_at: now }))
    }

    pub fn advance_head(&self, name: &RefName, new_head: ObjectId) -> Result<()> {
        match self.backend.get(name)? {
            Some(Ref::Timeline(mut tl)) => {
                tl.head = new_head;
                self.backend.set(name, &Ref::Timeline(tl))
            }
            Some(Ref::Tag(_)) => Err(Error::WrongRefKind(format!(
                "'{}' is a tag, not a timeline",
                name.as_str()
            ))),
            None => Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
        }
    }

    pub fn get_timeline(&self, name: &RefName) -> Result<Option<Timeline>> {
        match self.backend.get(name)? {
            Some(Ref::Timeline(t)) => Ok(Some(t)),
            Some(Ref::Tag(_)) => Err(Error::WrongRefKind(format!(
                "'{}' is a tag, not a timeline",
                name.as_str()
            ))),
            None => Ok(None),
        }
    }

    pub fn get(&self, name: &RefName) -> Result<Option<Ref>> {
        self.backend.get(name)
    }

    pub fn delete_ref(&self, name: &RefName) -> Result<()> {
        self.backend.delete(name)
    }

    pub fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
        self.backend.list(prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::RefStore;
    use crate::refs::{MemoryRefBackend, RefName, TimelinePolicy};
    use crate::object::ObjectId;

    fn store() -> RefStore { RefStore::new(MemoryRefBackend::new()) }
    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }

    #[test]
    fn create_and_get_tag() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, Some("release".into()), 1000).unwrap();
        let tag = s.get_tag(&name("v1")).unwrap().unwrap();
        assert_eq!(tag.target, id);
        assert_eq!(tag.message.as_deref(), Some("release"));
    }

    #[test]
    fn move_tag_updates_target() {
        let s = store();
        let id1 = ObjectId::new([1u8; 32]);
        let id2 = ObjectId::new([2u8; 32]);
        s.create_tag(name("v1"), id1, None, 1).unwrap();
        s.move_tag(&name("v1"), id2).unwrap();
        assert_eq!(s.get_tag(&name("v1")).unwrap().unwrap().target, id2);
    }

    #[test]
    fn move_tag_on_timeline_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
        assert!(s.move_tag(&name("main"), id).is_err());
    }

    #[test]
    fn create_and_advance_timeline() {
        let s = store();
        let s1 = ObjectId::new([1u8; 32]);
        let s2 = ObjectId::new([2u8; 32]);
        let s3 = ObjectId::new([3u8; 32]);
        s.create_timeline(name("main"), s1, TimelinePolicy::Append, 1).unwrap();
        s.advance_head(&name("main"), s2).unwrap();
        s.advance_head(&name("main"), s3).unwrap();
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, s3);
    }

    #[test]
    fn advance_head_on_tag_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        assert!(s.advance_head(&name("v1"), id).is_err());
    }

    #[test]
    fn delete_ref_works_for_both_kinds() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        s.delete_ref(&name("v1")).unwrap();
        assert!(s.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("leslie/exp-a"), id, None, 1).unwrap();
        s.create_tag(name("leslie/exp-b"), id, None, 1).unwrap();
        s.create_tag(name("v1"), id, None, 1).unwrap();
        let listed = s.list("leslie/").unwrap();
        assert_eq!(listed.len(), 2);
    }
}
```

- [ ] **Step 5: Run all tests**

```bash
cargo test refs
```

Expected: all refs tests pass (29 prior + new refs tests).

- [ ] **Step 6: Commit and close**

```bash
git add src/refs/mod.rs
git commit -m "feat: add RefStore with type-safe tag and timeline operations"
git checkout master && git merge bole-6 && git branch -d bole-6
bd close bole-6
```

---

### Task 7: Public API + Integration Tests

**Files:**
- Modify: `src/lib.rs`
- Create: `tests/refs.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–6
- Produces: clean public surface under `bole::*`, spec T2 tests pass

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 2: public API + integration tests" --description="Add refs re-exports to lib.rs and write spec T2 integration tests. Acceptance check for Gate 2 — all tests map to spec T2 requirements." --type=task --priority=2
```

Note the returned ID (e.g. `bole-7`). Replace `bole-7` below with your actual ID.

```bash
bd update bole-7 --claim
git checkout -b bole-7
```

- [ ] **Step 2: Update src/lib.rs**

```rust
// bole-49r
// bole-a7c
// bole-1
// bole-7
pub mod error;
pub mod object;
pub mod refs;
pub mod store;

pub(crate) mod codec;

pub use error::{Error, Result};
pub use object::{Blob, EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
pub use refs::{
    backend::RefBackend,
    disk::DiskRefBackend,
    memory::MemoryRefBackend,
    Ref, RefName, RefStore, Tag, Timeline, TimelinePolicy,
};
pub use store::{
    backend::StorageBackend,
    disk::DiskBackend,
    memory::MemoryBackend,
    ObjectStore,
};
```

- [ ] **Step 3: Write tests/refs.rs**

```rust
// bole-7
use bole::{DiskRefBackend, MemoryRefBackend, ObjectId, Ref, RefName, RefStore, TimelinePolicy};
use tempfile::TempDir;

fn name(s: &str) -> RefName { RefName::new(s).unwrap() }

fn run_t2_suite(store: RefStore) {
    let s1 = ObjectId::new([1u8; 32]);
    let s2 = ObjectId::new([2u8; 32]);
    let s3 = ObjectId::new([3u8; 32]);

    // T2: create tags v1 and experiment/foo
    store.create_tag(name("v1"), s1, None, 1000).unwrap();
    store.create_tag(name("experiment/foo"), s1, None, 1000).unwrap();

    // T2: move experiment/foo — pure reference update, v1 unchanged
    store.move_tag(&name("experiment/foo"), s2).unwrap();
    assert_eq!(store.get_tag(&name("v1")).unwrap().unwrap().target, s1);
    assert_eq!(store.get_tag(&name("experiment/foo")).unwrap().unwrap().target, s2);

    // T2: create main timeline and advance head S1→S2→S3
    store.create_timeline(name("main"), s1, TimelinePolicy::Append, 1000).unwrap();
    store.advance_head(&name("main"), s2).unwrap();
    store.advance_head(&name("main"), s3).unwrap();
    assert_eq!(store.get_timeline(&name("main")).unwrap().unwrap().head, s3);

    // T2: list by prefix
    let id = ObjectId::new([9u8; 32]);
    store.create_tag(name("leslie/exp-a"), id, None, 1).unwrap();
    store.create_tag(name("leslie/exp-b"), id, None, 1).unwrap();
    let listed = store.list("leslie/").unwrap();
    assert_eq!(listed.len(), 2);
}

#[test]
fn t2_memory_backend() {
    run_t2_suite(RefStore::new(MemoryRefBackend::new()));
}

#[test]
fn t2_disk_backend() {
    let dir = TempDir::new().unwrap();
    let backend = DiskRefBackend::open(dir.path()).unwrap();
    run_t2_suite(RefStore::new(backend));
}

#[test]
fn t2_wrong_kind_errors() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
    // move_tag on a timeline must fail
    assert!(store.move_tag(&name("main"), id).is_err());

    store.create_tag(name("v1"), id, None, 1).unwrap();
    // advance_head on a tag must fail
    assert!(store.advance_head(&name("v1"), id).is_err());
}

#[test]
fn t2_ref_name_validation() {
    assert!(RefName::new("").is_err());
    assert!(RefName::new("/leading").is_err());
    assert!(RefName::new("trailing/").is_err());
    assert!(RefName::new("a//b").is_err());
    assert!(RefName::new("../escape").is_err());
    assert!(RefName::new("valid/name").is_ok());
}

#[test]
fn t2_delete_ref() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_tag(name("v1"), id, None, 1).unwrap();
    store.delete_ref(&name("v1")).unwrap();
    assert!(store.get(&name("v1")).unwrap().is_none());
}

#[test]
fn t2_disk_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let id = ObjectId::new([1u8; 32]);
    {
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let store = RefStore::new(b);
        store.create_tag(name("v1"), id, Some("persisted".into()), 1).unwrap();
        store.create_timeline(name("main"), id, TimelinePolicy::Append, 1).unwrap();
    }
    let b = DiskRefBackend::open(dir.path()).unwrap();
    let store = RefStore::new(b);
    let tag = store.get_tag(&name("v1")).unwrap().unwrap();
    assert_eq!(tag.message.as_deref(), Some("persisted"));
    assert!(store.get_timeline(&name("main")).unwrap().is_some());
}

#[test]
fn t2_get_returns_correct_variant() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_tag(name("v1"), id, None, 1).unwrap();
    match store.get(&name("v1")).unwrap().unwrap() {
        Ref::Tag(t) => assert_eq!(t.target, id),
        Ref::Timeline(_) => panic!("expected tag"),
    }
}
```

- [ ] **Step 4: Run full test suite**

```bash
cargo test
```

Expected: all tests pass. Count includes 29 Gate 1 unit tests + all Gate 2 unit tests + 7 integration tests in `tests/refs.rs`.

- [ ] **Step 5: Verify no warnings**

```bash
cargo clippy -- -D warnings
```

Fix any warnings before committing.

- [ ] **Step 6: Commit and close**

```bash
git add src/lib.rs tests/refs.rs
git commit -m "feat: wire refs public API and add Gate 2 integration tests (spec T2)"
git checkout master && git merge bole-7 && git branch -d bole-7
bd close bole-7
```

---

## Self-Review

**Spec coverage:**
- `RefName` validated hierarchical path → Task 2 ✓
- `Tag` with serde → Task 3 ✓
- `Timeline` + `TimelinePolicy` with serde → Task 3 ✓
- `Ref` enum with serde → Task 3 ✓
- `RefBackend` sync trait → Task 4 ✓
- `MemoryRefBackend` → Task 4 ✓
- `DiskRefBackend` file-per-ref, atomic writes → Task 5 ✓
- `RefStore` public API → Task 6 ✓
- `Error::InvalidRefName`, `Error::WrongRefKind` → Task 1 ✓
- T2: tag create + move → `tests/refs.rs::run_t2_suite` ✓
- T2: timeline head advances → `tests/refs.rs::run_t2_suite` ✓
- T2: list by prefix → `tests/refs.rs::run_t2_suite` ✓
- T2: both backends run same suite → `t2_memory_backend`, `t2_disk_backend` ✓
- `TimelinePolicy` stored not enforced → no enforcement code anywhere ✓
- No new crate dependencies → all imports from existing Cargo.toml ✓
- Atomic write-then-rename in DiskRefBackend → Task 5 `set()` ✓

**Placeholder scan:** None found.

**Type consistency:** `RefName`, `Ref`, `Tag`, `Timeline`, `TimelinePolicy`, `RefBackend`, `MemoryRefBackend`, `DiskRefBackend`, `RefStore` used consistently across all tasks. `delete_ref` (not `delete_tag`) used consistently in Tasks 6 and 7.
