# Gate 5: In-Memory and Virtual Repos Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a thin `Repository` wrapper unifying `ObjectStore`+`RefStore`, generic backend-to-backend copy, a `materialize` function projecting Snapshot trees to disk, and T5 integration tests.

**Architecture:** `StorageBackend` gains a `list()` method enabling iteration without coupling concrete types; a new `src/repo/` module holds `Repository` (two public fields + three constructors) plus `copy_objects`/`copy_refs` free functions; `materialize` lives in `src/repo/materialize.rs` and recursively walks Tree entries, writing Blobs via `tokio::fs::write`.

**Tech Stack:** Rust (edition 2021, tokio async runtime), blake3, postcard, zstd, thiserror — all already in `Cargo.toml`. No new dependencies.

## Global Constraints

- `thiserror` only for error types — no `anyhow` anywhere in library code
- Both `MemoryBackend` and `DiskBackend` always compiled — no feature flags
- zstd compression always-on in `DiskBackend` — no opt-out
- Bead required before any code is written: `bd create`, then `bd update <id> --claim`, then `git checkout -b <id>`
- Branch name must exactly match the bead ID
- Tests must pass before merge
- After merge: `git branch -d <id>` then `bd close <id>`
- Conservative git profile: no push, no dolt sync without explicit request
- Bead comment on each contiguous block of new code: `// <bead-id>` — one comment per block, not per line

---

## File Map

| File | Status | Purpose |
|------|--------|---------|
| `src/store/backend.rs` | Modify | Add `list()` to `StorageBackend` trait |
| `src/store/memory.rs` | Modify | Impl `list()` — iterate HashMap keys |
| `src/store/disk.rs` | Modify | Impl `list()` — walk `objects/` shards; add hex helpers |
| `src/store/mod.rs` | Modify | Add `ObjectStore::list()` delegating to backend |
| `src/refs/mod.rs` | Modify | Add `pub(crate) fn set_raw()` to `RefStore` |
| `src/repo/mod.rs` | Create | `Repository` struct + `memory()`/`disk()`/`copy_to()` + `copy_objects` + `copy_refs` |
| `src/repo/materialize.rs` | Create | `materialize()` free async fn + `write_tree()` recursive helper |
| `src/lib.rs` | Modify | `pub mod repo` + re-exports |
| `tests/repo.rs` | Create | T5 integration tests |

---

## Task 1: `list()` on StorageBackend + ObjectStore

**Files:**
- Modify: `src/store/backend.rs`
- Modify: `src/store/memory.rs`
- Modify: `src/store/disk.rs`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `ObjectId::new([u8;32])`, `ObjectId::as_bytes() -> &[u8;32]` (already in `src/object/id.rs`)
- Produces:
  - `StorageBackend::list(&self) -> Result<Vec<ObjectId>>` (trait method)
  - `ObjectStore::list(&self) -> Result<Vec<ObjectId>>` (public method)

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 5 T1: list() on StorageBackend" \
  --description="Add list() method to StorageBackend trait, implement in MemoryBackend (iterate HashMap) and DiskBackend (walk objects/ shards), expose as ObjectStore::list(). Required for copy_objects free fn in Task 2." \
  --type=task --priority=2
# Note the bead ID printed, e.g. bole-abc
bd update bole-abc --claim
git checkout -b bole-abc
```

- [ ] **Step 2: Write failing tests**

Add to `src/store/memory.rs` inside the `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test]
    async fn list_returns_all_ids() {
        let backend = MemoryBackend::new();
        let id1 = ObjectId::from_bytes(b"a");
        let id2 = ObjectId::from_bytes(b"b");
        let id3 = ObjectId::from_bytes(b"c");
        backend.put(&id1, b"data1").await.unwrap();
        backend.put(&id2, b"data2").await.unwrap();
        backend.put(&id3, b"data3").await.unwrap();
        let ids = backend.list().await.unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
    }

    #[tokio::test]
    async fn list_empty_store_returns_empty() {
        let backend = MemoryBackend::new();
        assert!(backend.list().await.unwrap().is_empty());
    }
```

Add to `src/store/disk.rs` inside the `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test]
    async fn list_returns_all_ids() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        let id1 = ObjectId::from_bytes(b"a");
        let id2 = ObjectId::from_bytes(b"b");
        backend.put(&id1, b"data1").await.unwrap();
        backend.put(&id2, b"data2").await.unwrap();
        let ids = backend.list().await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[tokio::test]
    async fn list_empty_store_returns_empty() {
        let dir = TempDir::new().unwrap();
        let backend = DiskBackend::open(dir.path()).await.unwrap();
        assert!(backend.list().await.unwrap().is_empty());
    }
```

Add to `src/store/mod.rs` inside the `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test]
    async fn object_store_list() {
        let s = store();
        let id1 = s.put_blob(Bytes::from("foo")).await.unwrap();
        let id2 = s.put_blob(Bytes::from("bar")).await.unwrap();
        let ids = s.list().await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test list 2>&1 | head -30
```

Expected: compile errors — `list` not found on `StorageBackend` or `ObjectStore`.

- [ ] **Step 4: Add `list()` to the trait**

In `src/store/backend.rs`, replace the entire file with:

```rust
// bole-mbt
// <bead-id>
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
    async fn list(&self) -> Result<Vec<ObjectId>>;
}
```

- [ ] **Step 5: Implement `list()` in `MemoryBackend`**

In `src/store/memory.rs`, add this method to the `impl StorageBackend for MemoryBackend` block (after `delete`):

```rust
    // <bead-id>
    async fn list(&self) -> Result<Vec<ObjectId>> {
        Ok(self.store.read().await
            .keys()
            .map(|k| ObjectId::new(*k))
            .collect())
    }
```

- [ ] **Step 6: Implement `list()` in `DiskBackend`**

In `src/store/disk.rs`, add two private helper functions before the `impl DiskBackend` block (after the `use` declarations):

```rust
// <bead-id>
fn parse_hex_id(hex: &str) -> Option<ObjectId> {
    if hex.len() != 64 { return None; }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let hi = hex_nibble(hex.as_bytes()[i * 2])?;
        let lo = hex_nibble(hex.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(ObjectId::new(bytes))
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}
```

Then add this method to the `impl StorageBackend for DiskBackend` block (after `delete`):

```rust
    // <bead-id>
    async fn list(&self) -> Result<Vec<ObjectId>> {
        let objects_dir = self.root.join("objects");
        let mut ids = Vec::new();
        let mut shards = match tokio::fs::read_dir(&objects_dir).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(Error::Io(e)),
        };
        while let Some(shard) = shards.next_entry().await? {
            let prefix = shard.file_name().to_string_lossy().into_owned();
            if prefix.len() != 2 { continue; }
            let mut entries = match tokio::fs::read_dir(shard.path()).await {
                Ok(d) => d,
                Err(_) => continue,
            };
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".tmp") { continue; }
                let hex = format!("{}{}", prefix, name);
                if let Some(id) = parse_hex_id(&hex) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }
```

- [ ] **Step 7: Add `ObjectStore::list()`**

In `src/store/mod.rs`, add this method to `impl ObjectStore` (after `put_snapshot`):

```rust
    // <bead-id>
    pub async fn list(&self) -> Result<Vec<ObjectId>> {
        self.backend.list().await
    }
```

- [ ] **Step 8: Run tests and verify they pass**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. If `list` is not found, check that the trait method signature matches in all three files.

- [ ] **Step 9: Commit**

```bash
git add src/store/backend.rs src/store/memory.rs src/store/disk.rs src/store/mod.rs
git commit -m "feat(store): add list() to StorageBackend trait and ObjectStore"
```

- [ ] **Step 10: Merge and close**

```bash
git checkout master && git merge bole-abc
git branch -d bole-abc
bd close bole-abc
```

---

## Task 2: Repository + copy utilities + `set_raw`

**Files:**
- Modify: `src/refs/mod.rs` — add `set_raw` to `RefStore`
- Create: `src/repo/mod.rs` — `copy_objects`, `copy_refs`, `Repository`
- Modify: `src/lib.rs` — `pub mod repo` + re-exports

**Interfaces:**
- Consumes:
  - `ObjectStore::list(&self) -> Result<Vec<ObjectId>>` (Task 1)
  - `ObjectStore::get(&self, id: &ObjectId) -> Result<Option<Object>>`
  - `ObjectStore::put(&self, obj: &Object) -> Result<ObjectId>`
  - `RefStore::list(&self, prefix: &str) -> Result<Vec<RefName>>`
  - `RefStore::get(&self, name: &RefName) -> Result<Option<Ref>>`
  - `DiskBackend::open(root: impl AsRef<Path>) -> Result<Self>` (async)
  - `DiskRefBackend::open(root: impl AsRef<Path>) -> Result<Self>` (sync)
  - `MemoryBackend::new() -> Self`
  - `MemoryRefBackend::new() -> Self`
- Produces:
  - `pub(crate) fn RefStore::set_raw(&self, name: &RefName, r: &Ref) -> Result<()>`
  - `pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()>`
  - `pub fn copy_refs(from: &RefStore, to: &RefStore) -> Result<()>`
  - `pub struct Repository { pub objects: ObjectStore, pub refs: RefStore }`
  - `pub fn Repository::memory() -> Self`
  - `pub async fn Repository::disk(root: impl AsRef<Path>) -> Result<Self>`
  - `pub async fn Repository::copy_to(&self, dest: &Repository) -> Result<()>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 5 T2: Repository type and copy utilities" \
  --description="Add set_raw to RefStore (pub(crate)), create src/repo/mod.rs with copy_objects (async), copy_refs (sync), and Repository struct (memory/disk constructors + copy_to). Wire up pub mod repo in lib.rs." \
  --type=task --priority=2
bd update bole-xyz --claim
git checkout -b bole-xyz
```

- [ ] **Step 2: Write failing tests**

Create `src/repo/mod.rs` with tests only (stub module so it compiles):

```rust
// <bead-id>
pub mod materialize;

use std::path::Path;
use crate::error::Result;
use crate::object::ObjectId;
use crate::refs::{DiskRefBackend, MemoryRefBackend, Ref, RefName, RefStore};
use crate::store::{disk::DiskBackend, memory::MemoryBackend, ObjectStore};

pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
}

impl Repository {
    pub fn memory() -> Self { todo!() }
    pub async fn disk(_root: impl AsRef<Path>) -> Result<Self> { todo!() }
    pub async fn copy_to(&self, _dest: &Repository) -> Result<()> { todo!() }
}

pub async fn copy_objects(_from: &ObjectStore, _to: &ObjectStore) -> Result<()> { todo!() }
pub fn copy_refs(_from: &RefStore, _to: &RefStore) -> Result<()> { todo!() }

#[cfg(test)]
mod tests {
    use super::{copy_objects, copy_refs, Repository};
    use crate::object::{ObjectId, Snapshot};
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
}
```

Also create `src/repo/materialize.rs` as an empty stub so the `pub mod materialize` compiles:

```rust
// placeholder — implemented in Task 3
```

- [ ] **Step 3: Add `pub mod repo` to `src/lib.rs`**

In `src/lib.rs`, add after the existing `pub mod` declarations:

```rust
// <bead-id>
pub mod repo;
pub use repo::{copy_objects, materialize::materialize, Repository};
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
cargo test repo 2>&1 | head -40
```

Expected: `todo!()` panics or compile errors. That's correct — tests should fail.

- [ ] **Step 5: Add `set_raw` to `RefStore`**

In `src/refs/mod.rs`, inside the `mod store { ... impl RefStore { ... } }` block, add after the `list` method:

```rust
        // <bead-id>
        pub(crate) fn set_raw(&self, name: &RefName, r: &Ref) -> Result<()> {
            self.backend.set(name, r)
        }
```

- [ ] **Step 6: Implement `copy_objects` and `copy_refs`**

In `src/repo/mod.rs`, replace the `todo!()` stubs for `copy_objects` and `copy_refs`:

```rust
// <bead-id>
pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()> {
    for id in from.list().await? {
        if let Some(obj) = from.get(&id).await? {
            to.put(&obj).await?;
        }
    }
    Ok(())
}

pub fn copy_refs(from: &RefStore, to: &RefStore) -> Result<()> {
    for name in from.list("")? {
        if let Some(r) = from.get(&name)? {
            to.set_raw(&name, &r)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 7: Implement `Repository`**

In `src/repo/mod.rs`, replace the `todo!()` stubs for `Repository`:

```rust
// <bead-id>
impl Repository {
    pub fn memory() -> Self {
        Self {
            objects: ObjectStore::new(MemoryBackend::new()),
            refs: RefStore::new(MemoryRefBackend::new()),
        }
    }

    pub async fn disk(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            objects: ObjectStore::new(DiskBackend::open(root).await?),
            refs: RefStore::new(DiskRefBackend::open(root)?),
        })
    }

    pub async fn copy_to(&self, dest: &Repository) -> Result<()> {
        copy_objects(&self.objects, &dest.objects).await?;
        copy_refs(&self.refs, &dest.refs)?;
        Ok(())
    }
}
```

- [ ] **Step 8: Run tests and verify they pass**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests plus the new repo tests pass. If `set_raw not found`, check that `pub(crate)` is in the `impl RefStore` block inside `mod store`, not outside it.

- [ ] **Step 9: Commit**

```bash
git add src/refs/mod.rs src/repo/mod.rs src/repo/materialize.rs src/lib.rs
git commit -m "feat(repo): add Repository type, copy_objects, copy_refs, set_raw"
```

- [ ] **Step 10: Merge and close**

```bash
git checkout master && git merge bole-xyz
git branch -d bole-xyz
bd close bole-xyz
```

---

## Task 3: `materialize`

**Files:**
- Modify: `src/repo/materialize.rs` — full implementation replacing stub
- Modify: `src/repo/mod.rs` — promote `pub mod materialize` re-export

**Interfaces:**
- Consumes:
  - `ObjectStore::get(&self, id: &ObjectId) -> Result<Option<Object>>`
  - `Object::Snapshot(Snapshot)`, `Object::Tree(Tree)`, `Object::Blob(Blob)`
  - `Snapshot { root: ObjectId, .. }` (from `src/object/snapshot.rs`)
  - `Tree { entries: BTreeMap<String, TreeEntry> }` (from `src/object/tree.rs`)
  - `TreeEntry { id: ObjectId, kind: EntryKind }` with `EntryKind::Blob | Tree`
  - `Blob { data: Bytes }` (from `src/object/blob.rs`)
- Produces:
  - `pub async fn materialize(objects: &ObjectStore, snapshot_id: ObjectId, dest: impl AsRef<Path>) -> Result<()>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 5 T3: materialize() function" \
  --description="Implement materialize(objects, snapshot_id, dest): fetch Snapshot, recursively walk Tree entries, write Blobs to dest directory. Async recursive helper uses Box::pin. Error on missing or wrong-type objects." \
  --type=task --priority=2
bd update bole-pqr --claim
git checkout -b bole-pqr
```

- [ ] **Step 2: Write failing unit tests**

Replace the stub content of `src/repo/materialize.rs` with the full file including tests first:

```rust
// <bead-id>
use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId};
use crate::store::ObjectStore;
use std::path::Path;

pub async fn materialize(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
    dest: impl AsRef<Path>,
) -> Result<()> {
    todo!()
}

async fn write_tree(objects: &ObjectStore, tree_id: ObjectId, base: &Path) -> Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::materialize;
    use crate::object::{EntryKind, ObjectId, Snapshot, TreeEntry};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use bytes::Bytes;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn store() -> ObjectStore { ObjectStore::new(MemoryBackend::new()) }

    #[tokio::test]
    async fn missing_snapshot_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let id = ObjectId::new([9u8; 32]);
        let err = materialize(&s, id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }

    #[tokio::test]
    async fn wrong_object_type_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let blob_id = s.put_blob(Bytes::from("not a snapshot")).await.unwrap();
        let err = materialize(&s, blob_id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }

    #[tokio::test]
    async fn simple_flat_tree() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let blob_id = s.put_blob(Bytes::from("hello world")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("hello.txt".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = s.put_tree(entries).await.unwrap();
        let snap_id = s.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        materialize(&s, snap_id, dir.path()).await.unwrap();
        let content = std::fs::read(dir.path().join("hello.txt")).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn nested_directory_tree() {
        let s = store();
        let dir = TempDir::new().unwrap();

        let nested_blob = s.put_blob(Bytes::from("nested content")).await.unwrap();
        let mut nested_entries = BTreeMap::new();
        nested_entries.insert("file.txt".into(), TreeEntry { id: nested_blob, kind: EntryKind::Blob });
        let nested_tree = s.put_tree(nested_entries).await.unwrap();

        let root_blob = s.put_blob(Bytes::from("root content")).await.unwrap();
        let mut root_entries = BTreeMap::new();
        root_entries.insert("root.txt".into(), TreeEntry { id: root_blob, kind: EntryKind::Blob });
        root_entries.insert("sub".into(), TreeEntry { id: nested_tree, kind: EntryKind::Tree });
        let root_tree = s.put_tree(root_entries).await.unwrap();

        let snap_id = s.put_snapshot(Snapshot {
            root: root_tree, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        materialize(&s, snap_id, dir.path()).await.unwrap();

        assert_eq!(std::fs::read(dir.path().join("root.txt")).unwrap(), b"root content");
        assert_eq!(std::fs::read(dir.path().join("sub/file.txt")).unwrap(), b"nested content");
    }

    #[tokio::test]
    async fn missing_blob_errors() {
        let s = store();
        let dir = TempDir::new().unwrap();
        let missing_blob = ObjectId::new([7u8; 32]);
        let mut entries = BTreeMap::new();
        entries.insert("gone.txt".into(), TreeEntry { id: missing_blob, kind: EntryKind::Blob });
        let tree_id = s.put_tree(entries).await.unwrap();
        let snap_id = s.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();
        let err = materialize(&s, snap_id, dir.path()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test materialize 2>&1 | head -30
```

Expected: `todo!()` panics on the tests that call into the implementation. `missing_snapshot_errors` and `wrong_object_type_errors` will also fail because the function panics instead of returning errors.

- [ ] **Step 4: Implement `materialize` and `write_tree`**

Replace the `todo!()` bodies (keep the `#[cfg(test)]` block unchanged):

```rust
// <bead-id>
pub async fn materialize(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
    dest: impl AsRef<Path>,
) -> Result<()> {
    let dest = dest.as_ref();
    tokio::fs::create_dir_all(dest).await?;
    let snap = match objects.get(&snapshot_id).await? {
        Some(Object::Snapshot(s)) => s,
        Some(_) => return Err(Error::Storage(format!("{} is not a snapshot", snapshot_id))),
        None => return Err(Error::Storage(format!("snapshot not found: {}", snapshot_id))),
    };
    write_tree(objects, snap.root, dest).await
}

async fn write_tree(objects: &ObjectStore, tree_id: ObjectId, base: &Path) -> Result<()> {
    let tree = match objects.get(&tree_id).await? {
        Some(Object::Tree(t)) => t,
        Some(_) => return Err(Error::Storage(format!("{} is not a tree", tree_id))),
        None => return Err(Error::Storage(format!("tree not found: {}", tree_id))),
    };
    for (name, entry) in &tree.entries {
        let path = base.join(name);
        match entry.kind {
            EntryKind::Blob => match objects.get(&entry.id).await? {
                Some(Object::Blob(b)) => tokio::fs::write(&path, &b.data).await?,
                Some(_) => return Err(Error::Storage(format!("{} is not a blob", entry.id))),
                None => return Err(Error::Storage(format!("blob not found: {}", entry.id))),
            },
            EntryKind::Tree => {
                tokio::fs::create_dir_all(&path).await?;
                Box::pin(write_tree(objects, entry.id, &path)).await?;
            }
        }
    }
    Ok(())
}
```

**Note:** `write_tree` is async and calls itself recursively. Rust requires `Box::pin(write_tree(...))` to make the recursive async call compile — the future's size must be known at compile time, so we heap-allocate it.

- [ ] **Step 5: Run tests and verify they pass**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass including the new materialize tests. If you see "recursive async function" errors, verify the `Box::pin(write_tree(...)).await?` pattern is used inside the `EntryKind::Tree` arm.

- [ ] **Step 6: Commit**

```bash
git add src/repo/materialize.rs
git commit -m "feat(repo): implement materialize() with recursive tree walk"
```

- [ ] **Step 7: Merge and close**

```bash
git checkout master && git merge bole-pqr
git branch -d bole-pqr
bd close bole-pqr
```

---

## Task 4: T5 Integration Tests

**Files:**
- Create: `tests/repo.rs`

**Interfaces:**
- Consumes all public APIs from Tasks 1–3:
  - `bole::{Repository, materialize}`
  - `bole::store::{memory::MemoryBackend, ObjectStore}`
  - `bole::refs::{MemoryRefBackend, RefName, RefStore, TimelinePolicy}`
  - `bole::object::{EntryKind, ObjectId, Snapshot, Tree, TreeEntry}`
  - `bytes::Bytes`
  - `tempfile::TempDir`
- Produces: two integration tests validating T5 spec requirements

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 5 T4: T5 integration tests" \
  --description="Create tests/repo.rs with two T5 spec tests: (1) 1000 snapshot round-trip memory->disk->reload verifying all IDs and 10 tags, (2) materialize a 3-file nested snapshot, drop dest, re-materialize to new dest and verify contents match." \
  --type=task --priority=2
bd update bole-rst --claim
git checkout -b bole-rst
```

- [ ] **Step 2: Write the integration tests**

Create `tests/repo.rs`:

```rust
// <bead-id>
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{materialize, Repository};
use bytes::Bytes;
use std::collections::BTreeMap;
use tempfile::TempDir;

/// T5: 1000 snapshot round-trip. Create in-memory repo with 1000 sequential
/// snapshots (each pointing to the previous as its parent), tag every 100th
/// snapshot, copy to disk, reload, verify all IDs and tag targets survive.
#[tokio::test]
async fn t5_memory_to_disk_round_trip() {
    let mem_repo = Repository::memory();

    let mut snap_ids = Vec::with_capacity(1000);
    let mut prev: Option<bole::object::ObjectId> = None;

    for i in 0u32..1000 {
        let content = i.to_le_bytes().to_vec();
        let blob_id = mem_repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("data".to_string(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = mem_repo.objects.put_tree(entries).await.unwrap();
        let parents = prev.map_or_else(Vec::new, |p| vec![p]);
        let snap_id = mem_repo.objects.put_snapshot(Snapshot {
            root: tree_id,
            parents,
            author: "test".to_string(),
            created_at: i as u64,
            message: format!("snap {}", i),
        }).await.unwrap();
        snap_ids.push(snap_id);
        prev = Some(snap_id);
    }

    // Tag every 100th snapshot
    let mut tagged: Vec<(RefName, bole::object::ObjectId)> = Vec::new();
    for j in 0..10usize {
        let name = RefName::new(format!("milestone/{}", j)).unwrap();
        let target = snap_ids[j * 100];
        mem_repo.refs.create_tag(name.clone(), target, None, j as u64).unwrap();
        tagged.push((name, target));
    }

    // Copy to disk repo
    let dir = TempDir::new().unwrap();
    let disk_repo = Repository::disk(dir.path()).await.unwrap();
    mem_repo.copy_to(&disk_repo).await.unwrap();
    drop(disk_repo);

    // Reload from same directory
    let reloaded = Repository::disk(dir.path()).await.unwrap();

    // All 1000 snapshot IDs must be present
    for snap_id in &snap_ids {
        assert!(
            reloaded.objects.exists(snap_id).await.unwrap(),
            "snapshot {} missing after reload",
            snap_id
        );
    }

    // All 10 tags must have correct targets
    for (name, expected_target) in &tagged {
        let tag = reloaded.refs.get_tag(name).unwrap()
            .unwrap_or_else(|| panic!("tag {} missing after reload", name.as_str()));
        assert_eq!(
            tag.target, *expected_target,
            "tag {} has wrong target after reload",
            name.as_str()
        );
    }
}

/// T5: materialize and re-materialize. Build a 3-file nested snapshot in
/// memory, materialize to a temp dir, drop the dir, then materialize again
/// to a second dir and verify all contents still match (objects stay in store).
#[tokio::test]
async fn t5_materialize_and_rematerialize() {
    let repo = Repository::memory();

    // Build a small nested tree:
    //   src/main.rs -> "fn main() {}"
    //   README.md   -> "hello"
    //   nested/a.txt -> "a"
    let main_blob = repo.objects.put_blob(Bytes::from("fn main() {}")).await.unwrap();
    let readme_blob = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
    let nested_blob = repo.objects.put_blob(Bytes::from("a")).await.unwrap();

    let mut nested_entries = BTreeMap::new();
    nested_entries.insert("a.txt".to_string(), TreeEntry { id: nested_blob, kind: EntryKind::Blob });
    let nested_tree_id = repo.objects.put_tree(nested_entries).await.unwrap();

    let mut src_entries = BTreeMap::new();
    src_entries.insert("main.rs".to_string(), TreeEntry { id: main_blob, kind: EntryKind::Blob });
    let src_tree_id = repo.objects.put_tree(src_entries).await.unwrap();

    let mut root_entries = BTreeMap::new();
    root_entries.insert("src".to_string(), TreeEntry { id: src_tree_id, kind: EntryKind::Tree });
    root_entries.insert("README.md".to_string(), TreeEntry { id: readme_blob, kind: EntryKind::Blob });
    root_entries.insert("nested".to_string(), TreeEntry { id: nested_tree_id, kind: EntryKind::Tree });
    let root_tree_id = repo.objects.put_tree(root_entries).await.unwrap();

    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: root_tree_id, parents: vec![], author: "test".to_string(),
        created_at: 1, message: "init".to_string(),
    }).await.unwrap();

    // First materialization
    let dest1 = TempDir::new().unwrap();
    materialize(&repo.objects, snap_id, dest1.path()).await.unwrap();
    assert_eq!(std::fs::read(dest1.path().join("src/main.rs")).unwrap(), b"fn main() {}");
    assert_eq!(std::fs::read(dest1.path().join("README.md")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dest1.path().join("nested/a.txt")).unwrap(), b"a");

    // Drop dest1 — TempDir auto-deletes on drop
    drop(dest1);

    // Second materialization from same in-memory repo — objects are still in store
    let dest2 = TempDir::new().unwrap();
    materialize(&repo.objects, snap_id, dest2.path()).await.unwrap();
    assert_eq!(std::fs::read(dest2.path().join("src/main.rs")).unwrap(), b"fn main() {}");
    assert_eq!(std::fs::read(dest2.path().join("README.md")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dest2.path().join("nested/a.txt")).unwrap(), b"a");
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test --test repo 2>&1 | head -30
```

Expected: compile errors (module not wired up) or test failures from `todo!()` stubs. Either is correct at this point.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all unit tests from Tasks 1–3 pass. The integration tests in `tests/repo.rs` should also pass now because Tasks 1–3 are complete.

If either T5 test fails, diagnose:
- `t5_memory_to_disk_round_trip` failing: check `copy_to` calls both `copy_objects` and `copy_refs`, and that `DiskBackend::list()` returns all objects after write.
- `t5_materialize_and_rematerialize` failing: verify `write_tree` handles nested `EntryKind::Tree` entries and creates parent directories before writing children.

- [ ] **Step 5: Commit**

```bash
git add tests/repo.rs
git commit -m "test(repo): add T5 integration tests (round-trip and materialize)"
```

- [ ] **Step 6: Merge and close**

```bash
git checkout master && git merge bole-rst
git branch -d bole-rst
bd close bole-rst
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|-----------------|------|
| `list()` on `StorageBackend` (both backends) | Task 1 |
| `ObjectStore::list()` public method | Task 1 |
| `copy_objects` free async fn | Task 2 |
| `RefStore::set_raw` pub(crate) | Task 2 |
| `copy_refs` free fn | Task 2 |
| `Repository { objects, refs }` with public fields | Task 2 |
| `Repository::memory()` | Task 2 |
| `Repository::disk(root)` | Task 2 |
| `Repository::copy_to(dest)` | Task 2 |
| `pub mod repo` + lib.rs re-exports | Task 2 |
| `materialize(objects, snap_id, dest)` | Task 3 |
| Error on missing snapshot | Task 3 |
| Error on wrong object type | Task 3 |
| Error on missing blob | Task 3 |
| Recursive tree walk with `Box::pin` | Task 3 |
| T5 1000 snapshot round-trip | Task 4 |
| T5 materialize + drop + re-materialize | Task 4 |

**Placeholder scan:** None found — every step has concrete code.

**Type consistency check:**
- `copy_objects(&ObjectStore, &ObjectStore)` — matches usage in `copy_to` ✓
- `copy_refs(&RefStore, &RefStore)` — matches usage in `copy_to` ✓
- `set_raw(&RefName, &Ref)` — matches call in `copy_refs` ✓
- `materialize(&ObjectStore, ObjectId, impl AsRef<Path>)` — matches T5 test calls ✓
- `Repository::disk(dir.path())` — `&Path` satisfies `impl AsRef<Path>` ✓
- `EntryKind::Blob | Tree` — matches `src/object/tree.rs` ✓
