# Gate 4: Secrets and Env Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `Secret` (ChaCha20-Poly1305 encrypted bytes) and `EnvOverlay` (env var map with plain/secret values) as first-class `Object` variants, and a `Repository::compute_workspace_view` that combines ACL-filtered snapshot files with a resolved env map into a `WorkspaceView`.

**Architecture:** `Secret` and `EnvOverlay` join the existing `Object` enum and travel through the same content-addressed, zstd-compressed `ObjectStore`. Encryption is caller-supplied: `put_secret(plaintext, key)` encrypts and stores; `get_secret(id, key)` fetches and decrypts. `compute_workspace_view` reuses Gate 3's `get_snapshot_filtered` for the file half and resolves each `EnvValue` for the env half.

**Tech Stack:** Rust (edition 2021, stable, tokio), chacha20poly1305 0.10, rand 0.8, serde + postcard, thiserror — all in Cargo.toml after Task 1.

## Global Constraints

- `thiserror` only — no `anyhow` anywhere in library code
- Both `MemoryBackend` and `DiskBackend` always compiled — no feature flags
- `// <bead-id>` comment on each contiguous block of new code — one per block, not per line
- No crate dependencies beyond what the spec names: `chacha20poly1305 = "0.10"`, `rand = "0.8"`
- Branch name = bead ID exactly
- Tests must pass before merge; delete branch after merge; close bead after delete
- Conservative git: no push, no dolt sync

---

## File Map

| File | Status | Purpose |
|------|--------|---------|
| `Cargo.toml` | Modify | Add `chacha20poly1305`, `rand` |
| `src/object/secret.rs` | Create | `Secret` struct + `encrypt`/`decrypt` |
| `src/object/env.rs` | Create | `EnvValue` enum + `EnvOverlay` struct |
| `src/object/mod.rs` | Modify | Add `pub mod secret; pub mod env;`, two new `Object` variants, re-exports |
| `src/error.rs` | Modify | Add `DecryptionFailed`, `SecretNotUtf8` |
| `src/store/mod.rs` | Modify | Add `put_secret`, `get_secret`, `put_overlay`, `get_overlay` |
| `src/repo/workspace.rs` | Create | `WorkspaceView` struct |
| `src/repo/mod.rs` | Modify | Add `pub mod workspace;`, `compute_workspace_view` |
| `src/lib.rs` | Modify | Re-export `Secret`, `EnvOverlay`, `EnvValue`, `WorkspaceView` |
| `tests/secrets.rs` | Create | T4 integration tests |

---

## Task 1: Core types, error variants, Cargo deps

**Files:**
- Modify: `Cargo.toml`
- Create: `src/object/secret.rs`
- Create: `src/object/env.rs`
- Modify: `src/object/mod.rs`
- Modify: `src/error.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `crate::error::{Error, Result}`, `crate::object::ObjectId` (existing)
- Produces:
  - `pub struct Secret { pub nonce: [u8; 12], pub ciphertext: Vec<u8> }` with `Secret::encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Self>` and `Secret::decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>>`
  - `pub enum EnvValue { Plain(String), Secret(ObjectId) }`
  - `pub struct EnvOverlay { pub entries: BTreeMap<String, EnvValue> }`
  - `Object::Secret(Secret)` and `Object::EnvOverlay(EnvOverlay)` variants
  - `Error::DecryptionFailed` and `Error::SecretNotUtf8`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 4 T1: Secret, EnvOverlay types, error variants, Cargo deps" \
  --description="Add chacha20poly1305+rand deps. Secret struct with encrypt/decrypt (caller-supplied key, random nonce). EnvValue+EnvOverlay types. Object::Secret+Object::EnvOverlay variants. DecryptionFailed+SecretNotUtf8 error variants. lib.rs re-exports." \
  --type=task --priority=2
# note printed bead ID, e.g. bole-abc
bd update bole-abc --claim
git checkout -b bole-abc
```

- [ ] **Step 2: Add dependencies to `Cargo.toml`**

In `Cargo.toml`, add under `[dependencies]` after `thiserror = "1"`:

```toml
chacha20poly1305 = "0.10"
rand = "0.8"
```

- [ ] **Step 3: Write failing tests for `Secret::encrypt`/`decrypt`**

Create `src/object/secret.rs`:

```rust
// <bead-id>
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

impl Secret {
    pub fn encrypt(_plaintext: &[u8], _key: &[u8; 32]) -> Result<Self> {
        todo!()
    }

    pub fn decrypt(&self, _key: &[u8; 32]) -> Result<Vec<u8>> {
        todo!()
    }
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::Secret;
    use crate::error::Error;

    fn key() -> [u8; 32] { [42u8; 32] }
    fn wrong_key() -> [u8; 32] { [99u8; 32] }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = b"super secret value";
        let s = Secret::encrypt(plaintext, &key()).unwrap();
        let got = s.decrypt(&key()).unwrap();
        assert_eq!(got, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let s = Secret::encrypt(b"value", &key()).unwrap();
        let err = s.decrypt(&wrong_key()).unwrap_err();
        assert!(matches!(err, Error::DecryptionFailed));
    }

    #[test]
    fn two_encryptions_have_different_nonces() {
        let s1 = Secret::encrypt(b"val", &key()).unwrap();
        let s2 = Secret::encrypt(b"val", &key()).unwrap();
        // Same plaintext, different nonces → different ciphertext
        assert_ne!(s1.nonce, s2.nonce);
    }
}
```

- [ ] **Step 4: Add `DecryptionFailed` and `SecretNotUtf8` to `src/error.rs`**

```rust
// bole-49r
// bole-s5y
// bole-mhs
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
    #[error("decryption failed")] DecryptionFailed,
    #[error("secret value is not valid UTF-8")] SecretNotUtf8,
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 5: Run tests to verify they fail**

```bash
cargo test secret 2>&1 | head -20
```

Expected: compile errors (`todo!()` panics, `Error::DecryptionFailed` not yet found — add error first, then re-run).

Actually the order matters: add the error variants to `src/error.rs` first (Step 4), then create `src/object/secret.rs` (Step 3) so the `Error::DecryptionFailed` reference compiles. If you get compile errors about `DecryptionFailed` not existing, fix `src/error.rs` first.

- [ ] **Step 6: Implement `Secret::encrypt` and `Secret::decrypt`**

Replace the `todo!()` stubs in `src/object/secret.rs`:

```rust
// <bead-id>
use crate::error::{Error, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::random;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

impl Secret {
    pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Self> {
        let nonce_bytes: [u8; 12] = random();
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Codec("invalid key length".into()))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| Error::Codec("encryption failed".into()))?;
        Ok(Self { nonce: nonce_bytes, ciphertext })
    }

    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| Error::Codec("invalid key length".into()))?;
        let nonce = Nonce::from_slice(&self.nonce);
        cipher
            .decrypt(nonce, self.ciphertext.as_slice())
            .map_err(|_| Error::DecryptionFailed)
    }
}

// <bead-id>
#[cfg(test)]
mod tests {
    // ... (keep exactly as written in Step 3)
}
```

- [ ] **Step 7: Write failing tests for `EnvOverlay`**

Create `src/object/env.rs`:

```rust
// <bead-id>
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnvValue {
    Plain(String),
    Secret(ObjectId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvOverlay {
    pub entries: BTreeMap<String, EnvValue>,
}

// <bead-id>
#[cfg(test)]
mod tests {
    use super::{EnvOverlay, EnvValue};
    use crate::object::ObjectId;
    use std::collections::BTreeMap;

    #[test]
    fn overlay_serializes_roundtrip() {
        let mut entries = BTreeMap::new();
        entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
        entries.insert("DB_URL".into(), EnvValue::Secret(ObjectId::new([1u8; 32])));
        let overlay = EnvOverlay { entries };
        let encoded = postcard::to_allocvec(&overlay).unwrap();
        let decoded: EnvOverlay = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(overlay, decoded);
    }
}
```

- [ ] **Step 8: Extend `src/object/mod.rs`**

Replace the full file:

```rust
// bole-dq0
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;
// <bead-id>
pub mod env;
pub mod secret;

pub use blob::Blob;
pub use id::ObjectId;
pub use snapshot::Snapshot;
pub use tree::{EntryKind, Tree, TreeEntry};
// <bead-id>
pub use env::{EnvOverlay, EnvValue};
pub use secret::Secret;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
    // <bead-id>
    Secret(Secret),
    EnvOverlay(EnvOverlay),
}
```

- [ ] **Step 9: Add re-exports to `src/lib.rs`**

After `pub use object::{Blob, EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};` add:

```rust
// <bead-id>
pub use object::{EnvOverlay, EnvValue, Secret};
```

- [ ] **Step 10: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass plus the new `secret` and `env` tests.

- [ ] **Step 11: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean. Common warning: `EnvValue::Secret` shadowing the `Secret` type name — use `EnvValue::Secret(id)` unambiguously in patterns.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml Cargo.lock src/object/secret.rs src/object/env.rs src/object/mod.rs src/error.rs src/lib.rs
git commit -m "feat(object): add Secret and EnvOverlay types, DecryptionFailed error"
```

- [ ] **Step 13: Merge and close**

```bash
git checkout master && git merge bole-abc
git branch -d bole-abc
bd close bole-abc
```

---

## Task 2: ObjectStore methods — `put_secret`, `get_secret`, `put_overlay`, `get_overlay`

**Files:**
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes (from Task 1):
  - `Object::Secret(Secret)` and `Object::EnvOverlay(EnvOverlay)` variants
  - `Secret::encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Self>`
  - `Secret::decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>>`
  - `EnvOverlay { pub entries: BTreeMap<String, EnvValue> }`
  - `Error::DecryptionFailed`
- Produces:
  - `pub async fn ObjectStore::put_secret(&self, plaintext: &[u8], key: &[u8; 32]) -> Result<ObjectId>`
  - `pub async fn ObjectStore::get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>>`
  - `pub async fn ObjectStore::put_overlay(&self, overlay: EnvOverlay) -> Result<ObjectId>`
  - `pub async fn ObjectStore::get_overlay(&self, id: &ObjectId) -> Result<Option<EnvOverlay>>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 4 T2: ObjectStore put_secret/get_secret/put_overlay/get_overlay" \
  --description="Add four convenience methods to ObjectStore: put_secret encrypts and stores, get_secret fetches and decrypts, put_overlay stores postcard-encoded EnvOverlay, get_overlay fetches and decodes." \
  --type=task --priority=2
bd update bole-def --claim
git checkout -b bole-def
```

- [ ] **Step 2: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/store/mod.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn put_secret_returns_stable_id_for_same_ciphertext_nonce() {
        // Two puts of the same Secret object (same nonce + ciphertext) → same id
        let s = store();
        let key = [1u8; 32];
        let id1 = s.put_secret(b"value", &key).await.unwrap();
        // Can't assert id1 == id2 with different puts (different nonces),
        // but can assert the stored object round-trips correctly
        let got = s.get_secret(&id1, &key).await.unwrap().unwrap();
        assert_eq!(got, b"value");
    }

    #[tokio::test]
    async fn get_secret_wrong_key_returns_err() {
        let s = store();
        let key = [1u8; 32];
        let wrong_key = [2u8; 32];
        let id = s.put_secret(b"secret", &key).await.unwrap();
        let err = s.get_secret(&id, &wrong_key).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::DecryptionFailed));
    }

    #[tokio::test]
    async fn get_secret_missing_returns_none() {
        let s = store();
        let key = [1u8; 32];
        let id = crate::object::ObjectId::new([9u8; 32]);
        let got = s.get_secret(&id, &key).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn put_overlay_get_overlay_roundtrip() {
        use crate::object::{EnvOverlay, EnvValue, ObjectId};
        use std::collections::BTreeMap;
        let s = store();
        let mut entries = BTreeMap::new();
        entries.insert("LOG".into(), EnvValue::Plain("info".into()));
        entries.insert("KEY".into(), EnvValue::Secret(ObjectId::new([7u8; 32])));
        let overlay = EnvOverlay { entries };
        let id = s.put_overlay(overlay.clone()).await.unwrap();
        let got = s.get_overlay(&id).await.unwrap().unwrap();
        assert_eq!(got, overlay);
    }

    #[tokio::test]
    async fn get_secret_on_non_secret_object_returns_err() {
        let s = store();
        let blob_id = s.put_blob(Bytes::from("not a secret")).await.unwrap();
        let key = [1u8; 32];
        let err = s.get_secret(&blob_id, &key).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Codec(_)));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test "put_secret\|get_secret\|put_overlay\|get_overlay" 2>&1 | head -20
```

Expected: compile errors — methods not found.

- [ ] **Step 4: Implement the four methods**

In `src/store/mod.rs`, add to the `impl ObjectStore` block after `pub async fn list`:

```rust
    // <bead-id>
    pub async fn put_secret(&self, plaintext: &[u8], key: &[u8; 32]) -> Result<ObjectId> {
        let secret = crate::object::Secret::encrypt(plaintext, key)?;
        self.put(&Object::Secret(secret)).await
    }

    pub async fn get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        match self.get(id).await? {
            None => Ok(None),
            Some(Object::Secret(s)) => Ok(Some(s.decrypt(key)?)),
            Some(_) => Err(crate::error::Error::Codec("not a secret".into())),
        }
    }

    pub async fn put_overlay(&self, overlay: crate::object::EnvOverlay) -> Result<ObjectId> {
        self.put(&Object::EnvOverlay(overlay)).await
    }

    pub async fn get_overlay(&self, id: &ObjectId) -> Result<Option<crate::object::EnvOverlay>> {
        match self.get(id).await? {
            None => Ok(None),
            Some(Object::EnvOverlay(o)) => Ok(Some(o)),
            Some(_) => Err(crate::error::Error::Codec("not an env overlay".into())),
        }
    }
```

Also update the imports at the top of `src/store/mod.rs` — add `Secret` and `EnvOverlay` to the existing object import:

```rust
use crate::object::{Blob, EnvOverlay, Object, ObjectId, Secret, Snapshot, Tree, TreeEntry};
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests pass plus the 5 new ObjectStore tests.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/store/mod.rs
git commit -m "feat(store): add put_secret, get_secret, put_overlay, get_overlay to ObjectStore"
```

- [ ] **Step 8: Merge and close**

```bash
git checkout master && git merge bole-def
git branch -d bole-def
bd close bole-def
```

---

## Task 3: `WorkspaceView` + `Repository::compute_workspace_view`

**Files:**
- Create: `src/repo/workspace.rs`
- Modify: `src/repo/mod.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes (from Tasks 1+2):
  - `ObjectStore::get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>>`
  - `ObjectStore::get_overlay(&self, id: &ObjectId) -> Result<Option<EnvOverlay>>`
  - `EnvOverlay { pub entries: BTreeMap<String, EnvValue> }`
  - `EnvValue::Plain(String)` and `EnvValue::Secret(ObjectId)`
  - `Repository::get_snapshot_filtered(id: ObjectId, accessor: &Accessor) -> Result<Option<FilteredSnapshot>>` (Gate 3, in `src/repo/mod.rs`)
  - `FilteredSnapshot { pub visible_paths: BTreeMap<String, ObjectId>, ... }` (Gate 3)
  - `Accessor` from `crate::acl`
  - `Error::SecretNotUtf8`, `Error::DecryptionFailed`
- Produces:
  - `pub struct WorkspaceView { pub files: BTreeMap<String, ObjectId>, pub env: BTreeMap<String, String> }`
  - `pub async fn Repository::compute_workspace_view(&self, snapshot_id: ObjectId, overlay_id: ObjectId, key: &[u8; 32], accessor: &Accessor) -> Result<Option<WorkspaceView>>`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 4 T3: WorkspaceView and compute_workspace_view" \
  --description="Create WorkspaceView struct. Implement Repository::compute_workspace_view: get_snapshot_filtered for ACL-filtered files, get_overlay for env map, resolve Plain values directly and Secret values via get_secret+UTF-8." \
  --type=task --priority=2
bd update bole-ghi --claim
git checkout -b bole-ghi
```

- [ ] **Step 2: Create `src/repo/workspace.rs`**

```rust
// <bead-id>
use crate::object::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceView {
    pub files: BTreeMap<String, ObjectId>,
    pub env: BTreeMap<String, String>,
}
```

- [ ] **Step 3: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/repo/mod.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn compute_workspace_view_resolves_env() {
        use crate::acl::{Accessor, PathRole, Permission};
        use crate::object::{EnvOverlay, EnvValue, Snapshot, TreeEntry, EntryKind};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [42u8; 32];

        // Build a snapshot with one public path
        let blob_id = repo.objects.put_blob(Bytes::from("code")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/main.rs".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(),
            created_at: 1, message: "m".into(),
        }).await.unwrap();

        // Store a secret and build an overlay
        let secret_id = repo.objects.put_secret(b"postgres://prod", &key).await.unwrap();
        let mut env_entries = BTreeMap::new();
        env_entries.insert("DB_URL".into(), EnvValue::Secret(secret_id));
        env_entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: env_entries }).await.unwrap();

        // Full accessor (can read all paths)
        let accessor = Accessor::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });

        let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &accessor)
            .await.unwrap().unwrap();

        assert!(view.files.contains_key("src/main.rs"));
        assert_eq!(view.env.get("DB_URL").map(String::as_str), Some("postgres://prod"));
        assert_eq!(view.env.get("LOG_LEVEL").map(String::as_str), Some("info"));
    }

    #[tokio::test]
    async fn compute_workspace_view_acl_filters_files() {
        use crate::acl::{Accessor, PathAcl};
        use crate::object::{EnvOverlay, Snapshot, TreeEntry, EntryKind};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [1u8; 32];

        // Build snapshot with two paths
        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        entries.insert("src/config.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "t".into(),
            created_at: 1, message: "m".into(),
        }).await.unwrap();

        // Protect config.rs
        repo.acls.set_path_acl(PathAcl { glob: "src/config.rs".into() }).unwrap();

        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: BTreeMap::new() }).await.unwrap();

        // Empty accessor — cannot read config.rs
        let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &Accessor::new())
            .await.unwrap().unwrap();

        assert!(view.files.contains_key("src/app.rs"));
        assert!(!view.files.contains_key("src/config.rs"));
        assert!(view.env.is_empty());
    }

    #[tokio::test]
    async fn compute_workspace_view_returns_none_for_missing_snapshot() {
        use crate::acl::Accessor;
        use crate::object::{EnvOverlay, ObjectId};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [1u8; 32];
        let missing = ObjectId::new([9u8; 32]);
        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: BTreeMap::new() }).await.unwrap();
        let result = repo.compute_workspace_view(missing, overlay_id, &key, &Accessor::new())
            .await.unwrap();
        assert!(result.is_none());
    }
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
cargo test compute_workspace_view 2>&1 | head -20
```

Expected: compile errors — `workspace` module not found, method not found.

- [ ] **Step 5: Wire `workspace` module into `src/repo/mod.rs`**

Add `pub mod workspace;` at the top of `src/repo/mod.rs` after `pub mod materialize;`:

```rust
// bole-1vi
pub mod materialize;
// <bead-id>
pub mod workspace;
```

Also add these imports to the use block at the top of `src/repo/mod.rs`:

```rust
// <bead-id>
use crate::object::{EnvValue, ObjectId};
use workspace::WorkspaceView;
```

- [ ] **Step 6: Implement `compute_workspace_view`**

Add to `impl Repository` in `src/repo/mod.rs`:

```rust
    // <bead-id>
    pub async fn compute_workspace_view(
        &self,
        snapshot_id: ObjectId,
        overlay_id: ObjectId,
        key: &[u8; 32],
        accessor: &Accessor,
    ) -> Result<Option<WorkspaceView>> {
        let filtered = match self.get_snapshot_filtered(snapshot_id, accessor).await? {
            Some(f) => f,
            None => return Ok(None),
        };
        let overlay = match self.objects.get_overlay(&overlay_id).await? {
            Some(o) => o,
            None => return Err(crate::error::Error::Storage(
                format!("overlay not found: {}", overlay_id)
            )),
        };
        let mut env = std::collections::BTreeMap::new();
        for (var, value) in overlay.entries {
            let resolved = match value {
                EnvValue::Plain(s) => s,
                EnvValue::Secret(id) => {
                    let bytes = self.objects.get_secret(&id, key).await?
                        .ok_or_else(|| crate::error::Error::Storage(
                            format!("secret not found: {}", id)
                        ))?;
                    String::from_utf8(bytes)
                        .map_err(|_| crate::error::Error::SecretNotUtf8)?
                }
            };
            env.insert(var, resolved);
        }
        Ok(Some(WorkspaceView { files: filtered.visible_paths, env }))
    }
```

- [ ] **Step 7: Add `WorkspaceView` re-export to `src/lib.rs`**

After `pub use repo::{FilteredSnapshot, MergeCheck};` add:

```rust
// <bead-id>
pub use repo::workspace::WorkspaceView;
```

- [ ] **Step 8: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all existing tests plus the 3 new `compute_workspace_view` tests pass.

- [ ] **Step 9: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add src/repo/workspace.rs src/repo/mod.rs src/lib.rs
git commit -m "feat(repo): add WorkspaceView and compute_workspace_view"
```

- [ ] **Step 11: Merge and close**

```bash
git checkout master && git merge bole-ghi
git branch -d bole-ghi
bd close bole-ghi
```

---

## Task 4: T4 Integration Tests

**Files:**
- Create: `tests/secrets.rs`

**Interfaces:**
- Consumes all public APIs from Tasks 1–3:
  - `bole::{Secret, EnvOverlay, EnvValue, WorkspaceView, Repository}`
  - `bole::{Accessor, PathAcl, PathRole, Permission}`
  - `bole::object::{EntryKind, ObjectId, Snapshot, TreeEntry}`
  - `bole::error::Error`
  - `bytes::Bytes`
  - `std::collections::BTreeMap`

- [ ] **Step 1: Create bead and branch**

```bash
bd create --title="Gate 4 T4: T4 integration tests" \
  --description="Create tests/secrets.rs with t4_secret_roundtrip, t4_workspace_view, t4_workspace_view_acl_filtered integration tests." \
  --type=task --priority=2
bd update bole-jkl --claim
git checkout -b bole-jkl
```

- [ ] **Step 2: Create `tests/secrets.rs`**

```rust
// <bead-id>
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::{
    Accessor, EnvOverlay, EnvValue, PathAcl, PathRole, Permission, Repository, WorkspaceView,
};
use bytes::Bytes;
use std::collections::BTreeMap;

fn key() -> [u8; 32] { [42u8; 32] }
fn wrong_key() -> [u8; 32] { [99u8; 32] }

/// T4: Secret encrypt/decrypt roundtrip via ObjectStore.
/// Verifies put_secret stores ciphertext and get_secret decrypts correctly.
/// Verifies wrong key returns DecryptionFailed.
#[tokio::test]
async fn t4_secret_roundtrip() {
    let repo = Repository::memory();
    let key = key();

    // Store and retrieve
    let id = repo.objects.put_secret(b"s3cr3t value", &key).await.unwrap();
    let got = repo.objects.get_secret(&id, &key).await.unwrap().unwrap();
    assert_eq!(got, b"s3cr3t value");

    // Wrong key fails
    let err = repo.objects.get_secret(&id, &wrong_key()).await.unwrap_err();
    assert!(
        matches!(err, bole::Error::DecryptionFailed),
        "expected DecryptionFailed, got {:?}", err
    );

    // Missing id returns None
    let missing = bole::ObjectId::new([0u8; 32]);
    let none = repo.objects.get_secret(&missing, &key).await.unwrap();
    assert!(none.is_none());
}

/// T4: compute_workspace_view resolves Plain and Secret EnvValues,
/// returns correct file set from the snapshot.
#[tokio::test]
async fn t4_workspace_view() {
    let repo = Repository::memory();
    let key = key();

    // Build snapshot with two paths
    let blob1 = repo.objects.put_blob(Bytes::from("app code")).await.unwrap();
    let blob2 = repo.objects.put_blob(Bytes::from("config code")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
    entries.insert("src/config.rs".into(), TreeEntry { id: blob2, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id,
        parents: vec![],
        author: "test".into(),
        created_at: 1,
        message: "initial".into(),
    }).await.unwrap();

    // Store a secret
    let secret_id = repo.objects.put_secret(b"postgres://prod", &key).await.unwrap();

    // Build overlay with one plain value and one secret reference
    let mut env_entries = BTreeMap::new();
    env_entries.insert("DB_URL".into(), EnvValue::Secret(secret_id));
    env_entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
    let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: env_entries }).await.unwrap();

    // Full accessor (** glob = can read all paths)
    let accessor = Accessor::new()
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });

    let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &accessor)
        .await
        .unwrap()
        .unwrap();

    // Files: both paths visible
    assert_eq!(view.files.len(), 2);
    assert!(view.files.contains_key("src/app.rs"));
    assert!(view.files.contains_key("src/config.rs"));

    // Env: both values resolved
    assert_eq!(view.env.get("DB_URL").map(String::as_str), Some("postgres://prod"));
    assert_eq!(view.env.get("LOG_LEVEL").map(String::as_str), Some("info"));
}

/// T4: compute_workspace_view ACL filters snapshot paths.
/// Protected paths are hidden from callers lacking the role.
/// Env resolves independently of snapshot path ACLs.
#[tokio::test]
async fn t4_workspace_view_acl_filtered() {
    let repo = Repository::memory();
    let key = key();

    // Protect src/config.rs
    repo.acls.set_path_acl(PathAcl { glob: "src/config.rs".into() }).unwrap();

    // Build snapshot with one public + one protected path
    let blob = repo.objects.put_blob(Bytes::from("content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
    entries.insert("src/config.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id,
        parents: vec![],
        author: "test".into(),
        created_at: 1,
        message: "m".into(),
    }).await.unwrap();

    // Secret in overlay
    let secret_id = repo.objects.put_secret(b"my-api-key", &key).await.unwrap();
    let mut env_entries = BTreeMap::new();
    env_entries.insert("API_KEY".into(), EnvValue::Secret(secret_id));
    env_entries.insert("MODE".into(), EnvValue::Plain("dev".into()));
    let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: env_entries }).await.unwrap();

    // Empty accessor: no path roles → cannot read src/config.rs
    let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &Accessor::new())
        .await
        .unwrap()
        .unwrap();

    // Only the public path is visible
    assert_eq!(view.files.len(), 1);
    assert!(view.files.contains_key("src/app.rs"));
    assert!(!view.files.contains_key("src/config.rs"));

    // Env resolves regardless of path ACLs
    assert_eq!(view.env.get("API_KEY").map(String::as_str), Some("my-api-key"));
    assert_eq!(view.env.get("MODE").map(String::as_str), Some("dev"));
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test --test secrets 2>&1 | tail -20
```

Expected: all 3 T4 tests pass.

- [ ] **Step 4: Run full test suite**

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
git add tests/secrets.rs
git commit -m "test(secrets): add T4 integration tests"
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
|---|---|
| `Secret` struct with `nonce: [u8; 12]`, `ciphertext: Vec<u8>` | Task 1 |
| `Secret::encrypt(plaintext, key) -> Result<Self>` (ChaCha20-Poly1305, random nonce) | Task 1 |
| `Secret::decrypt(&self, key) -> Result<Vec<u8>>` | Task 1 |
| `EnvValue::Plain(String)` and `EnvValue::Secret(ObjectId)` | Task 1 |
| `EnvOverlay { pub entries: BTreeMap<String, EnvValue> }` | Task 1 |
| `Object::Secret(Secret)` and `Object::EnvOverlay(EnvOverlay)` variants | Task 1 |
| `Error::DecryptionFailed` and `Error::SecretNotUtf8` | Task 1 |
| `chacha20poly1305 = "0.10"` and `rand = "0.8"` in Cargo.toml | Task 1 |
| `ObjectStore::put_secret(plaintext, key) -> Result<ObjectId>` | Task 2 |
| `ObjectStore::get_secret(id, key) -> Result<Option<Vec<u8>>>` | Task 2 |
| `ObjectStore::put_overlay(overlay) -> Result<ObjectId>` | Task 2 |
| `ObjectStore::get_overlay(id) -> Result<Option<EnvOverlay>>` | Task 2 |
| `get_secret` on non-Secret object returns `Err(Codec(_))` | Task 2 |
| `WorkspaceView { files: BTreeMap<String, ObjectId>, env: BTreeMap<String, String> }` | Task 3 |
| `Repository::compute_workspace_view(snap_id, overlay_id, key, accessor) -> Result<Option<WorkspaceView>>` | Task 3 |
| `None` returned when snapshot_id not found | Task 3 |
| ACL-filtered files via `get_snapshot_filtered` | Task 3 |
| `Plain` values inserted directly, `Secret` values decrypted + UTF-8 decoded | Task 3 |
| `WorkspaceView` re-exported from `lib.rs` | Task 3 |
| T4 `t4_secret_roundtrip` | Task 4 |
| T4 `t4_workspace_view` | Task 4 |
| T4 `t4_workspace_view_acl_filtered` | Task 4 |

**Placeholder scan:** None found.

**Type consistency:**
- `EnvValue::Secret(ObjectId)` — used in Task 1 definition, Task 2 match arm, Task 3 resolution, Task 4 tests ✓
- `put_secret(&self, plaintext: &[u8], key: &[u8; 32])` — matches all call sites ✓
- `get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>>` — matches Task 3 call `self.objects.get_secret(&id, key).await?` ✓
- `WorkspaceView { files, env }` — matches all assertions in Task 4 ✓
- `Error::SecretNotUtf8` — defined Task 1, used Task 3 `String::from_utf8` path ✓
- `Error::DecryptionFailed` — defined Task 1, returned by `Secret::decrypt`, tested Task 2 + Task 4 ✓
