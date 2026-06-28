# Gate 1: Content-Addressed Object Store

**Project:** bole — a next-generation version control system  
**Language:** Rust (async, tokio)  
**Date:** 2026-06-26  
**Spec ref:** spec.md Gate 1, Test T1

---

## Context

Bole is a version control system built library-first. The CLI is a separate artifact on top. This document covers Gate 1: the content-addressed object store, which is the only durable primitive in the system. Everything else (tags, timelines, permissions, secrets) builds on top of this.

Key decisions made before this design:
- **Library-first**, CLI second
- **Async from the start** (tokio)
- **BLAKE3** for content addressing
- **Pragmatic but curated** dependency philosophy
- **Trait object** (`Box<dyn StorageBackend>`) for backend abstraction
- **zstd compression** always-on in DiskBackend (no feature flag)

---

## Core Types & Object Model

Every durable value is an `Object`, content-addressed by its BLAKE3 hash.

```rust
// 32-byte BLAKE3 digest — the identity of everything
pub struct ObjectId([u8; 32]);

pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
}

pub struct Blob {
    pub data: Bytes,
}

pub struct Tree {
    pub entries: BTreeMap<String, TreeEntry>,
}

pub struct TreeEntry {
    pub id: ObjectId,
    pub kind: EntryKind, // Blob | Tree
}

pub struct Snapshot {
    pub root: ObjectId,         // points to a Tree
    pub parents: Vec<ObjectId>, // prior Snapshots
    pub author: String,
    pub created_at: u64,        // unix timestamp
    pub message: String,
}
```

Serialization: `serde` + `postcard` (compact binary, pure Rust). The `ObjectId` is computed as the BLAKE3 hash of the serialized bytes — same content always produces the same id.

---

## StorageBackend Trait

The backend operates on raw bytes. Serialization and hashing happen in `ObjectStore`, not the backend.

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()>;
    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>>;
    async fn exists(&self, id: &ObjectId) -> Result<bool>;
    async fn delete(&self, id: &ObjectId) -> Result<()>;
}
```

### MemoryBackend

`Arc<RwLock<HashMap<ObjectId, Bytes>>>` — cheap to clone, safe for concurrent async access, no persistence. Used in tests and agent ephemeral workflows.

### DiskBackend

Objects stored as files under a root directory, sharded by hash prefix (e.g. `objects/3a/f7c2...`), same layout as Git's loose objects. Uses `tokio::fs`. zstd compression is always applied — no opt-out. This keeps storage costs low from day one.

Both backends are included in the crate with no feature flags.

---

## ObjectStore API

The public-facing layer owns serialization, hashing, and deduplication.

```rust
pub struct ObjectStore {
    backend: Box<dyn StorageBackend>,
}

impl ObjectStore {
    pub fn new(backend: impl StorageBackend + 'static) -> Self;

    // Core read/write
    pub async fn put(&self, obj: &Object) -> Result<ObjectId>;
    pub async fn get(&self, id: &ObjectId) -> Result<Option<Object>>;
    pub async fn exists(&self, id: &ObjectId) -> Result<bool>;

    // Convenience
    pub async fn put_blob(&self, data: Bytes) -> Result<ObjectId>;
    pub async fn put_tree(&self, entries: BTreeMap<String, TreeEntry>) -> Result<ObjectId>;
    pub async fn put_snapshot(&self, snap: Snapshot) -> Result<ObjectId>;
}
```

`put` is idempotent — if the object already exists (same hash), the write is a no-op. This is the deduplication guarantee.

Error handling: crate-level `Error` enum via `thiserror`. No `anyhow` in library code.

---

## Crate Structure

```
bole/
├── Cargo.toml
├── src/
│   ├── lib.rs              # public re-exports only
│   ├── error.rs            # crate-level Error + Result
│   ├── object/
│   │   ├── mod.rs
│   │   ├── id.rs           # ObjectId, hashing
│   │   ├── blob.rs
│   │   ├── tree.rs
│   │   └── snapshot.rs
│   ├── store/
│   │   ├── mod.rs          # ObjectStore
│   │   ├── backend.rs      # StorageBackend trait
│   │   ├── memory.rs       # MemoryBackend
│   │   └── disk.rs         # DiskBackend + zstd
│   └── codec.rs            # serde/postcard serialize/deserialize
└── tests/
    ├── object_store.rs     # integration tests (Gate 1 T1)
    └── backends.rs         # backend-specific tests
```

---

## Testing Approach

Tests map directly to spec T1. Both backends share a test suite:

```rust
async fn run_suite(store: ObjectStore) { ... }

#[tokio::test] async fn memory() { run_suite(ObjectStore::new(MemoryBackend::new())).await }
#[tokio::test] async fn disk()   { run_suite(ObjectStore::new(DiskBackend::open(tmp).await?)).await }
```

Test cases:

- **immutability** — "modifying" content produces a new `ObjectId`; original still resolves correctly
- **deduplication** — same blob put twice → same `ObjectId`, backend write called once
- **parent preservation** — `S2` references `S1` as parent; both retrievable independently
- **disk resilience** — deleting the on-disk file after put; object still retrievable (zstd decompression path)

---

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | async runtime |
| `blake3` | content addressing |
| `serde` + `postcard` | serialization |
| `bytes` | zero-copy byte buffers |
| `zstd` | compression in DiskBackend |
| `async-trait` | async fn in trait (or AFIT, Rust 1.75+) |
| `thiserror` | error types |

---

## Out of Scope (Gate 1)

- Tags, Timelines (Gate 2)
- Permission lattice (Gate 3)
- Secrets/EnvOverlay (Gate 4)
- Remote/custom backends (Gate 5)
- Multi-actor capability enforcement (Gate 6)
- Git projection (Gate 7)
