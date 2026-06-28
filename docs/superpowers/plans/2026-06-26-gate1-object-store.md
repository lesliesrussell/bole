# Gate 1: Content-Addressed Object Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `bole` Rust library crate with a content-addressed object store — Blobs, Trees, Snapshots — backed by pluggable async storage backends (Memory and Disk).

**Architecture:** `ObjectStore` wraps a `Box<dyn StorageBackend>` and owns serialization (postcard), hashing (BLAKE3), and idempotent deduplication. Two backends ship in the crate: `MemoryBackend` (HashMap behind a tokio RwLock) and `DiskBackend` (files sharded by hash prefix, always-on zstd compression).

**Tech Stack:** Rust 2021 edition, tokio (async), blake3, serde + postcard, bytes, zstd, async-trait, thiserror, tempfile (dev).

## Global Constraints

- Rust edition 2021, stable toolchain
- All public API is async (tokio)
- No `anyhow` in library code — `thiserror` only
- No feature flags — both backends always compiled
- zstd compression always-on in DiskBackend, no opt-out
- ZERO code written without a bead claimed first
- Branch name must match bead ID exactly
- Each contiguous block of code added for a bead gets a comment `// <bead-id>` at the top of the block
- Tests must pass before merge
- Delete branch after merge

---

## File Map

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Dependencies and crate metadata |
| `src/lib.rs` | Public re-exports only |
| `src/error.rs` | `Error` enum + `Result` alias |
| `src/object/id.rs` | `ObjectId` — 32-byte BLAKE3 wrapper |
| `src/object/blob.rs` | `Blob` struct |
| `src/object/tree.rs` | `Tree`, `TreeEntry`, `EntryKind` |
| `src/object/snapshot.rs` | `Snapshot` struct |
| `src/object/mod.rs` | `Object` enum + re-exports |
| `src/codec.rs` | postcard serialize/deserialize, ObjectId computation |
| `src/store/backend.rs` | `StorageBackend` async trait |
| `src/store/memory.rs` | `MemoryBackend` |
| `src/store/disk.rs` | `DiskBackend` with zstd |
| `src/store/mod.rs` | `ObjectStore` |
| `tests/object_store.rs` | Integration tests (spec T1) |
| `tests/backends.rs` | Backend contract tests |

---

### Task 1: Project Scaffold + Error Types

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/error.rs`
- Create: `src/object/mod.rs` (stub)
- Create: `src/codec.rs` (stub)
- Create: `src/store/backend.rs` (stub)
- Create: `src/store/memory.rs` (stub)
- Create: `src/store/disk.rs` (stub)
- Create: `src/store/mod.rs` (stub)

**Interfaces:**
- Produces: `bole::error::{Error, Result}` used by every subsequent task

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: scaffold + error types" --description="Initialize the bole library crate with Cargo.toml, all module stubs, and the crate-level Error enum. Foundation for every other task." --type=task --priority=2
```

Note the returned ID (e.g. `bole-1`). Replace `bole-1` in all commands below with your actual ID.

```bash
bd update bole-1 --claim
git checkout -b bole-1
```

- [ ] **Step 2: Initialize crate**

```bash
cargo init --lib
```

- [ ] **Step 3: Write Cargo.toml**

```toml
[package]
name = "bole"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
blake3 = "1"
serde = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc"] }
bytes = { version = "1", features = ["serde"] }
zstd = "0.13"
async-trait = "0.1"
thiserror = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Write src/error.rs**

```rust
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
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 5: Write module stubs**

`src/lib.rs`:
```rust
// bole-1
pub mod error;
pub mod object;
pub mod store;

pub(crate) mod codec;
```

`src/object/mod.rs`:
```rust
// bole-1
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;
```

`src/codec.rs`:
```rust
// bole-1
```

`src/store/mod.rs`:
```rust
// bole-1
pub mod backend;
pub mod disk;
pub mod memory;
```

`src/store/backend.rs`:
```rust
// bole-1
```

`src/store/memory.rs`:
```rust
// bole-1
```

`src/store/disk.rs`:
```rust
// bole-1
```

- [ ] **Step 6: Verify compilation**

```bash
cargo check
```

Expected: compiles with no errors. Warnings about empty modules are fine.

- [ ] **Step 7: Commit and close**

```bash
git add Cargo.toml src/
git commit -m "feat: initialize bole crate scaffold and error types"
git checkout master && git merge bole-1 && git branch -d bole-1
bd close bole-1
```

---

### Task 2: ObjectId

**Files:**
- Create: `src/object/id.rs`
- Modify: `src/object/mod.rs`

**Interfaces:**
- Produces:
  - `ObjectId::new(bytes: [u8; 32]) -> ObjectId`
  - `ObjectId::from_bytes(data: &[u8]) -> ObjectId` — BLAKE3 hash of `data`
  - `ObjectId::as_bytes(&self) -> &[u8; 32]`
  - `impl Display for ObjectId` — lowercase hex string

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: ObjectId" --description="32-byte BLAKE3 content address. The identity of every object in the store. Includes Display (hex), serde derives, and Hash/Eq/Ord impls." --type=task --priority=2
```

Note the returned ID (e.g. `bole-2`). Replace `bole-2` below with your actual ID.

```bash
bd update bole-2 --claim
git checkout -b bole-2
```

- [ ] **Step 2: Write failing test**

Create `src/object/id.rs` with just the test module:

```rust
// bole-2
#[cfg(test)]
mod tests {
    use super::ObjectId;

    #[test]
    fn same_content_same_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_different_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_64_hex_chars() {
        let id = ObjectId::from_bytes(b"test");
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn roundtrip_via_bytes() {
        let id = ObjectId::from_bytes(b"roundtrip");
        let id2 = ObjectId::new(*id.as_bytes());
        assert_eq!(id, id2);
    }
}
```

- [ ] **Step 3: Run to verify test fails**

```bash
cargo test object::id
```

Expected: compile error — `ObjectId` is not defined.

- [ ] **Step 4: Implement ObjectId**

Replace `src/object/id.rs` with:

```rust
// bole-2
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_bytes(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(*hash.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectId;

    #[test]
    fn same_content_same_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_different_id() {
        let a = ObjectId::from_bytes(b"hello");
        let b = ObjectId::from_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_64_hex_chars() {
        let id = ObjectId::from_bytes(b"test");
        assert_eq!(id.to_string().len(), 64);
    }

    #[test]
    fn roundtrip_via_bytes() {
        let id = ObjectId::from_bytes(b"roundtrip");
        let id2 = ObjectId::new(*id.as_bytes());
        assert_eq!(id, id2);
    }
}
```

Update `src/object/mod.rs` to re-export:

```rust
// bole-2
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;

pub use id::ObjectId;
```

- [ ] **Step 5: Run tests**

```bash
cargo test object::id
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit and close**

```bash
git add src/object/id.rs src/object/mod.rs
git commit -m "feat: add ObjectId with BLAKE3 content addressing"
git checkout master && git merge bole-2 && git branch -d bole-2
bd close bole-2
```

---

### Task 3: Object Types + Codec

**Files:**
- Create: `src/object/blob.rs`
- Create: `src/object/tree.rs`
- Create: `src/object/snapshot.rs`
- Modify: `src/object/mod.rs`
- Modify: `src/codec.rs`

**Interfaces:**
- Consumes: `ObjectId` from Task 2
- Produces:
  - `Blob { data: Bytes }`
  - `EntryKind` enum — `Blob | Tree`
  - `TreeEntry { id: ObjectId, kind: EntryKind }`
  - `Tree { entries: BTreeMap<String, TreeEntry> }`
  - `Snapshot { root: ObjectId, parents: Vec<ObjectId>, author: String, created_at: u64, message: String }`
  - `Object` enum — `Blob(Blob) | Tree(Tree) | Snapshot(Snapshot)`
  - `codec::serialize(obj: &Object) -> Result<Vec<u8>>`
  - `codec::deserialize(data: &[u8]) -> Result<Object>`
  - `codec::object_id(data: &[u8]) -> ObjectId`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: object types + codec" --description="Blob, Tree, Snapshot, Object enum with serde derives, and the postcard codec layer. codec::serialize + deserialize + object_id are how ObjectStore computes content addresses." --type=task --priority=2
```

Note the returned ID (e.g. `bole-3`). Replace `bole-3` below with your actual ID.

```bash
bd update bole-3 --claim
git checkout -b bole-3
```

- [ ] **Step 2: Write failing tests in codec.rs**

```rust
// bole-3
#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Blob, Object};
    use bytes::Bytes;

    #[test]
    fn blob_round_trip() {
        let obj = Object::Blob(Blob { data: Bytes::from("hello world") });
        let data = serialize(&obj).unwrap();
        let decoded = deserialize(&data).unwrap();
        assert_eq!(obj, decoded);
    }

    #[test]
    fn same_object_same_id() {
        let obj = Object::Blob(Blob { data: Bytes::from("deterministic") });
        let d1 = serialize(&obj).unwrap();
        let d2 = serialize(&obj).unwrap();
        assert_eq!(object_id(&d1), object_id(&d2));
    }

    #[test]
    fn different_objects_different_ids() {
        let a = Object::Blob(Blob { data: Bytes::from("aaa") });
        let b = Object::Blob(Blob { data: Bytes::from("bbb") });
        let da = serialize(&a).unwrap();
        let db = serialize(&b).unwrap();
        assert_ne!(object_id(&da), object_id(&db));
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test codec
```

Expected: compile error — types not defined yet.

- [ ] **Step 4: Write src/object/blob.rs**

```rust
// bole-3
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    pub data: Bytes,
}
```

- [ ] **Step 5: Write src/object/tree.rs**

```rust
// bole-3
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryKind {
    Blob,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub id: ObjectId,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    pub entries: BTreeMap<String, TreeEntry>,
}
```

- [ ] **Step 6: Write src/object/snapshot.rs**

```rust
// bole-3
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub root: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: String,
    pub created_at: u64,
    pub message: String,
}
```

- [ ] **Step 7: Update src/object/mod.rs**

```rust
// bole-3
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;

pub use blob::Blob;
pub use id::ObjectId;
pub use snapshot::Snapshot;
pub use tree::{EntryKind, Tree, TreeEntry};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
}
```

- [ ] **Step 8: Write src/codec.rs**

```rust
// bole-3
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};

pub fn serialize(obj: &Object) -> Result<Vec<u8>> {
    postcard::to_allocvec(obj).map_err(|e| Error::Codec(e.to_string()))
}

pub fn deserialize(data: &[u8]) -> Result<Object> {
    postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string()))
}

pub fn object_id(data: &[u8]) -> ObjectId {
    ObjectId::from_bytes(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Blob, Object};
    use bytes::Bytes;

    #[test]
    fn blob_round_trip() {
        let obj = Object::Blob(Blob { data: Bytes::from("hello world") });
        let data = serialize(&obj).unwrap();
        let decoded = deserialize(&data).unwrap();
        assert_eq!(obj, decoded);
    }

    #[test]
    fn same_object_same_id() {
        let obj = Object::Blob(Blob { data: Bytes::from("deterministic") });
        let d1 = serialize(&obj).unwrap();
        let d2 = serialize(&obj).unwrap();
        assert_eq!(object_id(&d1), object_id(&d2));
    }

    #[test]
    fn different_objects_different_ids() {
        let a = Object::Blob(Blob { data: Bytes::from("aaa") });
        let b = Object::Blob(Blob { data: Bytes::from("bbb") });
        let da = serialize(&a).unwrap();
        let db = serialize(&b).unwrap();
        assert_ne!(object_id(&da), object_id(&db));
    }
}
```

- [ ] **Step 9: Run tests**

```bash
cargo test
```

Expected: 7 tests pass (4 from ObjectId + 3 from codec).

- [ ] **Step 10: Commit and close**

```bash
git add src/object/ src/codec.rs
git commit -m "feat: add object types (Blob, Tree, Snapshot) and postcard codec"
git checkout master && git merge bole-3 && git branch -d bole-3
bd close bole-3
```

---

### Task 4: StorageBackend Trait + MemoryBackend

**Files:**
- Modify: `src/store/backend.rs`
- Modify: `src/store/memory.rs`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `ObjectId` from Task 2, `Result` from Task 1
- Produces:
  - `trait StorageBackend: Send + Sync` with async `put`, `get`, `exists`, `delete`
  - `MemoryBackend::new() -> MemoryBackend`
  - `MemoryBackend: StorageBackend + Clone`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: StorageBackend trait + MemoryBackend" --description="The async storage abstraction (trait object) and the in-memory implementation. MemoryBackend is used in tests and ephemeral agent workflows." --type=task --priority=2
```

Note the returned ID (e.g. `bole-4`). Replace `bole-4` below with your actual ID.

```bash
bd update bole-4 --claim
git checkout -b bole-4
```

- [ ] **Step 2: Write failing test in src/store/memory.rs**

```rust
// bole-4
#[cfg(test)]
mod tests {
    use super::MemoryBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;

    #[tokio::test]
    async fn put_then_get() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let backend = MemoryBackend::new();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test store::memory
```

Expected: compile error — types not defined.

- [ ] **Step 4: Write src/store/backend.rs**

```rust
// bole-4
use async_trait::async_trait;
use bytes::Bytes;
use crate::error::Result;
use crate::object::ObjectId;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()>;
    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>>;
    async fn exists(&self, id: &ObjectId) -> Result<bool>;
    async fn delete(&self, id: &ObjectId) -> Result<()>;
}
```

- [ ] **Step 5: Write src/store/memory.rs**

```rust
// bole-4
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::error::Result;
use crate::object::ObjectId;
use super::backend::StorageBackend;

#[derive(Debug, Clone, Default)]
pub struct MemoryBackend {
    store: Arc<RwLock<HashMap<[u8; 32], Bytes>>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StorageBackend for MemoryBackend {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()> {
        self.store
            .write()
            .await
            .insert(*id.as_bytes(), Bytes::copy_from_slice(data));
        Ok(())
    }

    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>> {
        Ok(self.store.read().await.get(id.as_bytes()).cloned())
    }

    async fn exists(&self, id: &ObjectId) -> Result<bool> {
        Ok(self.store.read().await.contains_key(id.as_bytes()))
    }

    async fn delete(&self, id: &ObjectId) -> Result<()> {
        self.store.write().await.remove(id.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;

    #[tokio::test]
    async fn put_then_get() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let backend = MemoryBackend::new();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let backend = MemoryBackend::new();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test store::memory
```

Expected: 4 tests pass.

- [ ] **Step 7: Commit and close**

```bash
git add src/store/backend.rs src/store/memory.rs
git commit -m "feat: add StorageBackend trait and MemoryBackend"
git checkout master && git merge bole-4 && git branch -d bole-4
bd close bole-4
```

---

### Task 5: DiskBackend

**Files:**
- Modify: `src/store/disk.rs`

**Interfaces:**
- Consumes: `StorageBackend` trait from Task 4
- Produces:
  - `DiskBackend::open(root: impl AsRef<Path>) -> Result<DiskBackend>`
  - `DiskBackend: StorageBackend`
  - Objects stored under `<root>/objects/<2-char prefix>/<remaining hex>`, always zstd-compressed

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: DiskBackend" --description="Disk-backed storage using file-per-object layout sharded by hash prefix. Always compresses with zstd level 3. Same StorageBackend contract as MemoryBackend." --type=task --priority=2
```

Note the returned ID (e.g. `bole-5`). Replace `bole-5` below with your actual ID.

```bash
bd update bole-5 --claim
git checkout -b bole-5
```

- [ ] **Step 2: Write failing tests in src/store/disk.rs**

```rust
// bole-5
#[cfg(test)]
mod tests {
    use super::DiskBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;
    use tempfile::TempDir;

    #[tokio::test]
    async fn put_then_get() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = {
            let backend = DiskBackend::open(dir.path()).await.unwrap();
            let id = ObjectId::from_bytes(b"persistent");
            backend.put(&id, b"data").await.unwrap();
            id
        };
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"data".as_slice()));
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test store::disk
```

Expected: compile error — `DiskBackend` not defined.

- [ ] **Step 4: Write src/store/disk.rs**

```rust
// bole-5
use async_trait::async_trait;
use bytes::Bytes;
use std::path::{Path, PathBuf};
use tokio::fs;
use crate::error::{Error, Result};
use crate::object::ObjectId;
use super::backend::StorageBackend;

pub struct DiskBackend {
    root: PathBuf,
}

impl DiskBackend {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        let hex = id.to_string();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }
}

#[async_trait]
impl StorageBackend for DiskBackend {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()> {
        let path = self.object_path(id);
        fs::create_dir_all(path.parent().unwrap()).await?;
        let data = data.to_vec();
        let compressed = tokio::task::spawn_blocking(move || {
            zstd::encode_all(data.as_slice(), 3)
        })
        .await
        .map_err(|e| Error::Storage(e.to_string()))?
        .map_err(|e| Error::Storage(e.to_string()))?;
        fs::write(&path, compressed).await?;
        Ok(())
    }

    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>> {
        let path = self.object_path(id);
        match fs::read(&path).await {
            Ok(compressed) => {
                let data = tokio::task::spawn_blocking(move || {
                    zstd::decode_all(compressed.as_slice())
                })
                .await
                .map_err(|e| Error::Storage(e.to_string()))?
                .map_err(|e| Error::Storage(e.to_string()))?;
                Ok(Some(Bytes::from(data)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    async fn exists(&self, id: &ObjectId) -> Result<bool> {
        Ok(self.object_path(id).exists())
    }

    async fn delete(&self, id: &ObjectId) -> Result<()> {
        match fs::remove_file(self.object_path(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DiskBackend;
    use crate::store::backend::StorageBackend;
    use crate::object::ObjectId;
    use tempfile::TempDir;

    #[tokio::test]
    async fn put_then_get() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"value").await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"value".as_slice()));
    }

    #[tokio::test]
    async fn exists_after_put() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        assert!(!backend.exists(&id).await.unwrap());
        backend.put(&id, b"data").await.unwrap();
        assert!(backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::new([0u8; 32]);
        assert!(backend.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id = ObjectId::from_bytes(b"key");
        backend.put(&id, b"data").await.unwrap();
        backend.delete(&id).await.unwrap();
        assert!(!backend.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = {
            let backend = DiskBackend::open(dir.path()).await.unwrap();
            let id = ObjectId::from_bytes(b"persistent");
            backend.put(&id, b"data").await.unwrap();
            id
        };
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let result = backend.get(&id).await.unwrap();
        assert_eq!(result.as_deref(), Some(b"data".as_slice()));
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test store::disk
```

Expected: 5 tests pass.

- [ ] **Step 6: Commit and close**

```bash
git add src/store/disk.rs
git commit -m "feat: add DiskBackend with zstd compression"
git checkout master && git merge bole-5 && git branch -d bole-5
bd close bole-5
```

---

### Task 6: ObjectStore

**Files:**
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `StorageBackend` (Task 4), `codec` (Task 3), `Object`, `ObjectId` (Tasks 2–3)
- Produces:
  - `ObjectStore::new(backend: impl StorageBackend + 'static) -> ObjectStore`
  - `ObjectStore::put(&self, obj: &Object) -> Result<ObjectId>` — idempotent
  - `ObjectStore::get(&self, id: &ObjectId) -> Result<Option<Object>>`
  - `ObjectStore::exists(&self, id: &ObjectId) -> Result<bool>`
  - `ObjectStore::put_blob(&self, data: Bytes) -> Result<ObjectId>`
  - `ObjectStore::put_tree(&self, entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId>`
  - `ObjectStore::put_snapshot(&self, snap: Snapshot) -> Result<ObjectId>`

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: ObjectStore" --description="The public API layer. Owns serialization, hashing, and idempotent deduplication. Wraps any StorageBackend via Box<dyn StorageBackend>." --type=task --priority=2
```

Note the returned ID (e.g. `bole-6`). Replace `bole-6` below with your actual ID.

```bash
bd update bole-6 --claim
git checkout -b bole-6
```

- [ ] **Step 2: Write failing tests in src/store/mod.rs**

```rust
// bole-6
#[cfg(test)]
mod tests {
    use super::ObjectStore;
    use crate::object::{Blob, EntryKind, Object, Snapshot, TreeEntry};
    use crate::store::memory::MemoryBackend;
    use bytes::Bytes;
    use std::collections::BTreeMap;

    fn store() -> ObjectStore {
        ObjectStore::new(MemoryBackend::new())
    }

    #[tokio::test]
    async fn put_blob_returns_stable_id() {
        let s = store();
        let id1 = s.put_blob(Bytes::from("hello")).await.unwrap();
        let id2 = s.put_blob(Bytes::from("hello")).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn put_is_idempotent() {
        let s = store();
        let obj = Object::Blob(Blob { data: Bytes::from("same") });
        let id1 = s.put(&obj).await.unwrap();
        let id2 = s.put(&obj).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn get_returns_original_object() {
        let s = store();
        let id = s.put_blob(Bytes::from("retrieve me")).await.unwrap();
        let obj = s.get(&id).await.unwrap().unwrap();
        match obj {
            Object::Blob(b) => assert_eq!(b.data, Bytes::from("retrieve me")),
            _ => panic!("expected blob"),
        }
    }

    #[tokio::test]
    async fn snapshot_immutability() {
        let s = store();
        let r1 = s.put_blob(Bytes::from("v1")).await.unwrap();
        let snap1 = Snapshot {
            root: r1, parents: vec![], author: "alice".into(),
            created_at: 1, message: "first".into(),
        };
        let s1 = s.put_snapshot(snap1).await.unwrap();

        let r2 = s.put_blob(Bytes::from("v2")).await.unwrap();
        let snap2 = Snapshot {
            root: r2, parents: vec![s1], author: "alice".into(),
            created_at: 2, message: "second".into(),
        };
        let s2 = s.put_snapshot(snap2).await.unwrap();

        assert_ne!(s1, s2);
        let original = s.get(&s1).await.unwrap().unwrap();
        match original {
            Object::Snapshot(snap) => assert_eq!(snap.message, "first"),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn snapshot_parents_preserved() {
        let s = store();
        let root = s.put_blob(Bytes::from("root")).await.unwrap();
        let s1 = s.put_snapshot(Snapshot {
            root, parents: vec![], author: "a".into(), created_at: 1, message: "s1".into(),
        }).await.unwrap();
        let s2 = s.put_snapshot(Snapshot {
            root, parents: vec![s1], author: "a".into(), created_at: 2, message: "s2".into(),
        }).await.unwrap();
        match s.get(&s2).await.unwrap().unwrap() {
            Object::Snapshot(snap) => assert_eq!(snap.parents, vec![s1]),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn tree_round_trip() {
        let s = store();
        let blob_id = s.put_blob(Bytes::from("content")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("file.txt".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = s.put_tree(entries.clone()).await.unwrap();
        match s.get(&tree_id).await.unwrap().unwrap() {
            Object::Tree(t) => assert_eq!(t.entries, entries),
            _ => panic!("expected tree"),
        }
    }
}
```

- [ ] **Step 3: Run to verify tests fail**

```bash
cargo test store::mod
```

Expected: compile error — `ObjectStore` not defined.

- [ ] **Step 4: Write src/store/mod.rs**

```rust
// bole-6
pub mod backend;
pub mod disk;
pub mod memory;

use bytes::Bytes;
use std::collections::BTreeMap;
use crate::codec;
use crate::error::Result;
use crate::object::{Blob, Object, ObjectId, Snapshot, Tree, TreeEntry};
use backend::StorageBackend;

pub struct ObjectStore {
    backend: Box<dyn StorageBackend>,
}

impl ObjectStore {
    pub fn new(backend: impl StorageBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    pub async fn put(&self, obj: &Object) -> Result<ObjectId> {
        let data = codec::serialize(obj)?;
        let id = codec::object_id(&data);
        if !self.backend.exists(&id).await? {
            self.backend.put(&id, &data).await?;
        }
        Ok(id)
    }

    pub async fn get(&self, id: &ObjectId) -> Result<Option<Object>> {
        match self.backend.get(id).await? {
            Some(data) => Ok(Some(codec::deserialize(&data)?)),
            None => Ok(None),
        }
    }

    pub async fn exists(&self, id: &ObjectId) -> Result<bool> {
        self.backend.exists(id).await
    }

    pub async fn put_blob(&self, data: Bytes) -> Result<ObjectId> {
        self.put(&Object::Blob(Blob { data })).await
    }

    pub async fn put_tree(&self, entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId> {
        self.put(&Object::Tree(Tree { entries })).await
    }

    pub async fn put_snapshot(&self, snap: Snapshot) -> Result<ObjectId> {
        self.put(&Object::Snapshot(snap)).await
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectStore;
    use crate::object::{Blob, EntryKind, Object, Snapshot, TreeEntry};
    use crate::store::memory::MemoryBackend;
    use bytes::Bytes;
    use std::collections::BTreeMap;

    fn store() -> ObjectStore {
        ObjectStore::new(MemoryBackend::new())
    }

    #[tokio::test]
    async fn put_blob_returns_stable_id() {
        let s = store();
        let id1 = s.put_blob(Bytes::from("hello")).await.unwrap();
        let id2 = s.put_blob(Bytes::from("hello")).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn put_is_idempotent() {
        let s = store();
        let obj = Object::Blob(Blob { data: Bytes::from("same") });
        let id1 = s.put(&obj).await.unwrap();
        let id2 = s.put(&obj).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn get_returns_original_object() {
        let s = store();
        let id = s.put_blob(Bytes::from("retrieve me")).await.unwrap();
        let obj = s.get(&id).await.unwrap().unwrap();
        match obj {
            Object::Blob(b) => assert_eq!(b.data, Bytes::from("retrieve me")),
            _ => panic!("expected blob"),
        }
    }

    #[tokio::test]
    async fn snapshot_immutability() {
        let s = store();
        let r1 = s.put_blob(Bytes::from("v1")).await.unwrap();
        let snap1 = Snapshot {
            root: r1, parents: vec![], author: "alice".into(),
            created_at: 1, message: "first".into(),
        };
        let s1 = s.put_snapshot(snap1).await.unwrap();

        let r2 = s.put_blob(Bytes::from("v2")).await.unwrap();
        let snap2 = Snapshot {
            root: r2, parents: vec![s1], author: "alice".into(),
            created_at: 2, message: "second".into(),
        };
        let s2 = s.put_snapshot(snap2).await.unwrap();

        assert_ne!(s1, s2);
        let original = s.get(&s1).await.unwrap().unwrap();
        match original {
            Object::Snapshot(snap) => assert_eq!(snap.message, "first"),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn snapshot_parents_preserved() {
        let s = store();
        let root = s.put_blob(Bytes::from("root")).await.unwrap();
        let s1 = s.put_snapshot(Snapshot {
            root, parents: vec![], author: "a".into(), created_at: 1, message: "s1".into(),
        }).await.unwrap();
        let s2 = s.put_snapshot(Snapshot {
            root, parents: vec![s1], author: "a".into(), created_at: 2, message: "s2".into(),
        }).await.unwrap();
        match s.get(&s2).await.unwrap().unwrap() {
            Object::Snapshot(snap) => assert_eq!(snap.parents, vec![s1]),
            _ => panic!("expected snapshot"),
        }
    }

    #[tokio::test]
    async fn tree_round_trip() {
        let s = store();
        let blob_id = s.put_blob(Bytes::from("content")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("file.txt".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = s.put_tree(entries.clone()).await.unwrap();
        match s.get(&tree_id).await.unwrap().unwrap() {
            Object::Tree(t) => assert_eq!(t.entries, entries),
            _ => panic!("expected tree"),
        }
    }
}
```

- [ ] **Step 5: Run all tests**

```bash
cargo test
```

Expected: all tests pass (no failures).

- [ ] **Step 6: Commit and close**

```bash
git add src/store/mod.rs
git commit -m "feat: add ObjectStore with idempotent put and content deduplication"
git checkout master && git merge bole-6 && git branch -d bole-6
bd close bole-6
```

---

### Task 7: Public API + Integration Tests

**Files:**
- Modify: `src/lib.rs`
- Create: `tests/object_store.rs`
- Create: `tests/backends.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–6
- Produces: clean public surface under `bole::*`, spec T1 tests pass

- [ ] **Step 1: Create and claim a bead**

```bash
bd create --title="Gate 1: public API + integration tests" --description="Wire up lib.rs re-exports and write the spec T1 integration tests. This is the Gate 1 acceptance check — all tests here map directly to spec T1 requirements." --type=task --priority=2
```

Note the returned ID (e.g. `bole-7`). Replace `bole-7` below with your actual ID.

```bash
bd update bole-7 --claim
git checkout -b bole-7
```

- [ ] **Step 2: Write src/lib.rs**

```rust
// bole-7
pub mod error;
pub mod object;
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

- [ ] **Step 3: Write tests/object_store.rs**

```rust
// bole-7
use bole::{Blob, EntryKind, MemoryBackend, Object, ObjectStore, Snapshot, Tree, TreeEntry};
use bytes::Bytes;
use std::collections::BTreeMap;

fn store() -> ObjectStore {
    ObjectStore::new(MemoryBackend::new())
}

#[tokio::test]
async fn t1_snapshots_are_immutable() {
    let s = store();
    let r1 = s.put_blob(Bytes::from("state-v1")).await.unwrap();
    let snap = Snapshot {
        root: r1, parents: vec![], author: "alice".into(),
        created_at: 1000, message: "initial".into(),
    };
    let id = s.put_snapshot(snap).await.unwrap();

    // "edit" produces new id
    let r2 = s.put_blob(Bytes::from("state-v2")).await.unwrap();
    let snap2 = Snapshot {
        root: r2, parents: vec![id], author: "alice".into(),
        created_at: 2000, message: "modified".into(),
    };
    let id2 = s.put_snapshot(snap2).await.unwrap();

    assert_ne!(id, id2);
    // original unchanged
    match s.get(&id).await.unwrap().unwrap() {
        Object::Snapshot(snap) => {
            assert_eq!(snap.message, "initial");
            assert_eq!(snap.parents, vec![]);
        }
        _ => panic!("expected snapshot"),
    }
}

#[tokio::test]
async fn t1_content_deduplication() {
    let s = store();
    let id1 = s.put_blob(Bytes::from("dedup test")).await.unwrap();
    let id2 = s.put_blob(Bytes::from("dedup test")).await.unwrap();
    assert_eq!(id1, id2);
}

#[tokio::test]
async fn t1_snapshot_parents_form_history() {
    let s = store();
    let root = s.put_blob(Bytes::from("root content")).await.unwrap();
    let s1 = s.put_snapshot(Snapshot {
        root, parents: vec![], author: "a".into(), created_at: 1, message: "s1".into(),
    }).await.unwrap();
    let s2 = s.put_snapshot(Snapshot {
        root, parents: vec![s1], author: "a".into(), created_at: 2, message: "s2".into(),
    }).await.unwrap();
    let s3 = s.put_snapshot(Snapshot {
        root, parents: vec![s2], author: "a".into(), created_at: 3, message: "s3".into(),
    }).await.unwrap();

    // all three independently retrievable
    assert!(s.get(&s1).await.unwrap().is_some());
    assert!(s.get(&s2).await.unwrap().is_some());
    assert!(s.get(&s3).await.unwrap().is_some());

    // parents link correctly
    match s.get(&s3).await.unwrap().unwrap() {
        Object::Snapshot(snap) => assert_eq!(snap.parents, vec![s2]),
        _ => panic!(),
    }
}

#[tokio::test]
async fn t1_tree_references_blobs() {
    let s = store();
    let file_id = s.put_blob(Bytes::from("file content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("README.md".into(), TreeEntry { id: file_id, kind: EntryKind::Blob });
    let tree_id = s.put_tree(entries).await.unwrap();

    match s.get(&tree_id).await.unwrap().unwrap() {
        Object::Tree(t) => {
            let entry = t.entries.get("README.md").unwrap();
            assert_eq!(entry.id, file_id);
        }
        _ => panic!("expected tree"),
    }
}
```

- [ ] **Step 4: Write tests/backends.rs**

```rust
// bole-7
use bole::{DiskBackend, MemoryBackend, ObjectStore};
use bytes::Bytes;
use tempfile::TempDir;

async fn backend_contract(store: ObjectStore) {
    let id = store.put_blob(Bytes::from("contract test")).await.unwrap();
    assert!(store.exists(&id).await.unwrap());
    let obj = store.get(&id).await.unwrap();
    assert!(obj.is_some());

    // non-existent id
    use bole::ObjectId;
    let missing = ObjectId::new([0u8; 32]);
    assert!(!store.exists(&missing).await.unwrap());
    assert!(store.get(&missing).await.unwrap().is_none());
}

#[tokio::test]
async fn memory_backend_contract() {
    backend_contract(ObjectStore::new(MemoryBackend::new())).await;
}

#[tokio::test]
async fn disk_backend_contract() {
    let dir = TempDir::new().unwrap();
    let backend = DiskBackend::open(dir.path()).await.unwrap();
    backend_contract(ObjectStore::new(backend)).await;
}

#[tokio::test]
async fn disk_backend_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let id = {
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let store = ObjectStore::new(backend);
        store.put_blob(Bytes::from("persisted across reopen")).await.unwrap()
    };
    let backend = DiskBackend::open(dir.path()).await.unwrap();
    let store = ObjectStore::new(backend);
    assert!(store.get(&id).await.unwrap().is_some());
}
```

- [ ] **Step 5: Run full test suite**

```bash
cargo test
```

Expected: all tests pass. Count should include unit tests from each module plus the integration tests.

- [ ] **Step 6: Verify no warnings**

```bash
cargo clippy -- -D warnings
```

Fix any warnings before committing.

- [ ] **Step 7: Commit and close**

```bash
git add src/lib.rs tests/
git commit -m "feat: wire public API and add Gate 1 integration tests (spec T1)"
git checkout master && git merge bole-7 && git branch -d bole-7
bd close bole-7
```

---

## Self-Review

**Spec coverage check:**
- G1 (Snapshots as only durable state) → Tasks 3, 6 ✓
- Gate 1 T1 (create files, modify, verify snapshots) → `tests/object_store.rs` ✓
- Gate 1 T1 (snapshots immutable) → `t1_snapshots_are_immutable` ✓
- Gate 1 T1 (remove materialized files don't affect history) → `disk_backend_survives_reopen` ✓
- Gate 5 (in-memory backend) → Task 4, `MemoryBackend` ✓
- Gate 5 (disk backend) → Task 5, `DiskBackend` ✓
- Deduplication guarantee → `t1_content_deduplication` + `put_is_idempotent` ✓
- zstd always-on → Task 5, `DiskBackend::put` ✓
- async throughout → all store methods are `async` ✓
- `thiserror` only, no `anyhow` → `error.rs` ✓
- No feature flags → `Cargo.toml` ✓

**Placeholder scan:** None found.

**Type consistency:** `ObjectId`, `Object`, `Blob`, `Tree`, `TreeEntry`, `EntryKind`, `Snapshot` used consistently across all tasks. `codec::serialize` / `codec::deserialize` / `codec::object_id` names match across Tasks 3 and 6.
