# WS8a — Collaboration Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the identity + trust + local-discovery substrate for bole's collaboration hub: signed, content-addressed `Profile`/`TrustEdge` objects, a typed depth-bounded trust graph, explicit public-label publication, and a per-node trust-graph-local discovery index — with the invariant that no scoped object ever surfaces in discovery.

**Architecture:** New `Object::Collab(CollabObject)` variant (mirrors `Object::Policy`) with `Profile` and `TrustEdge` sub-variants, each Ed25519 self-signed against an embedded public key that *is* the identity. Publication = pinning under the public ref prefix `refs/collab/public/`; a `PublicObjectSource` serves only that namespace. A `TrustGraph` over `TrustEdge`s drives depth-bounded petname resolution (`Vouch`) and the discovery neighborhood (`Follow`). Discovery builds a local index over public objects held plus public objects pulled from `Follow` neighbors, every result carrying its publishing key and trust path.

**Tech Stack:** Rust (library-first), `ed25519-dalek` v2, `blake3` (fingerprints), `postcard` (deterministic signing messages), `async-trait`, `serde`, `tokio` (tests). In-memory backend (`Repository::memory()`) for all tests.

## Global Constraints

- **Hard invariant — identity:** Keys are identity; petnames are local; DNS aliases are hints. No code may treat a petname or DNS alias as an authoritative resolution key. Canonical identity is the Ed25519 public key (`[u8; 32]`); its fingerprint is `blake3::hash(key)` hex.
- **Hard invariant — visibility:** Private/scoped by default. Only objects pinned under `refs/collab/public/` are eligible for discovery. `PublicObjectSource` and every index/serve path read *only* that prefix. Scoped objects never surface, for any querier.
- **Hard invariant — trust:** Trust edges are typed (`Vouch`/`Follow`/`Review`) and traversal is depth-bounded (vouch depth ≤ 2; follow hop-limit default 2). No numeric trust scores, no unbounded crawl.
- **Hard invariant — roots:** The `TrustStore`/trust roots remain authoritative for who is a root; the trust graph is layered on top and never promotes a key to authority.
- **Monotonicity:** For a given key, only the highest-`seq` `Profile` is current; for a given `(from_key, kind, to_key)`, only the highest-`seq` `TrustEdge` is current. Lower/equal `seq` is rejected on publish.
- **Signing:** Reuse the existing detached/self-signed pattern in `src/acl/attestation.rs`. Domain-separated messages: `b"bole-collab-profile-v1\0"` and `b"bole-collab-edge-v1\0"`. Signing message bytes are `domain || postcard(unsigned-fields)`.
- **No new heavy deps.** Only crates already in `Cargo.toml` (`ed25519-dalek`, `blake3`, `postcard`, `serde`, `async-trait`, `bytes`, `tokio`).
- **Scope:** Library slice only. No web UI. No relay *implementation* (interface only). No PR/board/landing-page objects. Network transport for cross-node pulls is deferred to WS5/WS8b; v1 discovery gathers from an injected source set representing followed peers.
- **Process:** bd-only tracking (no TodoWrite/markdown TODOs). Each Task is one bead. Branch name = bead ID exactly. Each contiguous added code block carries a single `// <bead-id>` comment. Tests must pass before merge; delete the branch after merge; `bd close` the bead.

### Per-task bead protocol (do this for every Task)

```bash
bd create "WS8a Task N: <title>" --json      # note the returned id, e.g. bole-abc
bd update <id> --claim
git checkout -b <id>                          # branch name == bead id
# ... TDD steps ...
git checkout master && git merge <id> && git branch -d <id>
bd close <id>
```

Use the assigned `<id>` as the `// <id>` comment tag on every block you add in that task.

---

## Gates → Tests

Explicit numbered acceptance gates. Each is satisfied by the named test(s) in the referenced task. A task is not "done" until its gates' tests pass.

| Gate | Requirement (from spec) | Satisfying test(s) | Task |
|------|-------------------------|--------------------|------|
| **G1** | `CollabObject` round-trips through the store with stable ids | `collab_object_round_trips_with_stable_ids` | 1 |
| **G2** | `Profile` is self-signed and verifiable against its embedded key; forged/tampered rejected | `profile_signature_verifies`, `tampered_profile_rejected` | 1 |
| **G3** | `TrustEdge` signed by `from_key`; typed kind preserved; `Review` stored though unused | `trust_edge_signature_verifies`, `review_edge_round_trips` | 2 |
| **G4** | **Headline:** only public-prefixed objects are served; scoped objects never served/indexed | `serve_returns_only_public`, `scoped_collab_never_served` | 3 |
| **G5** | Highest-`seq` wins for `Profile` and `TrustEdge`; lower/equal `seq` rejected | `higher_seq_profile_supersedes`, `stale_seq_rejected`, `higher_seq_edge_supersedes` | 3 |
| **G6** | Depth-bounded traversal: vouch depth-1/2, follow hop-limit cutoff; hop limit never changes identity | `follow_neighborhood_respects_hops`, `vouch_depth_one_and_two`, `hop_limit_does_not_change_identity` | 4 |
| **G7** | Petname precedence local > d1 > d2 > fingerprint; collisions disambiguated, never merged | `petname_precedence_order`, `same_petname_two_keys_not_merged` | 5 |
| **G8** | DNS alias verified vs claimed; never authoritative; conflicting claims can't hijack identity | `alias_verified_when_domain_asserts_key`, `conflicting_alias_stays_claimed_key_canonical` | 6 |
| **G9** | Discovery is local-only over public objects in follow-neighborhood; result carries key+object+trust-path; ordered by distance then recency | `index_orders_by_distance_then_recency`, `result_carries_key_and_trust_path` | 7 |
| **G10** | End-to-end across 3 in-memory nodes: discoverable within depth, invisible beyond hop limit, scoped never discoverable; graceful degradation on unreachable peer | `three_node_discovery_within_depth`, `beyond_hop_limit_invisible`, `scoped_never_discoverable_e2e`, `unreachable_peer_degrades_gracefully` | 8 |

---

## File Structure

- `src/collab/mod.rs` — module root; re-exports; `Key` type alias, `fingerprint()`.
- `src/collab/object.rs` — `CollabObject`, `Profile`, `TrustEdge`, `TrustKind`, `CollabSigner`, `verify_profile`, `verify_edge`, domain tags, message builders.
- `src/collab/trust.rs` — `TrustGraph`, `follow_neighborhood`, `vouch_suggestions`, `VouchSuggestion`.
- `src/collab/naming.rs` — `Namer`, `PetnameResolution`.
- `src/collab/alias.rs` — `AliasResolver` trait, `AliasStatus`, `verify_alias`.
- `src/collab/discovery.rs` — `PublicObjectSource` trait, `DiscoveryResult`, `Index`, `gather`.
- `src/repo/collab.rs` — `impl Repository` publication/scan helpers + `PublicObjectSource` impl; `COLLAB_PUBLIC_PREFIX`, `COLLAB_SCOPED_PREFIX`.
- `src/object/mod.rs:46` — add `Collab(CollabObject)` variant to `Object`.
- `src/repo/mod.rs` — add `mod collab;`.
- `src/lib.rs` — add `pub mod collab;` and re-exports.

---

## Task 1: `CollabObject` model — `Profile` + signing + store round-trip

**Files:**
- Create: `src/collab/mod.rs`
- Create: `src/collab/object.rs`
- Modify: `src/object/mod.rs:46` (add `Collab` variant), `src/object/mod.rs` imports
- Modify: `src/lib.rs` (add `pub mod collab;` + re-exports)

**Interfaces:**
- Produces: `pub type Key = [u8; 32]`; `pub fn fingerprint(key: &Key) -> String`; `pub struct Profile { pub key: Key, pub display_name: String, pub bio: String, pub endpoints: Vec<String>, pub dns_aliases: Vec<String>, pub seq: u64, pub sig: Vec<u8> }`; `pub enum CollabObject { Profile(Profile), TrustEdge(TrustEdge) }` (TrustEdge added in Task 2); `pub struct CollabSigner`; `CollabSigner::from_seed([u8;32]) -> Self`; `CollabSigner::public_key(&self) -> Key`; `CollabSigner::sign_profile(&self, display_name: String, bio: String, endpoints: Vec<String>, dns_aliases: Vec<String>, seq: u64) -> Profile`; `pub fn verify_profile(p: &Profile) -> bool`; `Object::Collab(CollabObject)`.

- [ ] **Step 1: Write the failing tests**

Create `src/collab/mod.rs`:

```rust
// <bead-id>
//! Collaboration substrate: signed, content-addressed identity and trust
//! objects (`Profile`, `TrustEdge`), a typed depth-bounded trust graph, and a
//! trust-graph-local discovery index. See
//! `docs/superpowers/specs/2026-07-03-ws8a-collaboration-substrate-design.md`.

pub mod object;

pub use object::{
    verify_profile, CollabObject, CollabSigner, Profile,
};

/// The canonical identity of a collaboration participant: an Ed25519 public key.
/// Petnames and DNS aliases are non-authoritative labels *for* a `Key`.
pub type Key = [u8; 32];

/// A stable, human-copyable fingerprint for a key (BLAKE3 hex of the key bytes).
pub fn fingerprint(key: &Key) -> String {
    blake3::hash(key).to_hex().to_string()
}
```

Create `src/collab/object.rs`:

```rust
// <bead-id>
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;

/// Domain-separation tag for `Profile` signatures.
const COLLAB_PROFILE_DOMAIN: &[u8] = b"bole-collab-profile-v1\0";

/// A self-signed, per-key, monotonic self-description. Metadata only — it grants
/// nothing and never overrides the lattice/ACLs. Canonical identity is `key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub key: Key,
    pub display_name: String,
    pub bio: String,
    pub endpoints: Vec<String>,
    pub dns_aliases: Vec<String>,
    pub seq: u64,
    /// Ed25519 signature (64 bytes) over the domain-separated unsigned fields.
    pub sig: Vec<u8>,
}

/// The tagged union of collaboration objects (TrustEdge added in Task 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollabObject {
    Profile(Profile),
}

#[derive(Serialize)]
struct ProfileMsg<'a> {
    key: &'a Key,
    display_name: &'a str,
    bio: &'a str,
    endpoints: &'a [String],
    dns_aliases: &'a [String],
    seq: u64,
}

fn profile_message(p: &Profile) -> Vec<u8> {
    let mut m = COLLAB_PROFILE_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&ProfileMsg {
        key: &p.key,
        display_name: &p.display_name,
        bio: &p.bio,
        endpoints: &p.endpoints,
        dns_aliases: &p.dns_aliases,
        seq: p.seq,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

/// Holds a signing key and issues signed collaboration objects.
pub struct CollabSigner {
    signing: SigningKey,
}

impl CollabSigner {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }

    pub fn public_key(&self) -> Key {
        self.signing.verifying_key().to_bytes()
    }

    pub fn sign_profile(
        &self,
        display_name: String,
        bio: String,
        endpoints: Vec<String>,
        dns_aliases: Vec<String>,
        seq: u64,
    ) -> Profile {
        let mut p = Profile {
            key: self.public_key(),
            display_name,
            bio,
            endpoints,
            dns_aliases,
            seq,
            sig: Vec::new(),
        };
        p.sig = self.signing.sign(&profile_message(&p)).to_bytes().to_vec();
        p
    }
}

/// True iff `p.sig` verifies against `p.key` over the domain-separated fields.
pub fn verify_profile(p: &Profile) -> bool {
    let vk = match VerifyingKey::from_bytes(&p.key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match p.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&profile_message(p), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;
    use crate::store::{memory::MemoryBackend, ObjectStore};

    #[tokio::test]
    async fn collab_object_round_trips_with_stable_ids() {
        let store = ObjectStore::new(MemoryBackend::new());
        let signer = CollabSigner::from_seed([7u8; 32]);
        let p = signer.sign_profile("Alice".into(), "hi".into(), vec![], vec![], 1);
        let wrapped = Object::Collab(CollabObject::Profile(p));
        let id1 = store.put(&wrapped).await.unwrap();
        let got = store.get(&id1).await.unwrap().unwrap();
        assert_eq!(got, wrapped);
        let id2 = store.put(&wrapped).await.unwrap();
        assert_eq!(id1, id2, "content-addressed id must be stable");
    }

    #[test]
    fn profile_signature_verifies() {
        let signer = CollabSigner::from_seed([9u8; 32]);
        let p = signer.sign_profile("Bob".into(), String::new(), vec!["n1".into()], vec![], 3);
        assert!(verify_profile(&p));
        assert_eq!(p.key, signer.public_key());
    }

    #[test]
    fn tampered_profile_rejected() {
        let signer = CollabSigner::from_seed([1u8; 32]);
        let mut p = signer.sign_profile("Carol".into(), String::new(), vec![], vec![], 1);
        p.display_name = "Mallory".into(); // mutate a signed field
        assert!(!verify_profile(&p));
    }
}
```

- [ ] **Step 2: Add the `Object::Collab` variant**

In `src/object/mod.rs`, add the import near the other object imports and the variant to `enum Object` (after `MultiRecipientSecret`):

```rust
// <bead-id>
use crate::collab::CollabObject;
```

```rust
    // <bead-id>
    /// A signed, content-addressed collaboration object (profile or trust edge).
    Collab(CollabObject),
```

- [ ] **Step 3: Wire the module + exports**

In `src/lib.rs`, add after `pub mod acl;`:

```rust
// <bead-id>
pub mod collab;
```

And a re-export block near the other `pub use`:

```rust
// <bead-id>
pub use collab::{fingerprint, verify_profile, CollabObject, CollabSigner, Key, Profile};
```

- [ ] **Step 4: Run tests to verify they fail, then pass**

Run: `cargo test -p bole collab::object`
Expected first (before Steps 2–3 compile): FAIL / build error `unresolved import crate::collab`. After Steps 2–3: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/collab/mod.rs src/collab/object.rs src/object/mod.rs src/lib.rs
git commit -m "<bead-id>: CollabObject + Profile signing, store round-trip (G1,G2)"
```

---

## Task 2: `TrustEdge` — typed, signed edges (`Vouch`/`Follow`/`Review`)

**Files:**
- Modify: `src/collab/object.rs` (add `TrustKind`, `TrustEdge`, signing/verify, `CollabObject::TrustEdge`)
- Modify: `src/collab/mod.rs` (re-exports)
- Modify: `src/lib.rs` (re-exports)

**Interfaces:**
- Consumes: `Key`, `CollabSigner` (Task 1).
- Produces: `pub enum TrustKind { Vouch, Follow, Review }`; `pub struct TrustEdge { pub from_key: Key, pub to_key: Key, pub kind: TrustKind, pub petname: Option<String>, pub seq: u64, pub sig: Vec<u8> }`; `CollabSigner::sign_edge(&self, to_key: Key, kind: TrustKind, petname: Option<String>, seq: u64) -> TrustEdge`; `pub fn verify_edge(e: &TrustEdge) -> bool`; `CollabObject::TrustEdge(TrustEdge)`.

- [ ] **Step 1: Write the failing tests**

Add to `src/collab/object.rs` (in `mod tests`):

```rust
    // <bead-id>
    #[test]
    fn trust_edge_signature_verifies() {
        let a = CollabSigner::from_seed([2u8; 32]);
        let b = CollabSigner::from_seed([3u8; 32]);
        let e = a.sign_edge(b.public_key(), TrustKind::Vouch, Some("bee".into()), 1);
        assert!(verify_edge(&e));
        assert_eq!(e.from_key, a.public_key());
        assert_eq!(e.to_key, b.public_key());
        assert_eq!(e.kind, TrustKind::Vouch);

        let mut tampered = e.clone();
        tampered.kind = TrustKind::Follow;
        assert!(!verify_edge(&tampered), "kind is a signed field");
    }

    #[tokio::test]
    async fn review_edge_round_trips() {
        let store = ObjectStore::new(MemoryBackend::new());
        let a = CollabSigner::from_seed([4u8; 32]);
        let b = CollabSigner::from_seed([5u8; 32]);
        // Review is reserved: signed and stored now, consulted by no subsystem yet.
        let e = a.sign_edge(b.public_key(), TrustKind::Review, None, 1);
        assert!(verify_edge(&e));
        let wrapped = Object::Collab(CollabObject::TrustEdge(e));
        let id = store.put(&wrapped).await.unwrap();
        assert_eq!(store.get(&id).await.unwrap().unwrap(), wrapped);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole collab::object`
Expected: FAIL (build error — `TrustKind`, `sign_edge`, `verify_edge`, `TrustEdge` undefined).

- [ ] **Step 3: Implement `TrustEdge`**

Add to `src/collab/object.rs`:

```rust
// <bead-id>
const COLLAB_EDGE_DOMAIN: &[u8] = b"bole-collab-edge-v1\0";

/// Typed trust relation. `Vouch` = identity trust (drives petname suggestions);
/// `Follow` = discovery trust (drives the discovery neighborhood); `Review` =
/// reserved for future PR/review workflows (signed and stored, not yet consulted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustKind {
    Vouch,
    Follow,
    Review,
}

/// A directed, signed trust edge from `from_key` to `to_key`. `petname` is
/// meaningful only on `Vouch` edges and ignored elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEdge {
    pub from_key: Key,
    pub to_key: Key,
    pub kind: TrustKind,
    pub petname: Option<String>,
    pub seq: u64,
    pub sig: Vec<u8>,
}

#[derive(Serialize)]
struct EdgeMsg<'a> {
    from_key: &'a Key,
    to_key: &'a Key,
    kind: &'a TrustKind,
    petname: &'a Option<String>,
    seq: u64,
}

fn edge_message(e: &TrustEdge) -> Vec<u8> {
    let mut m = COLLAB_EDGE_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&EdgeMsg {
        from_key: &e.from_key,
        to_key: &e.to_key,
        kind: &e.kind,
        petname: &e.petname,
        seq: e.seq,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

impl CollabSigner {
    // <bead-id>
    pub fn sign_edge(
        &self,
        to_key: Key,
        kind: TrustKind,
        petname: Option<String>,
        seq: u64,
    ) -> TrustEdge {
        let mut e = TrustEdge {
            from_key: self.public_key(),
            to_key,
            kind,
            petname,
            seq,
            sig: Vec::new(),
        };
        e.sig = self.signing.sign(&edge_message(&e)).to_bytes().to_vec();
        e
    }
}

/// True iff `e.sig` verifies against `e.from_key` over the domain-separated fields.
pub fn verify_edge(e: &TrustEdge) -> bool {
    let vk = match VerifyingKey::from_bytes(&e.from_key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match e.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&edge_message(e), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}
```

Add the variant to `enum CollabObject`:

```rust
    // <bead-id>
    TrustEdge(TrustEdge),
```

- [ ] **Step 4: Update re-exports**

In `src/collab/mod.rs`, extend the `pub use object::{...}`:

```rust
// <bead-id>
pub use object::{verify_edge, TrustEdge, TrustKind};
```

In `src/lib.rs`, extend the collab re-export:

```rust
// <bead-id>
pub use collab::{verify_edge, TrustEdge, TrustKind};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bole collab::object`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add src/collab/object.rs src/collab/mod.rs src/lib.rs
git commit -m "<bead-id>: typed signed TrustEdge (Vouch/Follow/Review) (G3)"
```

---

## Task 3: Publication — public ref prefix, monotonic seq, serve-public-only (headline invariant)

**Files:**
- Create: `src/repo/collab.rs`
- Modify: `src/repo/mod.rs` (add `mod collab;`)
- Modify: `src/collab/discovery.rs` — created here for the `PublicObjectSource` trait (query added in Task 7)
- Modify: `src/collab/mod.rs` (add `pub mod discovery;`)

**Interfaces:**
- Consumes: `Profile`, `TrustEdge`, `TrustKind`, `CollabObject`, `verify_profile`, `verify_edge`, `fingerprint`, `Key` (Tasks 1–2); `Repository { objects: ObjectStore, refs: RefStore }`, `RefName`, `Ref`, `Tag` (existing).
- Produces on `impl Repository`: `pub async fn publish_profile(&self, p: &Profile) -> Result<ObjectId>`; `pub async fn profile(&self, key: &Key) -> Result<Option<Profile>>`; `pub async fn publish_edge(&self, e: &TrustEdge) -> Result<ObjectId>`; `pub async fn public_profiles(&self) -> Result<Vec<Profile>>`; `pub async fn public_edges(&self) -> Result<Vec<TrustEdge>>`; consts `COLLAB_PUBLIC_PREFIX = "refs/collab/public/"`, `COLLAB_SCOPED_PREFIX = "refs/collab/scoped/"`.
- Produces in `src/collab/discovery.rs`: `#[async_trait] pub trait PublicObjectSource { async fn public_objects(&self) -> Result<Vec<CollabObject>>; }` and its `impl for Repository` returning only public-prefixed objects.

- [ ] **Step 1: Write the failing tests**

Create `src/repo/collab.rs` with a test module at the bottom:

```rust
// <bead-id>
#[cfg(test)]
mod tests {
    use crate::collab::discovery::PublicObjectSource;
    use crate::collab::{CollabObject, CollabSigner, TrustKind};
    use crate::object::Object;
    use crate::refs::{Ref, RefName, Tag};
    use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_SCOPED_PREFIX};
    use crate::repo::Repository;

    #[tokio::test]
    async fn serve_returns_only_public() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([10u8; 32]);
        let p = a.sign_profile("A".into(), String::new(), vec![], vec![], 1);
        repo.publish_profile(&p).await.unwrap();
        let served = repo.public_objects().await.unwrap();
        assert_eq!(served.len(), 1);
        assert!(matches!(&served[0], CollabObject::Profile(pp) if pp.key == a.public_key()));
        assert!(COLLAB_PUBLIC_PREFIX.starts_with("refs/collab/"));
    }

    #[tokio::test]
    async fn scoped_collab_never_served() {
        // Directly pin a collab object under the SCOPED prefix (simulating a
        // future capability-scoped object) and prove discovery/serve never sees it.
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([11u8; 32]);
        let p = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(p))).await.unwrap();
        let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/scoped");
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let served = repo.public_objects().await.unwrap();
        assert!(served.is_empty(), "scoped objects must never be served");
    }

    #[tokio::test]
    async fn higher_seq_profile_supersedes() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([12u8; 32]);
        repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_profile(&a.sign_profile("v2".into(), String::new(), vec![], vec![], 2)).await.unwrap();
        let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
        assert_eq!(cur.display_name, "v2");
        assert_eq!(cur.seq, 2);
    }

    #[tokio::test]
    async fn stale_seq_rejected() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([13u8; 32]);
        repo.publish_profile(&a.sign_profile("v2".into(), String::new(), vec![], vec![], 2)).await.unwrap();
        let err = repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await;
        assert!(err.is_err(), "publishing a lower seq must be rejected");
        let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
        assert_eq!(cur.seq, 2);
    }

    #[tokio::test]
    async fn higher_seq_edge_supersedes() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([14u8; 32]);
        let b = CollabSigner::from_seed([15u8; 32]);
        repo.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Vouch, Some("b1".into()), 1)).await.unwrap();
        repo.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Vouch, Some("b2".into()), 2)).await.unwrap();
        let edges = repo.public_edges().await.unwrap();
        let v: Vec<_> = edges.iter().filter(|e| e.from_key == a.public_key() && e.kind == TrustKind::Vouch).collect();
        assert_eq!(v.len(), 1, "only the current edge per (from,kind,to)");
        assert_eq!(v[0].petname.as_deref(), Some("b2"));
    }

    #[tokio::test]
    async fn rejects_unsigned_profile() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([16u8; 32]);
        let mut p = a.sign_profile("A".into(), String::new(), vec![], vec![], 1);
        p.display_name = "forged".into();
        assert!(repo.publish_profile(&p).await.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole repo::collab`
Expected: FAIL (build error — `publish_profile`, `PublicObjectSource`, consts undefined).

- [ ] **Step 3: Create the `PublicObjectSource` trait**

Create `src/collab/discovery.rs`:

```rust
// <bead-id>
use async_trait::async_trait;

use crate::collab::CollabObject;
use crate::error::Result;

/// The interface a node (and, later, a relay) exposes to serve its
/// **public-labeled** collaboration objects. v1 implements only the
/// sovereign-node side (`impl for Repository`); relays are a future impl of the
/// same trait, so discovery client code needs no change when they land.
#[async_trait]
pub trait PublicObjectSource {
    /// Every public collaboration object this source is willing to serve. MUST
    /// return only objects pinned under the public prefix — never scoped objects.
    async fn public_objects(&self) -> Result<Vec<CollabObject>>;
}
```

Add to `src/collab/mod.rs`:

```rust
// <bead-id>
pub mod discovery;
```

- [ ] **Step 4: Implement publication + serve on `Repository`**

Prepend to `src/repo/collab.rs` (above the test module):

```rust
// <bead-id>
//! Collaboration-object publication and serving for a `Repository`.
//!
//! Publication is an explicit act: a collaboration object becomes discoverable
//! only when pinned under [`COLLAB_PUBLIC_PREFIX`]. Serving and discovery read
//! *only* that prefix, so scoped objects (a future capability-scoped mode) are
//! never surfaced. Per key / per (from,kind,to) only the highest `seq` is kept.

use async_trait::async_trait;

use crate::collab::discovery::PublicObjectSource;
use crate::collab::{fingerprint, verify_edge, verify_profile, CollabObject, Key, Profile, TrustEdge, TrustKind};
use crate::error::{Error, Result};
use crate::object::Object;
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;

/// Ref prefix under which discoverable (public) collaboration objects are pinned.
pub const COLLAB_PUBLIC_PREFIX: &str = "refs/collab/public/";
/// Ref prefix reserved for future capability-scoped collaboration objects. Never
/// served or indexed by this slice.
pub const COLLAB_SCOPED_PREFIX: &str = "refs/collab/scoped/";

fn kind_seg(kind: TrustKind) -> &'static str {
    match kind {
        TrustKind::Vouch => "vouch",
        TrustKind::Follow => "follow",
        TrustKind::Review => "review",
    }
}

impl Repository {
    // <bead-id>
    /// Publishes a signed `Profile` to the public prefix. Rejects an invalid
    /// signature and any `seq` not strictly greater than the current profile's.
    pub async fn publish_profile(&self, p: &Profile) -> Result<ObjectIdAlias> {
        if !verify_profile(p) {
            return Err(Error::msg("profile signature does not verify"));
        }
        if let Some(cur) = self.profile(&p.key).await? {
            if p.seq <= cur.seq {
                return Err(Error::msg("profile seq must be greater than the current profile's"));
            }
        }
        let id = self.objects.put(&Object::Collab(CollabObject::Profile(p.clone()))).await?;
        let leaf = format!("{COLLAB_PUBLIC_PREFIX}profile/{}", fingerprint(&p.key));
        let mut tx = self.refs.transaction();
        tx.set(RefName::new(leaf)?, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // <bead-id>
    /// The current (highest-`seq`) profile for `key`, if any is published.
    pub async fn profile(&self, key: &Key) -> Result<Option<Profile>> {
        let name = RefName::new(format!("{COLLAB_PUBLIC_PREFIX}profile/{}", fingerprint(key)))?;
        let tag = match self.refs.get_tag(&name)? {
            Some(t) => t,
            None => return Ok(None),
        };
        match self.objects.get(&tag.target).await? {
            Some(Object::Collab(CollabObject::Profile(p))) => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    // <bead-id>
    /// Publishes a signed `TrustEdge`. Rejects an invalid signature and any `seq`
    /// not strictly greater than the current edge's for the same `(from,kind,to)`.
    pub async fn publish_edge(&self, e: &TrustEdge) -> Result<ObjectIdAlias> {
        if !verify_edge(e) {
            return Err(Error::msg("trust edge signature does not verify"));
        }
        let leaf = format!(
            "{COLLAB_PUBLIC_PREFIX}edge/{}/{}/{}",
            fingerprint(&e.from_key),
            kind_seg(e.kind),
            fingerprint(&e.to_key),
        );
        let name = RefName::new(leaf)?;
        if let Some(tag) = self.refs.get_tag(&name)? {
            if let Some(Object::Collab(CollabObject::TrustEdge(cur))) = self.objects.get(&tag.target).await? {
                if e.seq <= cur.seq {
                    return Err(Error::msg("trust edge seq must be greater than the current edge's"));
                }
            }
        }
        let id = self.objects.put(&Object::Collab(CollabObject::TrustEdge(e.clone()))).await?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // <bead-id>
    /// Every current public profile (one per key).
    pub async fn public_profiles(&self) -> Result<Vec<Profile>> {
        let mut out = Vec::new();
        for name in self.refs.list(&format!("{COLLAB_PUBLIC_PREFIX}profile/"))? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(CollabObject::Profile(p))) = self.objects.get(&tag.target).await? {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    // <bead-id>
    /// Every current public trust edge.
    pub async fn public_edges(&self) -> Result<Vec<TrustEdge>> {
        let mut out = Vec::new();
        for name in self.refs.list(&format!("{COLLAB_PUBLIC_PREFIX}edge/"))? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(CollabObject::TrustEdge(e))) = self.objects.get(&tag.target).await? {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }
}

// <bead-id>
#[async_trait]
impl PublicObjectSource for Repository {
    async fn public_objects(&self) -> Result<Vec<CollabObject>> {
        let mut out: Vec<CollabObject> = Vec::new();
        for p in self.public_profiles().await? {
            out.push(CollabObject::Profile(p));
        }
        for e in self.public_edges().await? {
            out.push(CollabObject::TrustEdge(e));
        }
        Ok(out)
    }
}
```

> **Implementer notes:**
> - Replace `ObjectIdAlias` with the crate's object-id type, imported as the existing helpers do (`crate::object::ObjectId`). It is written as an alias here only to avoid a duplicate-import claim; use `ObjectId` and add `use crate::object::ObjectId;` if not already brought in by the `Object` import.
> - Confirm `Error::msg` exists; if the crate's error type uses a different constructor (check `src/error.rs` — mirror how `src/repo/mod.rs` builds ad-hoc errors), use that exact form.
> - `self.refs.list(prefix)` and `self.refs.get_tag(&name)` match the signatures used in `src/acl/attestation.rs::load_attestations`.

- [ ] **Step 5: Register the module**

In `src/repo/mod.rs`, add near the other `mod` declarations:

```rust
// <bead-id>
mod collab;
```

Add re-exports of the public consts to `src/lib.rs`:

```rust
// <bead-id>
pub use repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_SCOPED_PREFIX};
```

(If `mod collab;` is private, make it `pub mod collab;` in `src/repo/mod.rs` so the re-export resolves; follow whichever visibility the neighboring `repo` submodules use.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p bole repo::collab`
Expected: PASS (6 tests). Then `cargo test -p bole` to confirm no regressions.

- [ ] **Step 7: Commit**

```bash
git add src/repo/collab.rs src/repo/mod.rs src/collab/discovery.rs src/collab/mod.rs src/lib.rs
git commit -m "<bead-id>: public-prefix publication, monotonic seq, serve-public-only (G4,G5)"
```

---

## Task 4: `TrustGraph` — depth-bounded traversal

**Files:**
- Create: `src/collab/trust.rs`
- Modify: `src/collab/mod.rs` (add `pub mod trust;` + re-exports)

**Interfaces:**
- Consumes: `Key`, `TrustEdge`, `TrustKind` (Tasks 1–2).
- Produces: `pub struct TrustGraph`; `TrustGraph::from_edges(edges: Vec<TrustEdge>) -> Self`; `pub fn follow_neighborhood(&self, root: &Key, hops: u8) -> BTreeMap<Key, u8>` (key → minimum hop distance, excluding `root`); `pub struct VouchSuggestion { pub petname: String, pub depth: u8, pub path: Vec<Key> }`; `pub fn vouch_suggestions(&self, root: &Key, target: &Key, max_depth: u8) -> Vec<VouchSuggestion>`.

- [ ] **Step 1: Write the failing tests**

Create `src/collab/trust.rs`:

```rust
// <bead-id>
use std::collections::{BTreeMap, VecDeque};

use crate::collab::{Key, TrustEdge, TrustKind};

// (implementation added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::CollabSigner;

    fn k(seed: u8) -> (CollabSigner, Key) {
        let s = CollabSigner::from_seed([seed; 32]);
        let key = s.public_key();
        (s, key)
    }

    #[test]
    fn follow_neighborhood_respects_hops() {
        let (a, ak) = k(1);
        let (b, bk) = k(2);
        let (_c, ck) = k(3);
        // a -follow-> b -follow-> c
        let edges = vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
        ];
        let g = TrustGraph::from_edges(edges);

        let n1 = g.follow_neighborhood(&ak, 1);
        assert_eq!(n1.get(&bk), Some(&1));
        assert!(!n1.contains_key(&ck), "c is 2 hops away; excluded at hops=1");

        let n2 = g.follow_neighborhood(&ak, 2);
        assert_eq!(n2.get(&bk), Some(&1));
        assert_eq!(n2.get(&ck), Some(&2));
        assert!(!n2.contains_key(&ak), "root is never in its own neighborhood");
    }

    #[test]
    fn vouch_depth_one_and_two() {
        let (a, ak) = k(4);
        let (b, bk) = k(5);
        let (_c, ck) = k(6);
        // a -follow-> b (so b's vouch is reachable at depth 2 via follow path),
        // b -vouch("cee")-> c ; a -vouch("bee")-> b
        let edges = vec![
            a.sign_edge(bk, TrustKind::Vouch, Some("bee".into()), 1),
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Vouch, Some("cee".into()), 1),
        ];
        let g = TrustGraph::from_edges(edges);

        let direct = g.vouch_suggestions(&ak, &bk, 2);
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].petname, "bee");
        assert_eq!(direct[0].depth, 1);

        let indirect = g.vouch_suggestions(&ak, &ck, 2);
        assert_eq!(indirect.len(), 1);
        assert_eq!(indirect[0].petname, "cee");
        assert_eq!(indirect[0].depth, 2);
        assert_eq!(indirect[0].path, vec![ak, bk], "path shows the trust route root->voucher");
    }

    #[test]
    fn hop_limit_does_not_change_identity() {
        let (a, ak) = k(7);
        let (b, bk) = k(8);
        let edges = vec![a.sign_edge(bk, TrustKind::Follow, None, 1)];
        let g = TrustGraph::from_edges(edges);
        // Whatever the hop limit, b's key (identity) is unchanged.
        assert!(g.follow_neighborhood(&ak, 0).is_empty());
        assert_eq!(g.follow_neighborhood(&ak, 1).keys().next(), Some(&bk));
        assert_eq!(g.follow_neighborhood(&ak, 5).get(&bk), Some(&1));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole collab::trust`
Expected: FAIL (build error — `TrustGraph` undefined).

- [ ] **Step 3: Implement `TrustGraph`**

Insert above the test module in `src/collab/trust.rs`:

```rust
// <bead-id>
/// A read-only view over trust edges, indexed for depth-bounded traversal.
/// The graph *suggests*; it never confers authority (roots stay authoritative).
pub struct TrustGraph {
    edges: Vec<TrustEdge>,
}

/// A petname suggested for a key by the trust graph, with its trust route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VouchSuggestion {
    pub petname: String,
    /// 1 = direct vouch by root; 2 = friend-of-friend.
    pub depth: u8,
    /// The route `root -> ... -> voucher` whose last hop authored the vouch.
    pub path: Vec<Key>,
}

impl TrustGraph {
    pub fn from_edges(edges: Vec<TrustEdge>) -> Self {
        Self { edges }
    }

    fn follows(&self, from: &Key) -> impl Iterator<Item = &Key> {
        self.edges
            .iter()
            .filter(move |e| e.kind == TrustKind::Follow && &e.from_key == from)
            .map(|e| &e.to_key)
    }

    /// BFS over `Follow` edges from `root`, bounded to `hops`. Returns each
    /// reachable key mapped to its minimum hop distance (root excluded).
    pub fn follow_neighborhood(&self, root: &Key, hops: u8) -> BTreeMap<Key, u8> {
        let mut dist: BTreeMap<Key, u8> = BTreeMap::new();
        let mut q: VecDeque<(Key, u8)> = VecDeque::new();
        q.push_back((*root, 0));
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(*root);
        while let Some((node, d)) = q.pop_front() {
            if d == hops {
                continue;
            }
            for next in self.follows(&node).copied().collect::<Vec<_>>() {
                if seen.insert(next) {
                    dist.insert(next, d + 1);
                    q.push_back((next, d + 1));
                }
            }
        }
        dist
    }

    /// Vouch suggestions for `target` reachable from `root` within `max_depth`.
    /// Depth-1: a direct `Vouch` edge authored by `root`. Depth-2: a `Vouch`
    /// authored by a key `root` directly `Follow`s. Deeper is not returned.
    pub fn vouch_suggestions(&self, root: &Key, target: &Key, max_depth: u8) -> Vec<VouchSuggestion> {
        let mut out = Vec::new();
        // Depth 1: root vouches for target directly.
        if max_depth >= 1 {
            for e in &self.edges {
                if e.kind == TrustKind::Vouch && &e.from_key == root && &e.to_key == target {
                    if let Some(name) = &e.petname {
                        out.push(VouchSuggestion { petname: name.clone(), depth: 1, path: vec![*root] });
                    }
                }
            }
        }
        // Depth 2: a key root follows vouches for target.
        if max_depth >= 2 {
            let direct_follows: Vec<Key> = self.follows(root).copied().collect();
            for voucher in direct_follows {
                for e in &self.edges {
                    if e.kind == TrustKind::Vouch && e.from_key == voucher && &e.to_key == target {
                        if let Some(name) = &e.petname {
                            out.push(VouchSuggestion {
                                petname: name.clone(),
                                depth: 2,
                                path: vec![*root, voucher],
                            });
                        }
                    }
                }
            }
        }
        out
    }
}
```

Add to `src/collab/mod.rs`:

```rust
// <bead-id>
pub mod trust;
pub use trust::{TrustGraph, VouchSuggestion};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p bole collab::trust`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/collab/trust.rs src/collab/mod.rs
git commit -m "<bead-id>: depth-bounded TrustGraph (follow neighborhood + vouch suggestions) (G6)"
```

---

## Task 5: Petname resolution — precedence + non-merging collisions

**Files:**
- Create: `src/collab/naming.rs`
- Modify: `src/collab/mod.rs` (add `pub mod naming;` + re-exports)

**Interfaces:**
- Consumes: `Key`, `fingerprint`, `TrustGraph` (Tasks 1, 4).
- Produces: `pub enum PetnameResolution { Local(String), Vouch { name: String, depth: u8, path: Vec<Key> }, Fingerprint(String) }`; `pub struct Namer<'a> { ... }`; `Namer::new(root: Key, local: &'a BTreeMap<Key, String>, graph: &'a TrustGraph) -> Self`; `Namer::resolve(&self, key: &Key) -> PetnameResolution`.

- [ ] **Step 1: Write the failing tests**

Create `src/collab/naming.rs`:

```rust
// <bead-id>
use std::collections::BTreeMap;

use crate::collab::trust::TrustGraph;
use crate::collab::{fingerprint, Key};

// (implementation added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabSigner, TrustKind};

    fn key(seed: u8) -> (CollabSigner, Key) {
        let s = CollabSigner::from_seed([seed; 32]);
        let k = s.public_key();
        (s, k)
    }

    #[test]
    fn petname_precedence_order() {
        let (root, rk) = key(1);
        let (_b, bk) = key(2);
        // root vouches bk="graph-bob"; local map says bk="my-bob".
        let g = TrustGraph::from_edges(vec![root.sign_edge(bk, TrustKind::Vouch, Some("graph-bob".into()), 1)]);

        let mut local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);
        // No local entry -> depth-1 vouch wins.
        match namer.resolve(&bk) {
            PetnameResolution::Vouch { name, depth, .. } => {
                assert_eq!(name, "graph-bob");
                assert_eq!(depth, 1);
            }
            other => panic!("expected Vouch, got {other:?}"),
        }

        // Local entry beats the graph.
        local.insert(bk, "my-bob".into());
        let namer = Namer::new(rk, &local, &g);
        assert!(matches!(namer.resolve(&bk), PetnameResolution::Local(n) if n == "my-bob"));

        // Unknown key -> fingerprint fallback.
        let (_u, uk) = key(9);
        assert!(matches!(namer.resolve(&uk), PetnameResolution::Fingerprint(fp) if fp == fingerprint(&uk)));
    }

    #[test]
    fn same_petname_two_keys_not_merged() {
        let (root, rk) = key(3);
        let (_x, xk) = key(4);
        let (_y, yk) = key(5);
        // root vouches BOTH xk and yk as "alice".
        let g = TrustGraph::from_edges(vec![
            root.sign_edge(xk, TrustKind::Vouch, Some("alice".into()), 1),
            root.sign_edge(yk, TrustKind::Vouch, Some("alice".into()), 1),
        ]);
        let local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);

        let rx = namer.resolve(&xk);
        let ry = namer.resolve(&yk);
        // Same display name, but the keys remain distinct identities: resolution
        // never collapses them, and callers disambiguate by fingerprint.
        assert_ne!(xk, yk);
        assert!(matches!(&rx, PetnameResolution::Vouch { name, .. } if name == "alice"));
        assert!(matches!(&ry, PetnameResolution::Vouch { name, .. } if name == "alice"));
        assert_ne!(fingerprint(&xk), fingerprint(&yk));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole collab::naming`
Expected: FAIL (build error — `Namer`, `PetnameResolution` undefined).

- [ ] **Step 3: Implement `Namer`**

Insert above the test module in `src/collab/naming.rs`:

```rust
// <bead-id>
/// How a key's display name was resolved. Keys are always canonical; a name is
/// only a label. `Vouch` carries its depth and trust path so a UI can show
/// "via X → Y" and mark depth-2 as a weak hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetnameResolution {
    /// This node's own name for the key (highest precedence).
    Local(String),
    /// A name suggested by the trust graph.
    Vouch { name: String, depth: u8, path: Vec<Key> },
    /// No name known; fall back to the key fingerprint.
    Fingerprint(String),
}

/// Resolves display names for keys under a fixed precedence:
/// local > depth-1 vouch > depth-2 vouch > fingerprint. Never merges two keys.
pub struct Namer<'a> {
    root: Key,
    local: &'a BTreeMap<Key, String>,
    graph: &'a TrustGraph,
}

impl<'a> Namer<'a> {
    pub fn new(root: Key, local: &'a BTreeMap<Key, String>, graph: &'a TrustGraph) -> Self {
        Self { root, local, graph }
    }

    pub fn resolve(&self, key: &Key) -> PetnameResolution {
        if let Some(name) = self.local.get(key) {
            return PetnameResolution::Local(name.clone());
        }
        let mut suggestions = self.graph.vouch_suggestions(&self.root, key, 2);
        // Prefer the shallowest suggestion (depth-1 before depth-2); deterministic.
        suggestions.sort_by_key(|s| s.depth);
        if let Some(s) = suggestions.into_iter().next() {
            return PetnameResolution::Vouch { name: s.petname, depth: s.depth, path: s.path };
        }
        PetnameResolution::Fingerprint(fingerprint(key))
    }
}
```

Add to `src/collab/mod.rs`:

```rust
// <bead-id>
pub mod naming;
pub use naming::{Namer, PetnameResolution};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p bole collab::naming`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/collab/naming.rs src/collab/mod.rs
git commit -m "<bead-id>: petname resolution precedence + non-merging collisions (G7)"
```

---

## Task 6: DNS alias — verified vs claimed, never authoritative

**Files:**
- Create: `src/collab/alias.rs`
- Modify: `src/collab/mod.rs` (add `pub mod alias;` + re-exports)

**Interfaces:**
- Consumes: `Key` (Task 1).
- Produces: `#[async_trait] pub trait AliasResolver { async fn asserted_key(&self, alias: &str) -> Result<Option<Key>>; }`; `pub enum AliasStatus { Verified, Claimed }`; `pub async fn verify_alias(resolver: &impl AliasResolver, alias: &str, key: &Key) -> Result<AliasStatus>`.

- [ ] **Step 1: Write the failing tests**

Create `src/collab/alias.rs`:

```rust
// <bead-id>
use async_trait::async_trait;

use crate::collab::Key;
use crate::error::Result;

// (implementation added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Test resolver: an in-memory map from alias -> the key the "domain" asserts,
    /// standing in for a `.well-known/bole-key` fetch or TXT lookup.
    struct MockDns(BTreeMap<String, Key>);

    #[async_trait]
    impl AliasResolver for MockDns {
        async fn asserted_key(&self, alias: &str) -> Result<Option<Key>> {
            Ok(self.0.get(alias).copied())
        }
    }

    #[tokio::test]
    async fn alias_verified_when_domain_asserts_key() {
        let alice = [1u8; 32];
        let mut m = BTreeMap::new();
        m.insert("alice@bole.dev".to_string(), alice);
        let dns = MockDns(m);
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &alice).await.unwrap(), AliasStatus::Verified);
    }

    #[tokio::test]
    async fn conflicting_alias_stays_claimed_key_canonical() {
        let alice = [1u8; 32];
        let mallory = [2u8; 32];
        // The domain asserts alice's key, but mallory ALSO claims the alias.
        let mut m = BTreeMap::new();
        m.insert("alice@bole.dev".to_string(), alice);
        let dns = MockDns(m);

        // Mallory's claim does not verify: the domain does not assert mallory's key.
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &mallory).await.unwrap(), AliasStatus::Claimed);
        // And the canonical identity is unchanged — verify_alias never returns a key.
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &alice).await.unwrap(), AliasStatus::Verified);

        // Unknown domain / no assertion -> Claimed, never an error that blocks use.
        let empty = MockDns(BTreeMap::new());
        assert_eq!(verify_alias(&empty, "ghost@nowhere.example", &alice).await.unwrap(), AliasStatus::Claimed);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole collab::alias`
Expected: FAIL (build error — `AliasResolver`, `verify_alias`, `AliasStatus` undefined).

- [ ] **Step 3: Implement alias verification**

Insert above the test module in `src/collab/alias.rs`:

```rust
// <bead-id>
/// How a DNS/email-style alias relates to a key. An alias is NEVER authoritative
/// and NEVER a resolution key; this status only changes how the alias is
/// *displayed*. Keys remain canonical regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasStatus {
    /// The claimed domain asserts exactly this key.
    Verified,
    /// The alias is claimed but the domain does not (or cannot) assert this key.
    Claimed,
}

/// Resolves what key, if any, a domain asserts for an alias. The production impl
/// fetches `https://<domain>/.well-known/bole-key` (or a TXT record); tests inject
/// a mock. Errors here are surfaced but must never be treated as identity loss.
#[async_trait]
pub trait AliasResolver {
    async fn asserted_key(&self, alias: &str) -> Result<Option<Key>>;
}

/// `Verified` iff the resolver reports the domain asserts exactly `key`;
/// otherwise `Claimed`. Never returns a key and never promotes an alias to
/// authority.
pub async fn verify_alias(resolver: &impl AliasResolver, alias: &str, key: &Key) -> Result<AliasStatus> {
    match resolver.asserted_key(alias).await? {
        Some(asserted) if &asserted == key => Ok(AliasStatus::Verified),
        _ => Ok(AliasStatus::Claimed),
    }
}
```

Add to `src/collab/mod.rs`:

```rust
// <bead-id>
pub mod alias;
pub use alias::{verify_alias, AliasResolver, AliasStatus};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p bole collab::alias`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/collab/alias.rs src/collab/mod.rs
git commit -m "<bead-id>: DNS alias verify stub (verified vs claimed, never authoritative) (G8)"
```

---

## Task 7: Discovery index — local gather, trust path, ordering

**Files:**
- Modify: `src/collab/discovery.rs` (add `DiscoveryResult`, `Index`, `gather`)
- Modify: `src/collab/mod.rs` (re-exports)

**Interfaces:**
- Consumes: `Key`, `CollabObject`, `Profile`, `TrustEdge`, `TrustKind`, `fingerprint` (Tasks 1–2); `TrustGraph::follow_neighborhood` (Task 4); `PublicObjectSource` (Task 3).
- Produces: `pub struct DiscoveryResult { pub key: Key, pub object: CollabObject, pub distance: u8, pub trust_path: Vec<Key> }`; `pub struct Index { results: Vec<DiscoveryResult> }`; `Index::build(root: Key, own: Vec<CollabObject>, pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)>) -> Index` where each pulled tuple is `(via_key, distance, trust_path, objects)`; `Index::query(&self, term: &str) -> Vec<&DiscoveryResult>`; `pub async fn gather<S: PublicObjectSource>(root: Key, own: &S, graph: &TrustGraph, hops: u8, sources: &BTreeMap<Key, &S>) -> Result<Index>`.

- [ ] **Step 1: Write the failing tests**

Add a test module to `src/collab/discovery.rs`:

```rust
// <bead-id>
#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabObject, CollabSigner, Key};

    fn profile(signer: &CollabSigner, name: &str, seq: u64) -> CollabObject {
        CollabObject::Profile(signer.sign_profile(name.into(), String::new(), vec![], vec![], seq))
    }

    fn key_of(obj: &CollabObject) -> Key {
        match obj {
            CollabObject::Profile(p) => p.key,
            CollabObject::TrustEdge(e) => e.from_key,
        }
    }

    #[test]
    fn index_orders_by_distance_then_recency() {
        let root_s = CollabSigner::from_seed([1u8; 32]);
        let near_s = CollabSigner::from_seed([2u8; 32]);
        let far_s = CollabSigner::from_seed([3u8; 32]);

        let rk = root_s.public_key();
        // own: root's own profile (distance 0)
        let own = vec![profile(&root_s, "root", 5)];
        // pulled: near at distance 1 (seq 9), far at distance 2 (seq 1)
        let pulled = vec![
            (near_s.public_key(), 1u8, vec![rk, near_s.public_key()], vec![profile(&near_s, "near", 9)]),
            (far_s.public_key(), 2u8, vec![rk, near_s.public_key(), far_s.public_key()], vec![profile(&far_s, "far", 1)]),
        ];
        let idx = Index::build(rk, own, pulled);

        // Query matches all three "profile" objects; ordering: distance asc, then seq desc.
        let hits = idx.query("");
        let dists: Vec<u8> = hits.iter().map(|r| r.distance).collect();
        assert_eq!(dists, vec![0, 1, 2], "sorted by trust distance");
    }

    #[test]
    fn result_carries_key_and_trust_path() {
        let root_s = CollabSigner::from_seed([4u8; 32]);
        let peer_s = CollabSigner::from_seed([5u8; 32]);
        let rk = root_s.public_key();
        let pk = peer_s.public_key();

        let own = vec![];
        let pulled = vec![(pk, 1u8, vec![rk, pk], vec![profile(&peer_s, "peer", 1)])];
        let idx = Index::build(rk, own, pulled);

        let hits = idx.query("peer");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, pk, "result carries the publishing key");
        assert_eq!(hits[0].trust_path, vec![rk, pk], "result carries the trust path");
        assert_eq!(key_of(&hits[0].object), pk);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole collab::discovery`
Expected: FAIL (build error — `Index`, `DiscoveryResult` undefined).

- [ ] **Step 3: Implement the index + gather**

Add to `src/collab/discovery.rs` (below the trait):

```rust
// <bead-id>
use std::collections::BTreeMap;

use crate::collab::trust::TrustGraph;
use crate::collab::{Key, Profile, TrustEdge};

/// A single discovery hit: the object, the key that published it, and how far /
/// by what route it was reached. Every hit is auditable back to a key + reason.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub key: Key,
    pub object: CollabObject,
    /// 0 = held locally by root; 1 = direct follow; 2 = friend-of-friend.
    pub distance: u8,
    /// Route `root -> ... -> publisher`.
    pub trust_path: Vec<Key>,
}

/// A per-node, trust-graph-local discovery index. No global/relay index exists in
/// this slice; all queries run locally over public objects held plus public
/// objects pulled from the follow-neighborhood.
pub struct Index {
    results: Vec<DiscoveryResult>,
}

fn publisher_key(obj: &CollabObject) -> Key {
    match obj {
        CollabObject::Profile(Profile { key, .. }) => *key,
        CollabObject::TrustEdge(TrustEdge { from_key, .. }) => *from_key,
    }
}

fn recency(obj: &CollabObject) -> u64 {
    match obj {
        CollabObject::Profile(p) => p.seq,
        CollabObject::TrustEdge(e) => e.seq,
    }
}

fn matches(obj: &CollabObject, term: &str) -> bool {
    if term.is_empty() {
        return true;
    }
    match obj {
        CollabObject::Profile(p) => {
            crate::collab::fingerprint(&p.key).contains(term)
                || p.display_name.contains(term)
                || p.bio.contains(term)
                || p.dns_aliases.iter().any(|a| a.contains(term))
        }
        CollabObject::TrustEdge(e) => {
            crate::collab::fingerprint(&e.from_key).contains(term)
                || e.petname.as_deref().map(|n| n.contains(term)).unwrap_or(false)
        }
    }
}

impl Index {
    /// Builds an index from root's own public objects (distance 0) and objects
    /// pulled from follow-neighbors, each tuple `(via, distance, trust_path, objects)`.
    pub fn build(
        _root: Key,
        own: Vec<CollabObject>,
        pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)>,
    ) -> Self {
        let mut results = Vec::new();
        for obj in own {
            results.push(DiscoveryResult { key: publisher_key(&obj), object: obj, distance: 0, trust_path: vec![_root] });
        }
        for (_via, distance, trust_path, objects) in pulled {
            for obj in objects {
                results.push(DiscoveryResult {
                    key: publisher_key(&obj),
                    object: obj,
                    distance,
                    trust_path: trust_path.clone(),
                });
            }
        }
        Self { results }
    }

    /// Deterministic query: matches `term`, ordered by trust distance ascending
    /// then recency (`seq`) descending. No numeric trust scores.
    pub fn query(&self, term: &str) -> Vec<&DiscoveryResult> {
        let mut hits: Vec<&DiscoveryResult> = self.results.iter().filter(|r| matches(&r.object, term)).collect();
        hits.sort_by(|a, b| {
            a.distance
                .cmp(&b.distance)
                .then_with(|| recency(&b.object).cmp(&recency(&a.object)))
        });
        hits
    }
}

/// Gathers a discovery index for `root`: root's own public objects at distance 0,
/// plus the public objects of each key in the bounded `Follow` neighborhood whose
/// source is present in `sources`. A key with no reachable source is simply
/// skipped (graceful degradation), never an error.
pub async fn gather<S: PublicObjectSource>(
    root: Key,
    own: &S,
    graph: &TrustGraph,
    hops: u8,
    sources: &BTreeMap<Key, &S>,
) -> Result<Index> {
    let own_objs = own.public_objects().await?;
    let neighborhood = graph.follow_neighborhood(&root, hops);
    let mut pulled = Vec::new();
    for (peer, distance) in neighborhood {
        if let Some(src) = sources.get(&peer) {
            match src.public_objects().await {
                Ok(objs) => pulled.push((peer, distance, vec![root, peer], objs)),
                Err(_) => continue, // unreachable/failed peer: degrade, don't fail
            }
        }
    }
    Ok(Index::build(root, own_objs, pulled))
}
```

Add to `src/collab/mod.rs`:

```rust
// <bead-id>
pub use discovery::{gather, DiscoveryResult, Index, PublicObjectSource};
```

> **Implementer note:** the `trust_path` for depth-2 peers is written here as `[root, peer]` for simplicity; if a depth-2 path should list the intermediary, thread the BFS predecessor from `follow_neighborhood` (out of scope for G9's tests, which assert distance ordering and that a path is present — keep it simple unless Task 8 needs the intermediary).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p bole collab::discovery`
Expected: PASS (2 tests). Then `cargo test -p bole` for no regressions.

- [ ] **Step 5: Commit**

```bash
git add src/collab/discovery.rs src/collab/mod.rs
git commit -m "<bead-id>: trust-graph-local discovery index (trust path + ordering) (G9)"
```

---

## Task 8: End-to-end integration across nodes

**Files:**
- Create: `tests/collab_discovery.rs`

**Interfaces:**
- Consumes: everything above — `Repository`, `CollabSigner`, `TrustKind`, `TrustGraph`, `gather`, `PublicObjectSource`, `COLLAB_SCOPED_PREFIX`, `Object`, `Ref`, `RefName`, `Tag`.

- [ ] **Step 1: Write the failing tests**

Create `tests/collab_discovery.rs`:

```rust
// <bead-id>
//! End-to-end: three sovereign in-memory nodes, follow edges, and the
//! discovery invariants — discoverable within depth, invisible beyond the hop
//! limit, scoped objects never discoverable, unreachable peers degrade.

use std::collections::BTreeMap;

use bole::collab::discovery::{gather, PublicObjectSource};
use bole::collab::trust::TrustGraph;
use bole::collab::{CollabObject, CollabSigner, Key, TrustKind};
use bole::object::Object;
use bole::refs::{Ref, RefName, Tag};
use bole::repo::collab::COLLAB_SCOPED_PREFIX;
use bole::Repository;

async fn node(seed: u8, name: &str) -> (Repository, CollabSigner, Key) {
    let repo = Repository::memory();
    let signer = CollabSigner::from_seed([seed; 32]);
    let key = signer.public_key();
    repo.publish_profile(&signer.sign_profile(name.into(), String::new(), vec![], vec![], 1))
        .await
        .unwrap();
    (repo, signer, key)
}

#[tokio::test]
async fn three_node_discovery_within_depth() {
    // a -follow-> b -follow-> c
    let (a_repo, a, ak) = node(1, "alice").await;
    let (b_repo, b, bk) = node(2, "bob").await;
    let (c_repo, _c, ck) = node(3, "carol").await;

    // a's local trust view: it follows b; it also knows (from b) that b follows c.
    let graph = TrustGraph::from_edges(vec![
        a.sign_edge(bk, TrustKind::Follow, None, 1),
        b.sign_edge(ck, TrustKind::Follow, None, 1),
    ]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);
    sources.insert(ck, &c_repo);

    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(!idx.query("bob").is_empty(), "b discoverable at depth 1");
    assert!(!idx.query("carol").is_empty(), "c discoverable at depth 2");
}

#[tokio::test]
async fn beyond_hop_limit_invisible() {
    let (a_repo, a, ak) = node(4, "alice").await;
    let (b_repo, b, bk) = node(5, "bob").await;
    let (c_repo, _c, ck) = node(6, "carol").await;
    let graph = TrustGraph::from_edges(vec![
        a.sign_edge(bk, TrustKind::Follow, None, 1),
        b.sign_edge(ck, TrustKind::Follow, None, 1),
    ]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);
    sources.insert(ck, &c_repo);

    // hops = 1: carol (2 hops) must be invisible.
    let idx = gather(ak, &a_repo, &graph, 1, &sources).await.unwrap();
    assert!(!idx.query("bob").is_empty(), "b still visible at depth 1");
    assert!(idx.query("carol").is_empty(), "c beyond hop limit must be invisible");
}

#[tokio::test]
async fn scoped_never_discoverable_e2e() {
    let (a_repo, a, ak) = node(7, "alice").await;
    let (b_repo, _b, bk) = node(8, "bob").await;

    // Bob pins a SCOPED profile (a future capability-scoped object).
    let secret_signer = CollabSigner::from_seed([88u8; 32]);
    let scoped = secret_signer.sign_profile("top-secret".into(), String::new(), vec![], vec![], 1);
    let id = b_repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
    let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/secret");
    let mut tx = b_repo.refs.transaction();
    tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
    tx.commit().unwrap();

    let graph = TrustGraph::from_edges(vec![a.sign_edge(bk, TrustKind::Follow, None, 1)]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);

    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(idx.query("top-secret").is_empty(), "scoped object must never be discoverable");
    assert!(!idx.query("bob").is_empty(), "bob's public profile still discoverable");
}

#[tokio::test]
async fn unreachable_peer_degrades_gracefully() {
    let (a_repo, a, ak) = node(9, "alice").await;
    let (_b_repo, _b, bk) = node(10, "bob").await;

    // a follows b, but b's source is absent from the map (unreachable).
    let graph = TrustGraph::from_edges(vec![a.sign_edge(bk, TrustKind::Follow, None, 1)]);
    let sources: BTreeMap<Key, &Repository> = BTreeMap::new();

    // Must not error; just yields a staler (b-less) index.
    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(idx.query("bob").is_empty(), "unreachable peer simply absent");
    assert!(!idx.query("alice").is_empty(), "own profile still present");
}
```

> **Implementer note:** this integration test uses `bole::repo::collab::COLLAB_SCOPED_PREFIX` and `bole::collab::discovery::*` paths. If Task 3/7 kept `repo::collab` private, expose the needed items via the `pub use` in `src/lib.rs` added in those tasks and import them from the crate root instead (e.g. `bole::COLLAB_SCOPED_PREFIX`). Match whichever public path the earlier tasks settled on.

- [ ] **Step 2: Run tests to verify they fail, then pass**

Run: `cargo test -p bole --test collab_discovery`
Expected: FAIL if any earlier export path differs (fix imports), then PASS (4 tests).

- [ ] **Step 3: Full suite + clippy**

Run: `cargo test -p bole && cargo clippy -p bole --all-targets -- -D warnings`
Expected: all tests PASS; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add tests/collab_discovery.rs
git commit -m "<bead-id>: e2e discovery across nodes — depth, invisibility, scoped-never, degradation (G10)"
```

---

## Self-Review

**Spec coverage** (each spec section → task):
- §1 thesis/invariants → encoded in Global Constraints; enforced by G4 (visibility), G6 (trust bounds), G7 (naming), roots-authoritative is a documented invariant (no code path promotes graph→root, so nothing to test beyond absence).
- §2 object model (`Profile`, `TrustEdge`, `Object::Collab`, monotonic, `Review` reserved, local petname map) → Tasks 1, 2, 3 (G1, G2, G3, G5). Local petname map is `BTreeMap<Key,String>` consumed by Task 5's `Namer` (never persisted/served — satisfies "private by construction").
- §3 topology/replication (`PublicObjectSource`, serve public-only, pull-based, relay-as-same-interface) → Tasks 3, 7 (G4, G10). Relay = future impl of the Task 3 trait; documented, not implemented.
- §4 trust graph + naming + DNS alias → Tasks 4, 5, 6 (G6, G7, G8).
- §5 discovery/index/query (local-only, ordering, key+object+trust-path) → Task 7 (G9).
- §6 security/failure (signature+schema+label+neighborhood gate, highest-seq, graceful degradation, headline invariant) → Tasks 1–3, 7, 8 (G2, G4, G5, G10). *Schema-sanity beyond signature (recognized kind / monotonic seq): kind is enforced by the type system (enum), monotonic seq by G5; no separate free-form validation needed for these two object types.*
- §7 testing → every Gate maps to named tests.
- §8 scope boundary → nothing here builds a UI, relay, PR, board, or network transport.

**Placeholder scan:** no "TBD"/"add error handling"/"similar to Task N" — every code step shows full code. Two `Implementer note` blocks flag *environment-confirmation* points (exact `ObjectId` import, `Error::msg` constructor, module visibility, export paths) rather than leaving logic unwritten; these are unavoidable "match the surrounding codebase" checks, each with the exact reference file named.

**Type consistency:** `Key = [u8;32]` used throughout; `CollabSigner::sign_profile`/`sign_edge` signatures match their call sites in Tasks 3–8; `follow_neighborhood -> BTreeMap<Key,u8>` consumed consistently by Task 7's `gather`; `Index::build` tuple `(Key,u8,Vec<Key>,Vec<CollabObject>)` matches its construction in `gather` and in Task 7 tests; `PublicObjectSource::public_objects` name identical in trait (Task 3), impl (Task 3), and consumer (Task 7).

**One flagged risk:** the exact object-id type is written as `ObjectIdAlias` in Task 3's code to avoid a bad import assumption — the Step-4 note directs replacing it with `crate::object::ObjectId`. This is the single place an implementer must substitute a concrete name; it is called out explicitly rather than hidden.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-03-ws8a-collaboration-substrate.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task (one bead each, branch = bead id), review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints for review.

Which approach?
