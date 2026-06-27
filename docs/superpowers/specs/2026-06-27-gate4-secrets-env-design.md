# Gate 4: Secrets and Env Graph

**Project:** bole — a next-generation version control system  
**Language:** Rust (async, tokio)  
**Date:** 2026-06-27  
**Spec ref:** spec.md Gate 4, Test T4

---

## Context

Gates 1, 2, 3, and 5 delivered a content-addressed object store, mutable references, granular ACL enforcement, and pluggable backends. Gate 4 adds secrets and environment overlays as first-class typed objects: `Secret` stores encrypted bytes in the object store, `EnvOverlay` maps environment variable names to plaintext values or secret references, and `Repository::compute_workspace_view` composes a snapshot's ACL-filtered file tree with a resolved environment map into a single `WorkspaceView`.

Key architectural decisions:
- **New `Object` variants** — `Secret` and `EnvOverlay` join `Blob`, `Tree`, and `Snapshot` in the `Object` enum and travel through the same content-addressed, zstd-compressed store. No new storage abstraction.
- **Caller-supplied key** — the library never stores, logs, or manages encryption keys. `put_secret(plaintext, key)` and `get_secret(id, key)` are the only key-touching surfaces. Key rotation and storage are the caller's responsibility.
- **ChaCha20-Poly1305 encryption** — one new dependency (`chacha20poly1305 = "0.10"`). Random 12-byte nonce per secret, generated at put time. `ObjectId` is the BLAKE3 hash of the encoded ciphertext, not the plaintext.
- **Mixed env values** — `EnvValue` is either `Plain(String)` or `Secret(ObjectId)`. Plain values are stored in clear in the overlay; secret-typed values require the caller's key to resolve.

---

## Core Types

```rust
// src/object/secret.rs

use chacha20poly1305::{AeadInPlace, ChaCha20Poly1305, KeyInit, Nonce};
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Secret {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

impl Secret {
    pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Self> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| Error::Codec(e.to_string()))?;
        Ok(Self { nonce: nonce_bytes, ciphertext })
    }

    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce = Nonce::from(self.nonce);
        cipher
            .decrypt(&nonce, self.ciphertext.as_slice())
            .map_err(|_| Error::DecryptionFailed)
    }
}
```

```rust
// src/object/env.rs

use crate::object::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EnvValue {
    Plain(String),
    Secret(ObjectId),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EnvOverlay {
    pub entries: BTreeMap<String, EnvValue>,
}
```

```rust
// src/repo/workspace.rs

use crate::object::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceView {
    pub files: BTreeMap<String, ObjectId>,
    pub env: BTreeMap<String, String>,
}
```

---

## Object Enum Extension

```rust
// src/object/mod.rs

pub mod env;
pub mod secret;

pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Snapshot(Snapshot),
    Secret(Secret),        // new
    EnvOverlay(EnvOverlay), // new
}
```

`Secret` and `EnvOverlay` serialize via postcard (same codec as all other objects) and are stored zstd-compressed. The `ObjectId` of a `Secret` is the BLAKE3 hash of its encoded ciphertext — identical plaintext encrypted with different nonces produces different IDs, which is correct and avoids leaking equality of secret values.

---

## ObjectStore Methods

New convenience methods on `ObjectStore` in `src/store/mod.rs`:

```rust
impl ObjectStore {
    /// Encrypts `plaintext` with `key`, stores as Object::Secret, returns ObjectId.
    pub async fn put_secret(&self, plaintext: &[u8], key: &[u8; 32]) -> Result<ObjectId>;

    /// Fetches Object::Secret at `id`, decrypts with `key`.
    /// Returns None if the object does not exist.
    /// Returns Err(DecryptionFailed) if key is wrong or ciphertext is corrupt.
    pub async fn get_secret(&self, id: &ObjectId, key: &[u8; 32]) -> Result<Option<Vec<u8>>>;

    /// Postcard-encodes and stores an EnvOverlay, returns ObjectId.
    pub async fn put_overlay(&self, overlay: EnvOverlay) -> Result<ObjectId>;

    /// Fetches and decodes an EnvOverlay. Returns None if not found.
    pub async fn get_overlay(&self, id: &ObjectId) -> Result<Option<EnvOverlay>>;
}
```

`put_secret` calls `Secret::encrypt(plaintext, key)`, wraps in `Object::Secret`, and passes through the existing codec + backend path. Callers who fetch a raw `Object::Secret` via `get` receive the struct with ciphertext bytes — decryption requires an explicit `secret.decrypt(key)` call. The convenience `get_secret` handles this. If `get_secret` is called on an `ObjectId` that resolves to a non-`Secret` variant (e.g., a `Blob`), it returns `Err(Error::Codec("not a secret".into()))`.

---

## Repository API

```rust
// src/repo/mod.rs

impl Repository {
    /// Computes a workspace view: ACL-filtered snapshot files + resolved env overlay.
    ///
    /// Algorithm:
    /// 1. Call get_snapshot_filtered(snapshot_id, accessor) → visible_paths
    /// 2. Fetch EnvOverlay at overlay_id
    /// 3. For each overlay entry:
    ///    - Plain(s)     → insert s directly into env map
    ///    - Secret(id)   → get_secret(id, key) → interpret as UTF-8 → insert into env map
    /// 4. Return WorkspaceView { files: visible_paths, env }
    ///
    /// Returns None if snapshot_id does not resolve to a Snapshot object.
    pub async fn compute_workspace_view(
        &self,
        snapshot_id: ObjectId,
        overlay_id: ObjectId,
        key: &[u8; 32],
        accessor: &Accessor,
    ) -> Result<Option<WorkspaceView>>;
}
```

**ACL for secrets:** Secrets referenced in an `EnvOverlay` are identified by `ObjectId`, not by snapshot path, so the path ACL system does not apply to them directly. The overlay object itself can be protected by storing it behind a path-ACL-guarded snapshot entry (e.g., `".overlays/prod"`), relying on `get_snapshot_filtered` to hide it from unauthorized callers. Explicit overlay-level ACLs are out of scope for Gate 4; Gate 6 can add them.

---

## Error Extension

Two new variants in `src/error.rs`:

```rust
#[error("decryption failed")]
DecryptionFailed,

#[error("secret value is not valid UTF-8")]
SecretNotUtf8,
```

`DecryptionFailed` is returned when ChaCha20-Poly1305 authentication fails (wrong key, truncated ciphertext, or bit-flip). It intentionally contains no detail that would aid an attacker. `SecretNotUtf8` is returned when a secret's decrypted bytes cannot be interpreted as a UTF-8 string during env resolution.

---

## New Dependencies

```toml
# Cargo.toml [dependencies]
chacha20poly1305 = "0.10"
rand = "0.8"
```

`rand` is used for `rand::random::<[u8; 12]>()` nonce generation in `Secret::encrypt`. It may already be a transitive dep via `blake3`, but must be listed explicitly in `[dependencies]` — never rely on a transitive dep without pinning it.

---

## Crate Structure Changes

```
src/
├── object/
│   ├── mod.rs        # add Secret, EnvOverlay variants to Object enum
│   ├── secret.rs     # Secret struct + encrypt/decrypt
│   └── env.rs        # EnvValue enum + EnvOverlay struct
├── store/
│   └── mod.rs        # add put_secret, get_secret, put_overlay, get_overlay
├── repo/
│   ├── mod.rs        # add compute_workspace_view
│   └── workspace.rs  # WorkspaceView struct
├── error.rs          # add DecryptionFailed, SecretNotUtf8
└── lib.rs            # re-export Secret, EnvOverlay, EnvValue, WorkspaceView

tests/
└── secrets.rs        # T4 integration tests
```

---

## Testing Approach

### Unit tests (in-module)

**`src/object/secret.rs`:**
- `encrypt_decrypt_roundtrip` — encrypt bytes, decrypt with same key, assert equal
- `wrong_key_fails` — decrypt with a different `[u8; 32]` key returns `Err(DecryptionFailed)`
- `different_nonce_different_id` — same plaintext encrypted twice produces different `ObjectId`s (via `ObjectStore::put_secret` called twice)

**`src/object/env.rs`:**
- `overlay_serializes_roundtrip` — postcard encode/decode an `EnvOverlay` with both `Plain` and `Secret` variants, assert equal

### T4 integration tests (`tests/secrets.rs`)

**`t4_secret_roundtrip`:**
- `repo.objects.put_secret(b"s3cr3t", &key)` → `ObjectId`
- `repo.objects.get_secret(id, &key)` → `Some(b"s3cr3t")`
- `repo.objects.get_secret(id, &wrong_key)` → `Err(DecryptionFailed)`

**`t4_workspace_view`:**
- Build a snapshot with `src/app.rs` and `src/config.rs`
- `put_secret(b"postgres://prod", &key)` → `secret_id`
- `put_overlay(EnvOverlay { entries: { "DB_URL" → Secret(secret_id), "LOG_LEVEL" → Plain("info") } })` → `overlay_id`
- `compute_workspace_view(snap_id, overlay_id, &key, &Accessor::new().with_path_role(PathRole { glob: "**".into(), permission: Permission::Read }))`:
  - `view.files` contains both paths
  - `view.env["DB_URL"]` == `"postgres://prod"`
  - `view.env["LOG_LEVEL"]` == `"info"`

**`t4_workspace_view_acl_filtered`:**
- Same setup but add `PathAcl { glob: "src/config.rs" }` to `repo.acls`
- `Accessor::new()` (empty, no path roles) → `view.files` contains only `src/app.rs`
- `view.env` still resolves both values (overlay ACL is independent of snapshot path ACL)

---

## Out of Scope (Gate 4)

- Key rotation or re-encryption of existing secrets
- Audit logging of secret access
- Secret expiry or TTL
- Explicit ACL enforcement on `EnvOverlay` objects (Gate 6)
- Mounting multiple overlays on one snapshot
- Binary secret values in env (Gate 4 requires UTF-8 for env vars)
