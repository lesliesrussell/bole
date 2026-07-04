# WS8f-a — Trusted Relay Set + Authenticated Multi-Relay Query Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist a key-pinned set of relays, prove each relay holds its pinned key via a challenge-response handshake, then query the whole set and merge the verified results into one trust-aware ranking attributed per relay.

**Architecture:** Library-first (`bole`) with a thin `bole-cli` surface. A relay pin is local unsigned config stored as an `Object::Blob` under a new local-only ref namespace `refs/collab/relays/`. The WS5 wire gains two optional fields (`Hello.client_nonce`, `Welcome.relay_sig`); a relay serving with `--relay` and a signer answers a client nonce with an Ed25519 signature over a domain-separated challenge, which the client verifies against the pinned key before processing any bytes. Multi-relay query authenticates each pinned relay, fetches transiently, verifies fail-closed, unions the corpora (profiles deduped by key/highest-seq, edges by object-id), and runs the WS8e ranking once with per-relay attribution. Failures skip-and-continue.

**Tech Stack:** Rust, tokio, `ed25519-dalek`, `postcard`, `blake3`, `bytes`, `serde`, `rand` (`OsRng`), `serde_json` (CLI), loopback `TcpConn` + real-binary CLI tests.

## Global Constraints

- **ZERO code without a bead.** Each Gate below is one bead; branch name = bead ID exactly; each contiguous added block gets a `// <bead-id>` comment (one per block, ID only). Use `bd` for tracking — never TodoWrite/markdown TODOs.
- **Relays never authoritative over objects.** Every returned object is verified fail-closed against its embedded author key (`verify_profile`/`verify_edge`). The handshake authenticates the *relay*, never blesses its objects.
- **Endpoint stays read-only.** No write/announce path is added.
- **Soundness from per-edge verification, not relay-trust.** Relay auth only gates *whether the client processes a relay's bytes*. A relay that fails auth, serves a bad object, or is offline degrades **completeness**, never **soundness**.
- **Transient query mutates no local state.** `discover relay` persists nothing about results. The only new persisted surface is `refs/collab/relays/`, written solely by `relay add`/`relay remove`, never by a query.
- **Local depth-2 query untouched.** `discover query` and `follow_*` behavior is unchanged.
- **Keys canonical / raw hex.** Relay keys and all output keys are raw 64-hex. Signing seeds (client and relay) come from env/file, never argv.
- **Domain separator is exactly** `b"bole-relay-auth-v1"` prepended to the 32-byte nonce; the signed message is `b"bole-relay-auth-v1" || nonce`.
- **One endpoint per key.** `relay add` is an upsert keyed by `fingerprint(&key)`; a key maps to exactly one endpoint.
- **Additive-optional wire fields.** `Hello.client_nonce: Option<[u8;32]>` and `Welcome.relay_sig: Option<[u8;64]>` default to `None` at every non-relay construction site; non-relay serve/pull flows behave identically (no auth attempted). There is no cross-version wire-compat requirement — client and relay are built from the same tree — so the extra `None` discriminant byte in the postcard frame is immaterial.
- **`node serve --relay` now REQUIRES a signer** (`--key-env`/`--key-file`). A non-relay `node serve` needs no key and is unaffected.

---

## File Structure

- **Create** `src/collab/relay.rs` — `RelayPin { key: Key, endpoint: String }` + postcard (de)serialize helpers. One responsibility: the relay-pin data type and its byte encoding.
- **Modify** `src/collab/mod.rs` — `mod relay; pub use relay::RelayPin;`.
- **Modify** `src/repo/collab.rs` — `COLLAB_RELAYS_PREFIX` + `add_relay`/`remove_relay`/`relays` CRUD.
- **Modify** `src/collab/object.rs` — `COLLAB_RELAY_AUTH_DOMAIN`, `relay_challenge_message`, `CollabSigner::sign_relay_challenge`, `verify_relay_challenge`.
- **Modify** `src/sync/wire.rs` — add optional `client_nonce` to `Hello`, `relay_sig` to `Welcome`; update the round-trip test.
- **Modify** `src/sync/session.rs` — set `client_nonce: None` / `relay_sig: None` at every existing `Hello`/`Welcome` construction site.
- **Modify** `src/sync/collab.rs` — `serve_collab`/`serve_collab_tcp_once` gain `relay_signer: Option<&CollabSigner>`; add `collab_fetch_authenticated`; add pure `rank_strangers_multi` + I/O `query_relay_set`.
- **Modify** `src/collab/discovery.rs` — add `relays: Vec<Key>` to `StrangerHit`.
- **Modify** `src/lib.rs` — re-export `RelayPin`, `query_relay_set`, `rank_strangers_multi`, `verify_relay_challenge`.
- **Create** `bole-cli/src/commands/relay.rs` — `relay add/remove/list`.
- **Modify** `bole-cli/src/commands/discover.rs` — `Relay` arm queries the pinned set; `--endpoint` escape hatch.
- **Modify** `bole-cli/src/commands/node.rs` — `serve --relay` requires signer.
- **Modify** `bole-cli/src/main.rs` — wire the `relay` command group.
- **Modify** `bole-cli/tests/collab_cli.rs` — migrate the two WS8d/e E2E tests to the new syntax; add the WS8f-a E2E.
- **Modify** `tests/collab_network.rs` — loopback multi-relay tests.

Each Gate is one bead. Recommended bead order: G1 → G2 → G3 → G4 → G5 (G4 depends on G1–G3; G5 depends on all).

---

## Gate 1 (bead: relay-set storage) — `RelayPin` + Repository CRUD

**Files:**
- Create: `src/collab/relay.rs`
- Modify: `src/collab/mod.rs`, `src/repo/collab.rs`, `src/lib.rs`
- Test: unit tests in `src/repo/collab.rs` (`#[cfg(test)]`) and `src/sync/collab.rs` adverts test

**Interfaces:**
- Consumes: `Key` (`src/collab/mod.rs`), `fingerprint` (`src/collab/mod.rs`), `Object::Blob`/`Blob` (`src/object`), `bytes::Bytes`, `RefName`/`Ref`/`Tag`/`self.refs`/`self.objects` (existing `Repository` internals, see `pin_profile` at `src/repo/collab.rs:60`).
- Produces:
  - `pub struct RelayPin { pub key: Key, pub endpoint: String }` (derives `Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize`)
  - `RelayPin::to_bytes(&self) -> Vec<u8>` and `RelayPin::from_bytes(&[u8]) -> Option<RelayPin>`
  - `Repository::add_relay(&self, pin: RelayPin) -> Result<()>` (upsert by `fingerprint(&pin.key)`)
  - `Repository::remove_relay(&self, key: &Key) -> Result<bool>`
  - `Repository::relays(&self) -> Result<Vec<RelayPin>>` (order by ref name = fingerprint)
  - `pub const COLLAB_RELAYS_PREFIX: &str = "refs/collab/relays/";`

- [ ] **Step 1: Write the failing test — RelayPin byte round-trip** (in `src/collab/relay.rs`, `#[cfg(test)] mod tests`)

```rust
#[test]
fn relay_pin_round_trips() {
    let pin = RelayPin { key: [7u8; 32], endpoint: "relay.example:9418".into() };
    let bytes = pin.to_bytes();
    assert_eq!(RelayPin::from_bytes(&bytes), Some(pin));
    assert_eq!(RelayPin::from_bytes(b"not-valid-postcard-\xff\xff"), None);
}
```

- [ ] **Step 2: Run it, verify it fails** — `cargo test -p bole --lib collab::relay` → FAIL (module/type absent).

- [ ] **Step 3: Create `src/collab/relay.rs`**

```rust
// <bead-id>
//! A trusted-relay pin: the relay's pinned Ed25519 public key and its transport
//! endpoint. Local, unsigned config (NOT a self-signed `CollabObject`): stored as
//! a plain `Object::Blob` under `refs/collab/relays/`. The key is the identity;
//! the endpoint is merely where to reach it.
use crate::collab::Key;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPin {
    pub key: Key,
    pub endpoint: String,
}

impl RelayPin {
    /// Postcard bytes for blob storage. Infallible for owned data.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("postcard serialization is infallible for owned data")
    }

    /// Parses a pin from blob bytes; `None` if the bytes are not a valid pin.
    pub fn from_bytes(bytes: &[u8]) -> Option<RelayPin> {
        postcard::from_bytes(bytes).ok()
    }
}
```

- [ ] **Step 4: Register the module** — in `src/collab/mod.rs`, next to the other `mod`/`pub use` lines, add:

```rust
// <bead-id>
mod relay;
pub use relay::RelayPin;
```

- [ ] **Step 5: Run it, verify it passes** — `cargo test -p bole --lib collab::relay` → PASS.

- [ ] **Step 6: Write the failing test — Repository CRUD + upsert** (in `src/repo/collab.rs` test module; mirror the existing profile tests there for repo construction)

```rust
#[tokio::test]
async fn relay_pin_crud_and_upsert() {
    let repo = test_repo().await; // use the same helper the other collab tests in this file use
    let key = [9u8; 32];
    assert!(repo.relays().await.unwrap().is_empty());

    repo.add_relay(RelayPin { key, endpoint: "a:1".into() }).await.unwrap();
    assert_eq!(repo.relays().await.unwrap(), vec![RelayPin { key, endpoint: "a:1".into() }]);

    // Upsert: same key, new endpoint -> still one entry, endpoint replaced.
    repo.add_relay(RelayPin { key, endpoint: "b:2".into() }).await.unwrap();
    assert_eq!(repo.relays().await.unwrap(), vec![RelayPin { key, endpoint: "b:2".into() }]);

    assert!(repo.remove_relay(&key).await.unwrap(), "removed an existing pin");
    assert!(!repo.remove_relay(&key).await.unwrap(), "removing absent pin returns false");
    assert!(repo.relays().await.unwrap().is_empty());
}
```

> If `src/repo/collab.rs` has no `test_repo()` helper, use whatever repo-construction the existing `#[tokio::test]` functions in that file already use (search for `async fn` tests near `pin_profile`); do not invent a new harness.

- [ ] **Step 7: Run it, verify it fails** — `cargo test -p bole --lib repo::collab::tests::relay_pin_crud_and_upsert` → FAIL (methods absent).

- [ ] **Step 8: Add the CRUD methods** in `src/repo/collab.rs`. Add the prefix constant near the other prefix consts (line ~20) and the methods in the `impl Repository` block near `pin_profile`:

```rust
// <bead-id>
/// Local-only namespace for trusted-relay pins. NEVER advertised or served
/// (see `collab_adverts`, which is an allowlist of public + remotes only).
pub const COLLAB_RELAYS_PREFIX: &str = "refs/collab/relays/";
```

```rust
// <bead-id>
/// Upserts a trusted-relay pin, keyed by `fingerprint(&pin.key)` so a key maps
/// to exactly one endpoint. Stored as an `Object::Blob` under
/// `refs/collab/relays/<relay-fp>`. Local config; not a signed collab object.
pub async fn add_relay(&self, pin: RelayPin) -> Result<()> {
    use crate::object::{Blob, Object};
    let id = self
        .objects
        .put(&Object::Blob(Blob { data: bytes::Bytes::from(pin.to_bytes()) }))
        .await?;
    let leaf = format!("{COLLAB_RELAYS_PREFIX}{}", crate::collab::fingerprint(&pin.key));
    let mut tx = self.refs.transaction();
    tx.set(RefName::new(leaf)?, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
    tx.commit()?;
    Ok(())
}

// <bead-id>
/// Removes a relay pin by key. Returns whether a pin existed.
pub async fn remove_relay(&self, key: &Key) -> Result<bool> {
    let leaf = format!("{COLLAB_RELAYS_PREFIX}{}", crate::collab::fingerprint(key));
    let name = RefName::new(leaf)?;
    if self.refs.get_tag(&name)?.is_none() {
        return Ok(false);
    }
    let mut tx = self.refs.transaction();
    tx.delete_ref(name);
    tx.commit()?;
    Ok(true)
}

// <bead-id>
/// All trusted-relay pins, ordered by ref name (relay fingerprint).
pub async fn relays(&self) -> Result<Vec<RelayPin>> {
    use crate::object::Object;
    let mut out = Vec::new();
    for name in self.refs.list(COLLAB_RELAYS_PREFIX)? {
        if let Some(tag) = self.refs.get_tag(&name)? {
            if let Some(Object::Blob(b)) = self.objects.get(&tag.target).await? {
                if let Some(pin) = RelayPin::from_bytes(&b.data) {
                    out.push(pin);
                }
            }
        }
    }
    Ok(out)
}
```

> Add `use crate::collab::RelayPin;` to the imports at the top of `src/repo/collab.rs` if not already imported. Confirm `self.refs.list(prefix)` returns names in a deterministic (sorted) order; if it does not, sort `out` by `fingerprint(&pin.key)` before returning.

- [ ] **Step 9: Run it, verify it passes** — `cargo test -p bole --lib repo::collab::tests::relay_pin_crud_and_upsert` → PASS.

- [ ] **Step 10: Write the failing test — adverts exclude `relays/`** (in `src/sync/collab.rs` test module, mirroring `collab_adverts_exclude_scoped` at line ~254)

```rust
#[tokio::test]
async fn collab_adverts_exclude_relays() {
    let repo = /* same repo builder used by collab_adverts_exclude_scoped */;
    // Pin a relay AND publish a public profile so adverts are non-empty.
    repo.add_relay(RelayPin { key: [5u8; 32], endpoint: "x:1".into() }).await.unwrap();
    // (publish a public profile here exactly as the neighboring advert tests do)
    for relay in [false, true] {
        let adverts = collab_adverts(&repo, relay).await.unwrap();
        for a in &adverts {
            assert!(
                !a.name.starts_with(crate::repo::collab::COLLAB_RELAYS_PREFIX),
                "relays/ must never be advertised (relay={relay})"
            );
        }
    }
}
```

> Match the `RefAdvert` field used for the ref path (the neighboring scoped test shows whether it is `a.name` or similar); reuse that test's exact repo setup and profile-publishing lines. The assertion for `scoped/` in the neighboring test already covers scoped exclusion — do not duplicate it.

- [ ] **Step 11: Run it, verify it passes immediately** — `cargo test -p bole --lib sync::collab::tests::collab_adverts_exclude_relays` → PASS (adverts is already an allowlist, so `relays/` is excluded by construction; this test locks that in). If it FAILS, `collab_adverts` is not an allowlist — stop and report.

- [ ] **Step 12: Re-export `RelayPin`** — in `src/lib.rs`, add `RelayPin` and `COLLAB_RELAYS_PREFIX` to the collab re-export line.

- [ ] **Step 13: Full check + commit**

```bash
cargo test -p bole --lib collab::relay repo::collab sync::collab
cargo clippy --workspace
git add src/collab/relay.rs src/collab/mod.rs src/repo/collab.rs src/sync/collab.rs src/lib.rs
git commit -m "<bead-id>: RelayPin + Repository relay-set CRUD (refs/collab/relays/, adverts-excluded)"
```

---

## Gate 2 (bead: relay-auth crypto) — domain-separated challenge sign/verify

**Files:**
- Modify: `src/collab/object.rs` (add domain const, challenge message builder, sign method, verify fn)
- Modify: `src/lib.rs` (re-export `verify_relay_challenge`)
- Test: unit tests in `src/collab/object.rs`

**Interfaces:**
- Consumes: `SigningKey`/`VerifyingKey`/`Signer`/`Verifier` (already imported at `src/collab/object.rs:2`), `Key`, `CollabSigner`.
- Produces:
  - `pub const COLLAB_RELAY_AUTH_DOMAIN: &[u8] = b"bole-relay-auth-v1";`
  - `CollabSigner::sign_relay_challenge(&self, nonce: &[u8; 32]) -> [u8; 64]`
  - `pub fn verify_relay_challenge(key: &Key, nonce: &[u8; 32], sig: &[u8; 64]) -> bool`

- [ ] **Step 1: Write the failing tests** (in `src/collab/object.rs` test module)

```rust
#[test]
fn relay_challenge_accepts_valid_and_rejects_tampering() {
    let signer = CollabSigner::from_seed([1u8; 32]);
    let nonce = [42u8; 32];
    let sig = signer.sign_relay_challenge(&nonce);

    // Accept: right key, right nonce, domain-separated.
    assert!(verify_relay_challenge(&signer.public_key(), &nonce, &sig));

    // Reject: wrong key.
    let other = CollabSigner::from_seed([2u8; 32]);
    assert!(!verify_relay_challenge(&other.public_key(), &nonce, &sig));

    // Reject: different nonce (replay of a signature for another challenge).
    let nonce2 = [43u8; 32];
    assert!(!verify_relay_challenge(&signer.public_key(), &nonce2, &sig));

    // Reject: a signature over the BARE nonce (no domain separator).
    let bare = signer_sign_raw(&signer, &nonce); // helper below
    assert!(!verify_relay_challenge(&signer.public_key(), &nonce, &bare));
}

// Test-only: sign the raw nonce with no domain separator, to prove the domain
// separator is load-bearing.
fn signer_sign_raw(signer: &CollabSigner, nonce: &[u8; 32]) -> [u8; 64] {
    signer.sign_relay_challenge_raw_for_test(nonce)
}
```

> To avoid exposing signing internals, add a `#[cfg(test)]`-only method on `CollabSigner` in `src/collab/object.rs` that signs the bare nonce:
> ```rust
> #[cfg(test)]
> pub fn sign_relay_challenge_raw_for_test(&self, nonce: &[u8; 32]) -> [u8; 64] {
>     self.signing.sign(nonce).to_bytes()
> }
> ```

- [ ] **Step 2: Run it, verify it fails** — `cargo test -p bole --lib collab::object::tests::relay_challenge` → FAIL (functions absent).

- [ ] **Step 3: Implement the challenge primitives** in `src/collab/object.rs`

```rust
// <bead-id>
/// Domain separator for the relay-auth possession handshake. Prepended to the
/// client nonce before signing so a relay-auth signature can never be confused
/// with a signature over an arbitrary 32-byte challenge from any other feature.
pub const COLLAB_RELAY_AUTH_DOMAIN: &[u8] = b"bole-relay-auth-v1";

// <bead-id>
/// The exact bytes a relay signs to prove possession of its key: the domain
/// separator followed by the client's nonce.
fn relay_challenge_message(nonce: &[u8; 32]) -> Vec<u8> {
    let mut m = COLLAB_RELAY_AUTH_DOMAIN.to_vec();
    m.extend_from_slice(nonce);
    m
}

// <bead-id>
/// True iff `sig` is `key`'s Ed25519 signature over `COLLAB_RELAY_AUTH_DOMAIN || nonce`.
pub fn verify_relay_challenge(key: &Key, nonce: &[u8; 32], sig: &[u8; 64]) -> bool {
    let vk = match VerifyingKey::from_bytes(key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    vk.verify(&relay_challenge_message(nonce), &ed25519_dalek::Signature::from_bytes(sig)).is_ok()
}
```

Add the signing method inside `impl CollabSigner` (near `sign_edge`):

```rust
// <bead-id>
/// Signs the domain-separated relay-auth challenge for `nonce`, proving
/// possession of this signer's key to a client that pinned its public key.
pub fn sign_relay_challenge(&self, nonce: &[u8; 32]) -> [u8; 64] {
    self.signing.sign(&relay_challenge_message(nonce)).to_bytes()
}
```

- [ ] **Step 4: Run it, verify it passes** — `cargo test -p bole --lib collab::object::tests::relay_challenge` → PASS.

- [ ] **Step 5: Re-export + commit** — add `verify_relay_challenge` to the `src/lib.rs` collab re-export.

```bash
cargo test -p bole --lib collab::object
cargo clippy --workspace
git add src/collab/object.rs src/lib.rs
git commit -m "<bead-id>: relay-auth challenge sign/verify (domain-separated nonce)"
```

---

## Gate 3 (bead: wire + serve-side auth) — optional handshake fields, relay signs

**Files:**
- Modify: `src/sync/wire.rs` (add fields; update round-trip test)
- Modify: `src/sync/session.rs` (set `None` at all existing Hello/Welcome sites)
- Modify: `src/sync/collab.rs` (`serve_collab`/`serve_collab_tcp_once` gain `relay_signer`; `collab_fetch_transient` sends `client_nonce: None`)
- Test: unit test in `src/sync/collab.rs`

**Interfaces:**
- Consumes: `CollabSigner::sign_relay_challenge` (Gate 2), `Message`, `PROTO_VERSION`, `CapSet`, `Intent`.
- Produces:
  - `Message::Hello { proto_min, proto_max, caps, intent, client_nonce: Option<[u8; 32]> }`
  - `Message::Welcome { proto, caps, refs, relay_sig: Option<[u8; 64]> }`
  - `serve_collab(conn, repo, relay: bool, relay_signer: Option<&CollabSigner>) -> Result<()>`
  - `serve_collab_tcp_once(listener, repo, relay: bool, relay_signer: Option<&CollabSigner>) -> Result<()>`

- [ ] **Step 1: Add the optional wire fields** in `src/sync/wire.rs` (variant definitions at lines 81/83):

```rust
    // <bead-id>
    /// client → server: version range, capabilities, intent. `client_nonce` is
    /// `Some` only on a relay-auth query; `None` (all other flows) requests no
    /// relay-auth and is byte-compatible behaviour with pre-WS8f-a callers.
    Hello { proto_min: u16, proto_max: u16, caps: CapSet, intent: Intent, client_nonce: Option<[u8; 32]> },
    /// server → client: chosen version + capabilities + advertised refs.
    /// `relay_sig` is `Some` only when a relay with a signer answers a
    /// `client_nonce`; `None` otherwise.
    Welcome { proto: u16, caps: CapSet, refs: Vec<RefAdvert>, relay_sig: Option<[u8; 64]> },
```

- [ ] **Step 2: Fix all construction/detarget sites to compile.** Update every `Message::Hello { .. }` construction to add `client_nonce: None` and every `Message::Welcome { .. }` construction to add `relay_sig: None`, EXCEPT the two relay-path sites changed in later steps. Sites (from grep):
  - `src/sync/wire.rs:177` (Hello test), `:186` (Welcome test) — add the fields; then extend the round-trip assertions (Step 3).
  - `src/sync/session.rs:45, 67` (Welcome), `:291, 337, 530, 655` (Hello).
  - `src/sync/collab.rs:77` (Welcome — serve; changed in Step 5), `:118, 211` (Hello — pull clients: `client_nonce: None`), `:290` (Hello test).
  - Pattern-match sites using `{ .. }` (e.g. `Message::Hello { intent, .. }`, `Message::Welcome { refs, .. }`) already ignore new fields — leave them.

> Do a `cargo build -p bole 2>&1` after this step and fix every "missing field" error the compiler names. Do not add the fields to match patterns that use `..`.

- [ ] **Step 3: Update the wire round-trip test** at `src/sync/wire.rs:177`/`:186` to exercise the new fields:

```rust
    // <bead-id>
    let m = Message::Hello {
        proto_min: 1, proto_max: 1, caps: CapSet::EMPTY, intent: Intent::Fetch,
        client_nonce: Some([7u8; 32]),
    };
    assert_eq!(decode_message(&encode_message(&m).unwrap()).unwrap(), m);
    let w = Message::Welcome {
        proto: 1, caps: CapSet::EMPTY, refs: vec![], relay_sig: Some([9u8; 64]),
    };
    assert_eq!(decode_message(&encode_message(&w).unwrap()).unwrap(), w);
```

> Keep whatever assertions already existed; just add the two new fields to the constructed values and confirm round-trip still holds. If `Message` lacks `PartialEq`, compare via re-encoding both sides.

- [ ] **Step 4: Write the failing test — serve signs a nonce; no-signer serves `None`** (in `src/sync/collab.rs` test module, using the in-memory duplex `Conn` the neighbouring `serve_collab` tests use, e.g. the pattern near line 290)

```rust
#[tokio::test]
async fn serve_relay_signs_client_nonce() {
    let repo = /* same builder as neighbouring serve tests */;
    let signer = CollabSigner::from_seed([3u8; 32]);
    let nonce = [8u8; 32];

    // Drive a serve with a relay signer against an in-memory client end.
    let (mut client, mut server) = /* duplex Conn pair as in existing tests */;
    let serve = tokio::spawn(async move {
        serve_collab(&mut server, &repo, true, Some(&signer)).await
    });
    client.send(&Message::Hello {
        proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: CapSet::EMPTY,
        intent: Intent::Fetch, client_nonce: Some(nonce),
    }).await.unwrap();
    let welcome = client.recv().await.unwrap();
    let relay_sig = match welcome { Message::Welcome { relay_sig, .. } => relay_sig, o => panic!("{o:?}") };
    let sig = relay_sig.expect("relay with signer signs the client nonce");
    assert!(bole::verify_relay_challenge(&signer.public_key(), &nonce, &sig));
    // drain the rest of the exchange so the server task completes
    client.send(&Message::HaveWant { want: vec![], have: vec![] }).await.unwrap();
    let _ = client.recv().await; // Pack
    let _ = client.recv().await; // Done
    serve.await.unwrap().unwrap();
}
```

> Use the exact duplex-`Conn` construction and repo builder the other `serve_collab` tests in this file already use; do not invent a transport. If those tests use `serve_collab_tcp_once` over loopback instead, follow that shape and connect a `TcpConn` client.

- [ ] **Step 5: Thread the signer + sign in `serve_collab`** (`src/sync/collab.rs`). Change the signature and the Welcome send:

```rust
// <bead-id>
pub async fn serve_collab(
    conn: &mut dyn Conn,
    repo: &Repository,
    relay: bool,
    relay_signer: Option<&CollabSigner>,
) -> Result<()> {
    let hello = conn.recv().await?;
    let client_nonce = match &hello {
        Message::Hello { intent: Intent::Fetch, client_nonce, .. }
        | Message::Hello { intent: Intent::Clone, client_nonce, .. } => *client_nonce,
        Message::Hello { intent: Intent::Push, .. } => {
            conn.send(&Message::Error("collab endpoint is read-only".into())).await?;
            return Err(Error::Storage("collab: push not permitted".into()));
        }
        _ => {
            conn.send(&Message::Error("expected Hello".into())).await?;
            return Err(Error::Storage("collab: expected Hello".into()));
        }
    };
    let refs = collab_adverts(repo, relay).await?;
    let authorized: HashSet<_> = refs.iter().map(|r| r.target).collect();
    // A relay with a signer proves possession of its key over the client nonce.
    let relay_sig = match (relay_signer, client_nonce) {
        (Some(signer), Some(nonce)) => Some(signer.sign_relay_challenge(&nonce)),
        _ => None,
    };
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs, relay_sig }).await?;
    // ... rest unchanged (HaveWant → filter → pack → Done) ...
}
```

Update `serve_collab_tcp_once` to take and forward `relay_signer: Option<&CollabSigner>`:

```rust
// <bead-id>
pub async fn serve_collab_tcp_once(
    listener: &tokio::net::TcpListener,
    repo: &Repository,
    relay: bool,
    relay_signer: Option<&CollabSigner>,
) -> Result<()> {
    let (stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let mut conn = crate::sync::transport::TcpConn::new(stream);
    serve_collab(&mut conn, repo, relay, relay_signer).await
}
```

- [ ] **Step 6: Set `client_nonce: None` in `collab_fetch_transient`** (`src/sync/collab.rs:118`-area) so the ad-hoc/unpinned fetch requests no auth (behaviour unchanged). Leave its Welcome-destructure `{ refs, .. }` as is.

- [ ] **Step 7: Fix callers of the two changed fns.** Update every `serve_collab(` / `serve_collab_tcp_once(` call outside tests to pass `None` for `relay_signer` (the CLI relay-mode caller gets its real signer in Gate 5). Run `cargo build --workspace 2>&1` and fix each named call site.

- [ ] **Step 8: Run tests** — `cargo test -p bole --lib sync::` → PASS (new serve test + wire round-trip + existing sync tests). `cargo clippy --workspace` clean.

- [ ] **Step 9: Commit**

```bash
git add src/sync/wire.rs src/sync/session.rs src/sync/collab.rs
git commit -m "<bead-id>: additive relay-auth wire fields + serve-side relay signing"
```

---

## Gate 4 (bead: authenticated fetch + multi-relay merge) — client auth, union, rank, attribute

**Files:**
- Modify: `src/sync/collab.rs` (`collab_fetch_authenticated`, `query_relay_set`)
- Modify: `src/collab/discovery.rs` (`StrangerHit.relays`, `rank_strangers_multi`)
- Modify: `src/lib.rs` (re-exports)
- Test: `tests/collab_network.rs` loopback tests; unit test for `rank_strangers_multi` in `src/collab/discovery.rs`

**Interfaces:**
- Consumes: `collab_fetch_transient` internals (Gate 3), `verify_relay_challenge` (Gate 2), `RelayPin` (Gate 1), `rank_strangers`/`StrangerHit`/`TrustGraph` (WS8e), `verify_profile`/`verify_edge`, `TcpConn`, `Profile`/`TrustEdge`/`CollabObject`.
- Produces:
  - `StrangerHit { key, display_name, trust_path, hops, relays: Vec<Key> }` (new field)
  - `pub fn rank_strangers_multi(self_key: &Key, own_edges: &[TrustEdge], per_relay: &[(Key, Vec<CollabObject>)], term: &str, max_hops: u8) -> Vec<StrangerHit>`
  - `pub async fn collab_fetch_authenticated(conn: &mut dyn Conn, pinned_key: &Key) -> Result<Vec<CollabObject>>`
  - `pub async fn query_relay_set(self_key: &Key, own_edges: &[TrustEdge], relays: &[RelayPin], term: &str, max_hops: u8) -> Vec<StrangerHit>`

- [ ] **Step 1: Add `relays: Vec<Key>` to `StrangerHit`** (`src/collab/discovery.rs`) and set `relays: Vec::new()` in the `rank_strangers` construction (the single existing `hits.push(StrangerHit { .. })`). This keeps `rank_strangers` behaviour identical (single-relay/ad-hoc path has empty attribution).

- [ ] **Step 2: Write the failing unit test — `rank_strangers_multi` merges + attributes** (`src/collab/discovery.rs` test module)

```rust
#[tokio::test]
async fn multi_merges_dedups_and_attributes() {
    use crate::collab::{CollabSigner, TrustKind};
    let me = CollabSigner::from_seed([50u8; 32]);
    let x = CollabSigner::from_seed([51u8; 32]);
    let stranger = CollabSigner::from_seed([52u8; 32]);
    let ra = [0xAAu8; 32]; // relay A key
    let rb = [0xBBu8; 32]; // relay B key

    let own_edges = vec![me.sign_edge(x.public_key(), TrustKind::Follow, None, 1)];
    // Relay A supplies the x->stranger edge; relay B supplies the stranger's profile (also A).
    let edge_xs = CollabObject::TrustEdge(x.sign_edge(stranger.public_key(), TrustKind::Follow, None, 1));
    let prof = CollabObject::Profile(stranger.sign_profile("cand".into(), String::new(), vec![], vec![], 1));
    let per_relay = vec![
        (ra, vec![edge_xs.clone(), prof.clone()]),
        (rb, vec![prof.clone()]),
    ];

    let hits = rank_strangers_multi(&me.public_key(), &own_edges, &per_relay, "cand", 4);
    assert_eq!(hits.len(), 1, "stranger deduped to one hit");
    assert_eq!(hits[0].key, stranger.public_key());
    assert_eq!(hits[0].hops, Some(2), "trust path me->x->stranger");
    let mut relays = hits[0].relays.clone();
    relays.sort();
    assert_eq!(relays, vec![ra, rb], "attributed to both relays that served the profile");
}
```

- [ ] **Step 3: Run it, verify it fails** — `cargo test -p bole --lib collab::discovery::tests::multi_merges` → FAIL.

- [ ] **Step 4: Implement `rank_strangers_multi`** (`src/collab/discovery.rs`). Union edges by object-id, dedup profiles by key/highest-seq, attribute each surviving profile to the relays that served it, then call `rank_strangers` over the merged corpus and fill `relays`.

```rust
// <bead-id>
/// Merges per-relay verified corpora and ranks strangers once (WS8e semantics),
/// attributing each hit to the relay keys that served its profile. Profiles are
/// deduped by key (highest `seq` wins); edges are deduped by content id.
pub fn rank_strangers_multi(
    self_key: &Key,
    own_edges: &[TrustEdge],
    per_relay: &[(Key, Vec<CollabObject>)],
    term: &str,
    max_hops: u8,
) -> Vec<StrangerHit> {
    use std::collections::BTreeMap;
    // Dedup profiles by key (highest seq); track serving relays per profile key.
    let mut best: BTreeMap<Key, Profile> = BTreeMap::new();
    let mut served_by: BTreeMap<Key, std::collections::BTreeSet<Key>> = BTreeMap::new();
    // Dedup edges by content id.
    let mut edges: BTreeMap<crate::object::ObjectId, TrustEdge> = BTreeMap::new();
    for (relay_key, corpus) in per_relay {
        for obj in corpus {
            match obj {
                CollabObject::Profile(p) => {
                    served_by.entry(p.key).or_default().insert(*relay_key);
                    match best.get(&p.key) {
                        Some(cur) if cur.seq >= p.seq => {}
                        _ => { best.insert(p.key, p.clone()); }
                    }
                }
                CollabObject::TrustEdge(e) => {
                    let id = crate::codec::object_id(&crate::object::Object::Collab(
                        CollabObject::TrustEdge(e.clone()),
                    ));
                    edges.entry(id).or_insert_with(|| e.clone());
                }
            }
        }
    }
    let merged: Vec<CollabObject> = best
        .values()
        .cloned()
        .map(CollabObject::Profile)
        .chain(edges.values().cloned().map(CollabObject::TrustEdge))
        .collect();
    let mut hits = rank_strangers(self_key, own_edges, &merged, term, max_hops);
    for h in &mut hits {
        if let Some(set) = served_by.get(&h.key) {
            h.relays = set.iter().copied().collect();
        }
    }
    hits
}
```

> Verify the exact content-id helper name: the codebase computes object ids somewhere the object store uses (search `fn object_id`, `hash`, or how `objects.put` derives an id). Use that same function so the dedup id matches how edges are content-addressed. If the only path is `store.put`, dedup edges by their signature bytes + `(from_key,to_key,kind,seq)` tuple instead — pick the one that already exists and note it in the report.

- [ ] **Step 5: Run it, verify it passes** — `cargo test -p bole --lib collab::discovery::tests::multi_merges` → PASS.

- [ ] **Step 6: Implement `collab_fetch_authenticated`** (`src/sync/collab.rs`) — like `collab_fetch_transient` but sends a fresh nonce and verifies the relay signature against the pinned key before accepting the pack.

```rust
// <bead-id>
/// Authenticated transient fetch: sends a fresh single-use nonce, requires the
/// relay to sign it (`verify_relay_challenge` against `pinned_key`), then fetches
/// and verifies objects fail-closed. Errors (no/invalid signature, transport)
/// bubble up so the caller can skip this relay. Writes nothing.
pub async fn collab_fetch_authenticated(
    conn: &mut dyn Conn,
    pinned_key: &Key,
) -> Result<Vec<CollabObject>> {
    use rand::RngCore;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: CapSet::EMPTY,
        intent: Intent::Fetch, client_nonce: Some(nonce),
    }).await?;
    let (refs, relay_sig) = match conn.recv().await? {
        Message::Welcome { refs, relay_sig, .. } => (refs, relay_sig),
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let sig = relay_sig.ok_or_else(|| Error::Storage("relay did not authenticate".into()))?;
    if !crate::collab::verify_relay_challenge(pinned_key, &nonce, &sig) {
        return Err(Error::Storage("relay auth signature invalid".into()));
    }
    let want: Vec<_> = refs.iter().map(|r| r.target).collect();
    conn.send(&Message::HaveWant { want, have: vec![] }).await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
    match conn.recv().await? {
        Message::Done => {}
        other => return Err(Error::Storage(format!("collab: expected Done, got {other:?}"))),
    }
    let mut out = Vec::new();
    for (_id, canonical) in decode_pack(&pack)? {
        if let Ok(Object::Collab(obj)) = crate::codec::deserialize(&canonical) {
            if verified(&obj) {
                out.push(obj);
            }
        }
    }
    Ok(out)
}
```

> Confirm `rand` with `OsRng` is a dependency (ed25519-dalek pulls `rand_core`; check `Cargo.toml` for `rand`). If only `rand_core` is present, use `rand_core::OsRng` + `RngCore`. Do not add a new dependency without noting it; if neither is available, add `rand` to `[dependencies]` and mention it in the report.

- [ ] **Step 7: Implement `query_relay_set`** (`src/sync/collab.rs`) — connect + auth + fetch each pin, skip-and-continue, then `rank_strangers_multi`.

```rust
// <bead-id>
/// Queries every pinned relay: authenticate (possession proof against the pinned
/// key), fetch transiently, verify fail-closed. Unreachable or auth-failing relays
/// are skipped (completeness degrades, soundness holds). Merges + ranks once with
/// per-relay attribution. Mutates no local state.
pub async fn query_relay_set(
    self_key: &Key,
    own_edges: &[TrustEdge],
    relays: &[RelayPin],
    term: &str,
    max_hops: u8,
) -> Vec<StrangerHit> {
    let mut per_relay: Vec<(Key, Vec<CollabObject>)> = Vec::new();
    for pin in relays {
        let stream = match tokio::net::TcpStream::connect(&pin.endpoint).await {
            Ok(s) => s,
            Err(_) => continue, // unreachable: skip
        };
        let mut conn = crate::sync::transport::TcpConn::new(stream);
        match collab_fetch_authenticated(&mut conn, &pin.key).await {
            Ok(objs) => per_relay.push((pin.key, objs)),
            Err(_) => continue, // auth-fail / transport error: skip
        }
    }
    crate::collab::rank_strangers_multi(self_key, own_edges, &per_relay, term, max_hops)
}
```

- [ ] **Step 8: Re-export** `query_relay_set`, `collab_fetch_authenticated`, `rank_strangers_multi` in `src/lib.rs`.

- [ ] **Step 9: Write the loopback tests** (`tests/collab_network.rs`). Reuse the file's existing relay/serve loopback helpers (see the WS8d/e tests). Add:

```rust
// <bead-id>
// Two relays each hold one slice of the chain; a merged query yields the stranger
// with a trust-path spanning both, attributed to both relay keys.
#[tokio::test]
async fn multi_relay_merged_trust_path_and_attribution() { /* ...see below... */ }

// <bead-id>
// A relay serving a bad handshake signature is dropped; the query still completes
// using the honest relay (completeness degraded, soundness intact).
#[tokio::test]
async fn multi_relay_bad_sig_dropped_query_completes() { /* ... */ }

// <bead-id>
// An unreachable relay endpoint is skipped; the query still completes.
#[tokio::test]
async fn multi_relay_unreachable_skipped() { /* ... */ }

// <bead-id>
// A stranger served by BOTH relays appears once (profile dedup), attributed to both.
#[tokio::test]
async fn multi_relay_dedups_shared_stranger() { /* ... */ }
```

Concrete shape for `multi_relay_merged_trust_path_and_attribution` (adapt helpers to the file's existing ones):

```rust
    // Build two relay repos, each with a signer; relay A caches me->x follow + x->stranger... etc.
    // Serve each on a loopback listener via serve_collab_tcp_once(.., true, Some(&signerX)).
    // Pin both: relays = vec![RelayPin{key: a_signer.public_key(), endpoint: a_addr}, RelayPin{..b}].
    let hits = bole::query_relay_set(&me.public_key(), &own_edges, &relays, "Pat", 4).await;
    let hit = hits.iter().find(|h| h.key == stranger).expect("stranger found via merged relays");
    assert!(hit.trust_path.is_some());
    assert!(hit.relays.contains(&a_key) || hit.relays.contains(&b_key));
```

> For the **bad-sig** relay: serve it with `serve_collab_tcp_once(.., true, Some(&WRONG_signer))` where `WRONG_signer`'s key ≠ the pinned key → `collab_fetch_authenticated` rejects it. Assert the query still returns the honest relay's stranger. For **unreachable**: pin an endpoint with no listener (e.g. `127.0.0.1:1` or a closed port) alongside one good relay; assert the good stranger still appears. For **tampered object dropped**: reuse the existing `transient_fetch_drops_tampered` pattern; a tampered object from an authenticated relay is still dropped on `verified()`.

- [ ] **Step 10: Run all new tests** — `cargo test -p bole --test collab_network` and `cargo test -p bole --lib collab::discovery` → PASS. `cargo clippy --workspace` clean.

- [ ] **Step 11: Commit**

```bash
git add src/sync/collab.rs src/collab/discovery.rs src/lib.rs tests/collab_network.rs
git commit -m "<bead-id>: authenticated multi-relay fetch + union merge + attribution"
```

---

## Gate 5 (bead: CLI relay group + set query) — `relay add/remove/list`, `discover relay` over the set, serve signer, E2E migration

**Files:**
- Create: `bole-cli/src/commands/relay.rs`
- Modify: `bole-cli/src/commands/discover.rs` (Relay arm), `bole-cli/src/commands/node.rs` (serve signer), `bole-cli/src/main.rs` (wire `relay`)
- Test: `bole-cli/tests/collab_cli.rs` (migrate 2 existing E2E, add 1 new)

**Interfaces:**
- Consumes: `Repository::{add_relay, remove_relay, relays}` (G1), `RelayPin` (G1), `query_relay_set` (G4), `collab_fetch_transient` (ad-hoc path), `signer_from` (`bole-cli/src/collabkey.rs`), `key::hex32`/`key::parse_hex_32` (`bole-cli/src/key.rs`), `serve_collab_tcp_once` (G3 signature).
- Produces: `relay add/remove/list` verbs; `discover relay <term> [--endpoint <addr>] [--max-hops N] [--json]`; `node serve --relay --key-env/--key-file`.

- [ ] **Step 1: Create `bole-cli/src/commands/relay.rs`** with the command group.

```rust
// <bead-id>
//! `bole relay` — manage the trusted-relay set (local, per-repo).
use crate::key;
use crate::output::Output;
use crate::RepoContext;
use bole::{RelayPin, Result};
use clap::Subcommand;
use std::path::PathBuf; // if needed

#[derive(Subcommand)]
pub enum Cmd {
    /// Pin a trusted relay by its raw 64-hex key + endpoint (upsert).
    Add { key_hex: String, endpoint: String },
    /// Remove a pinned relay by its raw 64-hex key.
    Remove { key_hex: String },
    /// List pinned relays.
    List,
}

pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        // <bead-id>
        Cmd::Add { key_hex, endpoint } => {
            let key = key::parse_hex_32(&key_hex)?;
            ctx.repo.add_relay(RelayPin { key, endpoint }).await?;
            out.emit(|| "relay pinned".to_string(), || serde_json::json!({ "ok": true }));
            Ok(())
        }
        Cmd::Remove { key_hex } => {
            let key = key::parse_hex_32(&key_hex)?;
            let removed = ctx.repo.remove_relay(&key).await?;
            out.emit(
                || if removed { "relay removed".into() } else { "no such relay".into() },
                || serde_json::json!({ "removed": removed }),
            );
            Ok(())
        }
        Cmd::List => {
            let pins = ctx.repo.relays().await?;
            let rows: Vec<_> = pins.iter()
                .map(|p| serde_json::json!({ "key": key::hex32(&p.key), "endpoint": p.endpoint }))
                .collect();
            out.emit(
                || rows.iter().map(|r| format!("{} {}", r["key"], r["endpoint"])).collect::<Vec<_>>().join("\n"),
                || serde_json::json!(rows),
            );
            Ok(())
        }
    }
}
```

> Match `Output::emit`'s exact signature and the `key::parse_hex_32` name to what `bole-cli/src/key.rs` and `output.rs` actually expose (grep them). Follow the module shape of `bole-cli/src/commands/trust.rs`.

- [ ] **Step 2: Wire the `relay` group** in `bole-cli/src/main.rs` — add `mod relay;` under `commands`, a `Relay(commands::relay::Cmd)` top-level subcommand, and a dispatch arm `Command::Relay(c) => commands::relay::run(&ctx, &out, c).await`. Mirror how `Trust`/`Discover` are wired.

- [ ] **Step 3: Rework the `Cmd::Relay` discover arm** (`bole-cli/src/commands/discover.rs`). Change fields: make `endpoint` an optional flag, `term` positional first.

```rust
    // <bead-id>
    /// Search trusted relays for strangers with a verifiable trust path (transient).
    Relay {
        /// Substring to match against profile name/bio/aliases/key.
        term: String,
        /// Ad-hoc: query a single unpinned endpoint (host:port) instead of the pinned set.
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long, default_value_t = 4)]
        max_hops: u8,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
```

Handler:

```rust
        // <bead-id>
        Cmd::Relay { term, endpoint, max_hops, key_env, key_file } => {
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            let mut own_edges = ctx.repo.public_edges().await?;
            for o in ctx.repo.tracked_collab().await? {
                if let bole::CollabObject::TrustEdge(e) = o {
                    own_edges.push(e);
                }
            }
            let hits = match endpoint {
                // Ad-hoc one-off: WS8d behaviour, no pin handshake (still fail-closed verify).
                Some(addr) => {
                    let stream = tokio::net::TcpStream::connect(&addr).await?;
                    let mut conn = TcpConn::new(stream);
                    let corpus = collab_fetch_transient(&mut conn).await?;
                    bole::rank_strangers(&self_key, &own_edges, &corpus, &term, max_hops)
                }
                // Query the pinned set: authenticate each, merge, attribute.
                None => {
                    let relays = ctx.repo.relays().await?;
                    bole::query_relay_set(&self_key, &own_edges, &relays, &term, max_hops).await
                }
            };
            let rows: Vec<_> = hits.iter().map(|h| {
                let trust_path = h.trust_path.as_ref().map(|path| {
                    path.iter().map(|hop| serde_json::json!({
                        "key": key::hex32(&hop.key),
                        "via": match hop.via {
                            bole::TrustKind::Vouch => "vouch",
                            bole::TrustKind::Follow => "follow",
                            bole::TrustKind::Review => "review",
                        },
                    })).collect::<Vec<_>>()
                });
                serde_json::json!({
                    "key": key::hex32(&h.key),
                    "display_name": h.display_name,
                    "reach": "stranger",
                    "trust_path": trust_path,
                    "hops": h.hops,
                    "relays": h.relays.iter().map(key::hex32).collect::<Vec<_>>(),
                })
            }).collect();
            out.emit(
                || {
                    if rows.is_empty() { "no strangers matched".to_string() }
                    else {
                        rows.iter().map(|r| {
                            let hops = if r["hops"].is_null() { "no path".into() }
                                       else { format!("{} hops", r["hops"]) };
                            let nrelays = r["relays"].as_array().map(|a| a.len()).unwrap_or(0);
                            format!("{} [stranger, {}, via {} relays] {}", r["key"], hops, nrelays, r["display_name"])
                        }).collect::<Vec<_>>().join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
```

- [ ] **Step 4: `node serve --relay` requires a signer** (`bole-cli/src/commands/node.rs`). Add `key_env`/`key_file` to `Serve`, require them when `relay`, and pass the signer to `serve_collab_tcp_once`.

```rust
        // <bead-id>
        Cmd::Serve { listen, relay, key_env, key_file } => {
            let relay_signer = if relay {
                Some(signer_from(&key_env, key_file.as_deref())?)
            } else {
                None
            };
            // ... existing listener setup ...
            // in the accept loop, pass the signer:
            serve_collab_tcp_once(&listener, &ctx.repo, relay, relay_signer.as_ref()),
        }
```

> Add `key_env: String` (default `"BOLE_COLLAB_KEY"`) and `key_file: Option<PathBuf>` to the `Serve` variant, plus `use crate::collabkey::signer_from;`. Keep the existing 30-second per-connection timeout wrapper (`bole-g87`) exactly as is, just threading `relay_signer.as_ref()` into the `serve_collab_tcp_once` call.

- [ ] **Step 5: Migrate the two existing E2E tests** in `bole-cli/tests/collab_cli.rs` to the new syntax. Both currently call `["discover", "relay", raddr, "Pat", "--json"]` (lines ~162 and ~214). Change to the ad-hoc flag form:

```rust
    // <bead-id>
    let out = ok(q, &["discover", "relay", "Pat", "--endpoint", raddr, "--json"], Some(&qseed));
```

Also update the relay serve invocations in those tests: `node serve --relay` now needs a key — add `--key-env`/`--key-file` (or an env var) providing the relay's seed, exactly as those tests already provide the querier seed. Assertions (Pat present, hops, via) stay unchanged.

- [ ] **Step 6: Add the WS8f-a E2E** — two relay nodes, pin both, query the set.

```rust
// <bead-id>
#[test]
fn cli_discover_relay_set_merged_attributed() {
    // Two publishers/relays with distinct seeds; each pulls a different author so
    // each serves a slice. Serve both: `node serve --relay --key-file <relaySeed>`.
    // Q: `relay add <relayA_key_hex> <addrA>`, `relay add <relayB_key_hex> <addrB>`.
    // Q: `discover relay "<term>" --json` (NO --endpoint) -> queries the set.
    // Assert: the merged stranger appears with reach "stranger", non-null trust_path,
    // and a non-empty "relays" array. Assert Q's local `refs/collab/` shows the
    // relays/ pins but no new remotes/ (query mutated no neighborhood state).
    // Use valid-hex seeds and fixed ports as the existing E2E tests do.
}
```

> Flesh this out following `cli_discover_relay_trust_path`'s exact structure (process spawn helpers, `ok(...)`, port selection, seed hex). Derive each relay's public key hex to pass to `relay add` via `bole profile show`/the same derivation the other tests use, or by reading the relay's published profile key. Keep ports distinct from the other tests.

- [ ] **Step 7: Build + run the CLI suite**

```bash
cargo build --workspace
cargo test -p bole-cli
cargo clippy --workspace
```

Expected: all `collab_cli` tests pass including the migrated two and the new set-query E2E; clippy clean.

- [ ] **Step 8: Commit**

```bash
git add bole-cli/src/commands/relay.rs bole-cli/src/commands/discover.rs bole-cli/src/commands/node.rs bole-cli/src/main.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: relay set CLI + discover relay over the pinned set + serve signer"
```

---

## Self-Review

**Spec coverage:**
- §2 relay-set storage/CRUD/upsert/one-endpoint-per-key/never-served → Gate 1. ✅
- §3 relay-auth handshake (domain-separated nonce, optional wire fields, relay signs, client verifies, node serve requires signer) → Gate 2 (crypto) + Gate 3 (wire + serve) + Gate 4 (client verify in `collab_fetch_authenticated`). ✅
- §4 multi-relay query, union merge (profile dedup by key/highest-seq, edge dedup by id), single WS8e rank, per-relay attribution, skip-and-continue, no local mutation → Gate 4. ✅
- §5 CLI (`relay add/remove/list`, `discover relay <term>` over set with `relays` attribution, `--endpoint` escape hatch, serve signer) + WS8d E2E migration → Gate 5. ✅
- §6 tests: CRUD/upsert, adverts-exclude-relays, handshake accept/reject-wrong-key/reject-bare-nonce/reject-replay, loopback (merged+attributed, bad-sig dropped, unreachable skipped, tampered dropped, dedup once), CLI E2E → distributed across G1/G2/G3/G4/G5. ✅
- Invariants (relays not authoritative, endpoint read-only, soundness from per-edge verify, transient no-mutation, local depth-2 untouched, keys raw hex) → Global Constraints + carried in each gate. ✅

**Placeholder scan:** Test bodies in Gate 4 Step 9 and Gate 5 Step 6 intentionally reference "reuse the file's existing helpers" because the loopback/process harness already exists in those test files and must be matched, not reinvented — the concrete assertions and the exact library calls under test are fully specified. All library/CLI implementation steps carry complete code.

**Type consistency:** `RelayPin { key: Key, endpoint: String }`, `StrangerHit.relays: Vec<Key>`, `serve_collab(.., relay: bool, relay_signer: Option<&CollabSigner>)`, `collab_fetch_authenticated(conn, pinned_key: &Key)`, `query_relay_set(self_key, own_edges, relays, term, max_hops)`, `rank_strangers_multi(self_key, own_edges, per_relay: &[(Key, Vec<CollabObject>)], term, max_hops)`, `verify_relay_challenge(key, nonce: &[u8;32], sig: &[u8;64])` — names/signatures are consistent across gates and match the Interfaces blocks.

**Open verification items for implementers (named in-step):** the object content-id helper name for edge dedup (G4 S4); `rand`/`OsRng` availability (G4 S6); `Output::emit` and `key::parse_hex_32` exact signatures (G5 S1); the duplex-`Conn` vs loopback shape of existing serve tests (G3 S4). Each is flagged at its step with a fallback.
