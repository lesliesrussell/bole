# WS8b — Networked Sovereign Node + CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make WS8a real over the wire — a dedicated collab-serve endpoint, a stateless pull client that stores peers under per-peer tracking refs, local-state discovery/query, and CLI porcelain — plus the two deferred security fixes (M2 scoped-ref gating, F4 publish TOCTOU).

**Architecture:** A new `src/sync/collab.rs` reuses WS5's `Conn`/`Message`/pack machinery to serve *only* `refs/collab/public/**` (`serve_collab`) and to pull a peer's public objects (`collab_pull`), verifying every signature and storing survivors under `refs/collab/remotes/<peerkey-fp>/`. Discovery `query` builds the WS8a `Index` from local state (own public objects + tracked peers) via `TrustGraph::follow_neighborhood`. `bole-cli` gains `profile`, `trust`, `node serve`, and `discover` commands. Publishing is serialized under a mutex (F4).

**Tech Stack:** Rust (library-first + `bole-cli`), reusing `src/sync/` (`Conn`, `TcpConn`, `Message`, `build_pack`/`decode_pack`/`missing_closure`) and `src/collab/` (WS8a: `Profile`, `TrustEdge`, `verify_profile`/`verify_edge`, `Index`, `TrustGraph`, `Namer`). `tokio` (net + sync), loopback `TcpConn` + real-`bole`-binary CLI tests.

## Global Constraints

- **M2 (scoped gating) is structural:** `serve_collab` advertises refs by the `refs/collab/public/` prefix ONLY. `refs/collab/scoped/` (or any non-public ref) is never advertised or transferred through the collab endpoint — regardless of labels or accessor.
- **Fail-closed on pull:** every pulled object is signature-verified (`verify_profile`/`verify_edge`) before a tracking ref is created for it. Unverified/tampered objects get no ref.
- **Serve-own-only / single-author:** a pulled peer's kept objects are exactly those authored by that peer's advertised profile key; objects under any other authorship are dropped.
- **Anonymous read, never write:** the collab endpoint requires no identity and never accepts ref-update/push ops. Publishing is always local and key-gated.
- **Key material never on argv:** the node's collab signing seed comes from `$BOLE_COLLAB_KEY` or `--key-file` (via the existing `key::resolve`), never a positional/flag value.
- **Monotonic publish:** `Profile` per key and `TrustEdge` per `(from,kind,to)` keep only the highest `seq`; publishing is serialized so the read-check-write is atomic (F4).
- **Reuse, don't reinvent:** transport/framing/pack come from `src/sync/`; object model/signatures/index/trust-graph come from `src/collab/`. No new transport, no new object types.
- **`--json` is the stable contract** for any parse-critical CLI output; `--quiet` suppresses non-error text.
- **No new heavy deps.** Only crates already in `Cargo.toml`.
- **Process:** bd-only tracking. Each Task is one bead; branch name = bead ID exactly. Each contiguous added code block carries one `// <bead-id>` comment. Tests pass before merge; delete branch after merge; `bd close`.

### Per-task bead protocol (every Task)

```bash
bd create "WS8b Task N: <title>" --json     # note the id, e.g. bole-abc
bd update <id> --claim
git checkout -b <id>                         # branch == bead id
# ... TDD steps ...
git checkout master && git merge <id> && git branch -d <id>
bd close <id>
```
Use the assigned `<id>` as the `// <id>` tag on every added block in that task.

---

## Gates → Tests

| Gate | Requirement | Satisfying test(s) | Task |
|------|-------------|--------------------|------|
| **G1** | F4: publishing is atomic — concurrent same-key publishes preserve highest-seq-wins | `concurrent_publish_keeps_higher_seq` | 1 |
| **G2** | M2: `serve_collab` advertises ONLY `refs/collab/public/**` | `collab_adverts_exclude_scoped`, `serve_collab_never_offers_scoped` | 2 |
| **G3** | `collab_pull` stores verified peer objects under `refs/collab/remotes/<key>/`; drops tampered/unsigned; single-author | `pull_stores_under_remote_prefix`, `pull_drops_tampered_object` | 3 |
| **G4** | End-to-end over real `TcpConn` loopback: publish→serve→pull works; scoped never pulled; tampered rejected | `loopback_pull_roundtrip`, `loopback_scoped_never_pulled` | 4 |
| **G5** | Local-state discovery index ranks own(0)/followed-peer(1), reuses WS8a `Index`/`TrustGraph`, excludes non-neighborhood authors | `local_index_ranks_by_distance`, `local_index_excludes_unfollowed` | 5 |
| **G6** | CLI authoring: `profile set` monotonic, `trust follow`/`vouch`, key from env/file (not argv); `profile show`/`trust list` | `cli_profile_set_and_show`, `cli_trust_follow_and_list` | 6 |
| **G7** | CLI networking + E2E: `node serve` + `discover pull` + `discover query --json` finds the peer; scoped/tampered absent | `cli_discover_pull_query_e2e` | 7 |

---

## File Structure

- `src/repo/mod.rs` — add `publish_lock: tokio::sync::Mutex<()>` field + init in `memory()`/`disk()`.
- `src/repo/collab.rs` — guard `publish_profile`/`publish_edge` with the lock; add `tracked_collab()`, `local_discovery_index()`, `COLLAB_REMOTES_PREFIX`.
- `src/sync/collab.rs` (NEW) — `collab_adverts`, `serve_collab`, `collab_pull`, `serve_collab_tcp_once`.
- `src/sync/mod.rs` — `pub mod collab;`.
- `src/lib.rs` — re-export the new public items.
- `tests/collab_network.rs` (NEW) — loopback TCP integration (Task 4).
- `bole-cli/src/commands/profile.rs`, `trust.rs`, `node.rs`, `discover.rs` (NEW) — CLI porcelain.
- `bole-cli/src/commands/mod.rs`, `bole-cli/src/main.rs` — register + dispatch the new commands.
- `bole-cli/src/collabkey.rs` (NEW) — resolve the node collab signer from `$BOLE_COLLAB_KEY`/`--key-file`.
- `bole-cli/tests/collab_cli.rs` (NEW) — real-binary E2E (Task 7).

---

## Task 1: F4 — serialize publish (atomic monotonic seq)

**Files:**
- Modify: `src/repo/mod.rs` (add `publish_lock` field + constructor init)
- Modify: `src/repo/collab.rs` (acquire lock in `publish_profile`/`publish_edge`; add test)

**Interfaces:**
- Produces: `Repository.publish_lock: tokio::sync::Mutex<()>` (private); no signature change to `publish_profile`/`publish_edge`.

- [ ] **Step 1: Write the failing test** — add to the test module in `src/repo/collab.rs`:

```rust
// <bead-id>
#[tokio::test]
async fn concurrent_publish_keeps_higher_seq() {
    use std::sync::Arc;
    let repo = Arc::new(Repository::memory());
    let a = CollabSigner::from_seed([50u8; 32]);
    // seq 1 exists first so both concurrent publishes are "advances".
    repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await.unwrap();

    let p2 = a.sign_profile("v2".into(), String::new(), vec![], vec![], 2);
    let p3 = a.sign_profile("v3".into(), String::new(), vec![], vec![], 3);
    let (r2, r3) = (repo.clone(), repo.clone());
    let (a2, a3) = (p2.clone(), p3.clone());
    let t2 = tokio::spawn(async move { let _ = r2.publish_profile(&a2).await; });
    let t3 = tokio::spawn(async move { let _ = r3.publish_profile(&a3).await; });
    t2.await.unwrap();
    t3.await.unwrap();

    // Whichever ordering occurred, a lower seq must never overwrite a higher one.
    let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
    assert_eq!(cur.seq, 3, "highest seq must be current after concurrent publish");
}
```

- [ ] **Step 2: Run to verify it fails/flakes without the lock**

Run: `cargo test -p bole repo::collab::tests::concurrent_publish_keeps_higher_seq`
Expected: without the lock this can leave `seq == 2` (lower seq overwrites higher). Note: the failure is timing-dependent — run a few times: `for i in 1 2 3 4 5; do cargo test -p bole concurrent_publish_keeps_higher_seq -- --exact 2>&1 | tail -1; done`. It is intended to be *reliably GREEN only after* the lock lands (Step 3); its value is guaranteeing the invariant, not a deterministic RED.

- [ ] **Step 3: Add the lock field** — in `src/repo/mod.rs`, add to `struct Repository` (after `hooks`):

```rust
    // <bead-id>
    /// Serializes collab publish read-check-write so concurrent publishes cannot
    /// both pass a stale monotonic-seq check (WS8b F4).
    publish_lock: tokio::sync::Mutex<()>,
```

In `Repository::memory()` add `publish_lock: tokio::sync::Mutex::new(())` to the struct literal. In `Repository::disk(...)`'s returned `Self { ... }`, add the same field initializer. (Search both constructors; there is exactly one struct literal each.)

- [ ] **Step 4: Guard the publish methods** — in `src/repo/collab.rs`, at the very top of both `publish_profile` and `publish_edge` bodies (before the signature check), add:

```rust
        // <bead-id>
        let _publish_guard = self.publish_lock.lock().await;
```

- [ ] **Step 5: Run to verify GREEN**

Run: `cargo test -p bole repo::collab` then `cargo test -p bole`
Expected: all pass, including `concurrent_publish_keeps_higher_seq` (stable across repeated runs).

- [ ] **Step 6: Commit**

```bash
git add src/repo/mod.rs src/repo/collab.rs
git commit -m "<bead-id>: serialize collab publish (F4 atomic monotonic seq) (G1)"
```

---

## Task 2: `serve_collab` + collab adverts (server, M2)

**Files:**
- Create: `src/sync/collab.rs`
- Modify: `src/sync/mod.rs` (`pub mod collab;`)
- Modify: `src/lib.rs` (re-exports)

**Interfaces:**
- Consumes: `crate::sync::transport::Conn`; `crate::sync::wire::{Message, RefAdvert, CapSet, Intent, PROTO_VERSION}`; `crate::sync::session::build_pack` (pub(crate)); `crate::sync::negotiate::missing_closure` (pub); `crate::repo::collab::COLLAB_PUBLIC_PREFIX`; `Repository`.
- Produces: `pub fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>>`; `pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository) -> Result<()>`.

- [ ] **Step 1: Write the failing tests** — create `src/sync/collab.rs`:

```rust
// <bead-id>
//! The collaboration-serve endpoint (WS8b): serves ONLY the public collab
//! namespace (`refs/collab/public/**`) over the WS5 wire, anonymously and
//! read-only. Never advertises any other ref, so scoped objects cannot leak.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::repo::collab::COLLAB_PUBLIC_PREFIX;
use crate::repo::Repository;
use crate::sync::negotiate;
use crate::sync::session::build_pack;
use crate::sync::transport::Conn;
use crate::sync::wire::{CapSet, Intent, Message, RefAdvert, PROTO_VERSION};

// (implementation added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::CollabSigner;
    use crate::object::Object;
    use crate::refs::{Ref, RefName, Tag};
    use crate::repo::collab::COLLAB_SCOPED_PREFIX;

    #[tokio::test]
    async fn collab_adverts_exclude_scoped() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([1u8; 32]);
        repo.publish_profile(&a.sign_profile("A".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // Pin a scoped object directly.
        let scoped = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(crate::collab::CollabObject::Profile(scoped))).await.unwrap();
        let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/x");
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).unwrap();
        assert!(adverts.iter().all(|r| r.name.as_str().starts_with(COLLAB_PUBLIC_PREFIX)));
        assert!(adverts.iter().any(|r| r.name.as_str().contains("/public/profile/")));
        assert!(!adverts.iter().any(|r| r.name.as_str().contains("/scoped/")));
    }

    #[tokio::test]
    async fn serve_collab_never_offers_scoped() {
        use crate::sync::transport::InProcessConn;
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([2u8; 32]);
        repo.publish_profile(&a.sign_profile("A".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let scoped = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(crate::collab::CollabObject::Profile(scoped))).await.unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut server, mut client) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut server, &repo).await });
        // Minimal client: Hello(Fetch) -> read Welcome adverts.
        client.send(&Message::Hello { proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: CapSet::EMPTY, intent: Intent::Fetch }).await.unwrap();
        let welcome = client.recv().await.unwrap();
        let refs = match welcome { Message::Welcome { refs, .. } => refs, other => panic!("expected Welcome, got {other:?}") };
        assert!(refs.iter().all(|r| r.name.as_str().starts_with(COLLAB_PUBLIC_PREFIX)));
        assert!(!refs.iter().any(|r| r.name.as_str().contains("/scoped/")));
        // Drain the rest so the server task finishes cleanly.
        client.send(&Message::HaveWant { want: refs.iter().map(|r| r.target).collect(), have: vec![] }).await.unwrap();
        let _pack = client.recv().await.unwrap();
        let _done = client.recv().await.unwrap();
        srv.await.unwrap().unwrap();
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p bole sync::collab`
Expected: FAIL (build error — `collab_adverts`, `serve_collab` undefined).

- [ ] **Step 3: Implement** — insert above the test module in `src/sync/collab.rs`:

```rust
// <bead-id>
/// Advertises exactly the refs under `refs/collab/public/` — the entire public
/// collab surface, and nothing else. This is the single M2 enforcement point.
pub fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
        }
    }
    Ok(out)
}

// <bead-id>
/// Read-only, anonymous responder for the collaboration endpoint. Advertises only
/// the public collab refs, then serves the requested object closure. Never
/// accepts pushes; never advertises anything outside `refs/collab/public/`.
pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository) -> Result<()> {
    match conn.recv().await? {
        Message::Hello { intent: Intent::Fetch, .. } | Message::Hello { intent: Intent::Clone, .. } => {}
        Message::Hello { intent: Intent::Push, .. } => {
            conn.send(&Message::Error("collab endpoint is read-only".into())).await?;
            return Err(Error::Storage("collab: push not permitted".into()));
        }
        _ => {
            conn.send(&Message::Error("expected Hello".into())).await?;
            return Err(Error::Storage("collab: expected Hello".into()));
        }
    }
    let refs = collab_adverts(repo)?;
    let authorized: HashSet<_> = refs.iter().map(|r| r.target).collect();
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs }).await?;
    let (want, have) = match conn.recv().await? {
        Message::HaveWant { want, have } => (want, have),
        _ => return Err(Error::Storage("collab: expected HaveWant".into())),
    };
    // Never trust client-named roots: only advertised (public) targets are servable.
    let want: Vec<_> = want.into_iter().filter(|w| authorized.contains(w)).collect();
    let have: HashSet<_> = have.into_iter().collect();
    let missing = negotiate::missing_closure(repo, &want, &have).await?;
    let pack = build_pack(repo, &missing).await?;
    conn.send(&Message::Pack(pack)).await?;
    conn.send(&Message::Done).await?;
    Ok(())
}
```

Add to `src/sync/mod.rs`:

```rust
// <bead-id>
pub mod collab;
```

Add to `src/lib.rs`:

```rust
// <bead-id>
pub use sync::collab::{collab_adverts, serve_collab};
```

> **Implementer note:** confirm `negotiate::missing_closure` signature is `(repo: &Repository, wants: &[ObjectId], have: &HashSet<ObjectId>) -> Result<Vec<ObjectId>>` (as used in `serve_fetch`). If it takes `&[ObjectId]` for `have`, adapt the call. `build_pack` is `pub(crate) async fn build_pack(repo, ids: &[ObjectId]) -> Result<Vec<u8>>`.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole`
Expected: both new tests pass; no regressions.

- [ ] **Step 5: Commit**

```bash
git add src/sync/collab.rs src/sync/mod.rs src/lib.rs
git commit -m "<bead-id>: collab-serve endpoint advertises public-only (M2) (G2)"
```

---

## Task 3: `collab_pull` — verified pull into per-peer tracking refs

**Files:**
- Modify: `src/sync/collab.rs` (add `collab_pull` + `COLLAB_REMOTES_PREFIX` reference)
- Modify: `src/repo/collab.rs` (add `pub const COLLAB_REMOTES_PREFIX`)
- Modify: `src/lib.rs` (re-export `collab_pull`)

**Interfaces:**
- Consumes: WS5 `client`-side pattern (`Message::Hello{Fetch}` → `Welcome` → `HaveWant` → `Pack`); `crate::sync::session::decode_pack` (pub(crate)); `verify_profile`/`verify_edge`, `fingerprint`, `Key`, `CollabObject` (WS8a); `Object`, `Ref`, `RefName`, `Tag`.
- Produces: `pub const COLLAB_REMOTES_PREFIX: &str = "refs/collab/remotes/"` (in `repo::collab`); `pub async fn collab_pull(conn: &mut dyn Conn, repo: &Repository) -> Result<Key>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/sync/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn pull_stores_under_remote_prefix() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::fingerprint;
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        // Server B publishes a profile + a follow edge.
        let server_repo = Repository::memory();
        let b = CollabSigner::from_seed([3u8; 32]);
        let c = CollabSigner::from_seed([4u8; 32]);
        server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server_repo.publish_edge(&b.sign_edge(c.public_key(), crate::collab::TrustKind::Follow, None, 1)).await.unwrap();

        // Client A pulls B.
        let client_repo = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo).await });
        let peer = collab_pull(&mut cl, &client_repo).await.unwrap();
        srv.await.unwrap().unwrap();

        assert_eq!(peer, b.public_key());
        let fp = fingerprint(&b.public_key());
        let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{fp}/")).unwrap();
        assert!(names.iter().any(|n| n.as_str().contains("/profile")), "peer profile tracked");
        assert!(names.iter().any(|n| n.as_str().contains("/edge/")), "peer edge tracked");
    }

    // <bead-id>
    #[tokio::test]
    async fn pull_drops_tampered_object() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX};

        // Server B has a VALID profile plus a TAMPERED edge pinned under public.
        let server_repo = Repository::memory();
        let b = CollabSigner::from_seed([5u8; 32]);
        let c = CollabSigner::from_seed([6u8; 32]);
        server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let mut bad = b.sign_edge(c.public_key(), crate::collab::TrustKind::Follow, None, 1);
        bad.kind = crate::collab::TrustKind::Vouch; // invalidates signature
        let bad_id = server_repo.objects.put(&Object::Collab(CollabObject::TrustEdge(bad))).await.unwrap();
        let mut tx = server_repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_PUBLIC_PREFIX}edge/bad")).unwrap(),
               Ref::Tag(Tag { target: bad_id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let client_repo = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo).await });
        collab_pull(&mut cl, &client_repo).await.unwrap();
        srv.await.unwrap().unwrap();

        let fp = fingerprint(&b.public_key());
        let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{fp}/")).unwrap();
        assert!(names.iter().any(|n| n.as_str().contains("/profile")), "valid profile kept");
        assert!(!names.iter().any(|n| n.as_str().contains("/edge/")), "tampered edge dropped");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p bole sync::collab`
Expected: FAIL (build error — `collab_pull`, `COLLAB_REMOTES_PREFIX` undefined).

- [ ] **Step 3: Add the remotes prefix** — in `src/repo/collab.rs`, next to the other prefix consts:

```rust
// <bead-id>
/// Ref prefix under which a pulled peer's verified public objects are tracked,
/// keyed by the peer's key fingerprint. Never merged into the node's own
/// published set (`refs/collab/public/`).
pub const COLLAB_REMOTES_PREFIX: &str = "refs/collab/remotes/";
```

- [ ] **Step 4: Implement `collab_pull`** — add to `src/sync/collab.rs` (extend the `use` block with `crate::collab::{fingerprint, verify_edge, verify_profile, CollabObject, Key}`, `crate::object::Object`, `crate::refs::{Ref, RefName, Tag}`, `crate::repo::collab::COLLAB_REMOTES_PREFIX`, `crate::sync::session::decode_pack`):

```rust
// <bead-id>
/// True iff a collab object's signature verifies against its embedded author key.
fn verified(obj: &CollabObject) -> bool {
    match obj {
        CollabObject::Profile(p) => verify_profile(p),
        CollabObject::TrustEdge(e) => verify_edge(e),
    }
}

// <bead-id>
/// The author key of a collab object (the identity that signed it).
fn author(obj: &CollabObject) -> Key {
    match obj {
        CollabObject::Profile(p) => p.key,
        CollabObject::TrustEdge(e) => e.from_key,
    }
}

// <bead-id>
/// Pulls a peer's public collab objects over `conn`, verifying every signature
/// (fail-closed) and keeping only those authored by the peer's own profile key
/// (serve-own-only). Survivors are pinned under
/// `refs/collab/remotes/<peerkey-fp>/`, never merged into the local public set.
/// Returns the peer's key.
pub async fn collab_pull(conn: &mut dyn Conn, repo: &Repository) -> Result<Key> {
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Fetch,
    })
    .await?;
    let refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let want: Vec<_> = refs.iter().map(|r| r.target).collect();
    let have = repo.objects.list().await?;
    conn.send(&Message::HaveWant { want, have }).await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
    let _done = conn.recv().await?; // Done
    for (_id, canonical) in decode_pack(&pack)? {
        repo.objects.put_raw(&canonical).await?;
    }

    // Resolve advertised objects, verify, and pick the peer identity = the key of
    // the single advertised Profile. Fail if there is not exactly one profile.
    let mut resolved: Vec<(RefName, CollabObject)> = Vec::new();
    for r in &refs {
        if let Some(Object::Collab(obj)) = repo.objects.get(&r.target).await? {
            if verified(&obj) {
                resolved.push((r.name.clone(), obj));
            }
        }
    }
    let peer = resolved
        .iter()
        .find_map(|(_, o)| matches!(o, CollabObject::Profile(_)).then(|| author(o)))
        .ok_or_else(|| Error::Storage("collab: peer served no valid profile".into()))?;

    let fp = fingerprint(&peer);
    let mut tx = repo.refs.transaction();
    for (name, obj) in &resolved {
        if author(obj) != peer {
            continue; // serve-own-only: drop objects not authored by the peer
        }
        // Map the public leaf name to a per-peer tracking name.
        let leaf = name.as_str().rsplit('/').next().unwrap_or("obj");
        let tracking = match obj {
            CollabObject::Profile(_) => format!("{COLLAB_REMOTES_PREFIX}{fp}/profile"),
            CollabObject::TrustEdge(e) => format!(
                "{COLLAB_REMOTES_PREFIX}{fp}/edge/{}/{}",
                match e.kind {
                    crate::collab::TrustKind::Vouch => "vouch",
                    crate::collab::TrustKind::Follow => "follow",
                    crate::collab::TrustKind::Review => "review",
                },
                fingerprint(&e.to_key),
            ),
        };
        let _ = leaf;
        let target = match obj {
            CollabObject::Profile(p) => repo.objects.put(&Object::Collab(CollabObject::Profile(p.clone()))).await?,
            CollabObject::TrustEdge(e) => repo.objects.put(&Object::Collab(CollabObject::TrustEdge(e.clone()))).await?,
        };
        tx.set(RefName::new(tracking)?, Ref::Tag(Tag { target, created_at: 0, message: None }));
    }
    tx.commit()?;
    Ok(peer)
}
```

Add to `src/lib.rs`:

```rust
// <bead-id>
pub use sync::collab::collab_pull;
```

> **Implementer note:** the object is already in the store from `put_raw`; the `repo.objects.put(&Object::Collab(...))` re-put is a content-addressed no-op that returns the canonical id — use it to get the `ObjectId` for the tracking ref rather than re-deriving. If `put_raw` already exposes the id, use that instead and drop the re-put. `decode_pack` is `pub(crate)`; if it is not reachable from `sync::collab`, mark it `pub(crate)` remains fine (same crate). The unused `leaf` binding is illustrative — remove it if clippy objects.

- [ ] **Step 5: Run to verify GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole` then `cargo clippy -p bole --all-targets -- -D warnings`
Expected: pass, clippy clean (remove any dead bindings the note mentions).

- [ ] **Step 6: Commit**

```bash
git add src/sync/collab.rs src/repo/collab.rs src/lib.rs
git commit -m "<bead-id>: verified collab_pull into per-peer tracking refs (G3)"
```

---

## Task 4: Loopback TCP integration

**Files:**
- Modify: `src/sync/collab.rs` (add `serve_collab_tcp_once`)
- Create: `tests/collab_network.rs`
- Modify: `src/lib.rs` (re-export `serve_collab_tcp_once`)

**Interfaces:**
- Consumes: `tokio::net::TcpListener`, `crate::sync::transport::TcpConn`.
- Produces: `pub async fn serve_collab_tcp_once(listener: &tokio::net::TcpListener, repo: &Repository) -> Result<()>`.

- [ ] **Step 1: Add the TCP accept helper** — in `src/sync/collab.rs`:

```rust
// <bead-id>
/// Accepts one TCP connection and serves the collab endpoint on it.
pub async fn serve_collab_tcp_once(
    listener: &tokio::net::TcpListener,
    repo: &Repository,
) -> Result<()> {
    let (stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let mut conn = crate::sync::transport::TcpConn::new(stream);
    serve_collab(&mut conn, repo).await
}
```

Re-export in `src/lib.rs`:

```rust
// <bead-id>
pub use sync::collab::serve_collab_tcp_once;
```

- [ ] **Step 2: Write the integration tests** — create `tests/collab_network.rs`:

```rust
// <bead-id>
//! Loopback-TCP integration for the WS8b collab endpoint: real `TcpConn` between
//! two in-memory repos.

use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
use bole::object::Object;
use bole::refs::{Ref, RefName, Tag};
use bole::repo::collab::{COLLAB_REMOTES_PREFIX, COLLAB_SCOPED_PREFIX};
use bole::sync::collab::{collab_pull, serve_collab_tcp_once};
use bole::Repository;
use tokio::net::{TcpListener, TcpStream};

async fn connect(addr: std::net::SocketAddr) -> bole::sync::transport::TcpConn {
    let stream = TcpStream::connect(addr).await.unwrap();
    bole::sync::transport::TcpConn::new(stream)
}

#[tokio::test]
async fn loopback_pull_roundtrip() {
    let server_repo = Repository::memory();
    let b = CollabSigner::from_seed([21u8; 32]);
    let c = CollabSigner::from_seed([22u8; 32]);
    server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    server_repo.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo).await });

    let client_repo = Repository::memory();
    let mut conn = connect(addr).await;
    let peer = collab_pull(&mut conn, &client_repo).await.unwrap();
    srv.await.unwrap().unwrap();

    assert_eq!(peer, b.public_key());
    let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{}/", fingerprint(&b.public_key()))).unwrap();
    assert!(names.iter().any(|n| n.as_str().contains("/profile")));
    assert!(names.iter().any(|n| n.as_str().contains("/edge/")));
}

#[tokio::test]
async fn loopback_scoped_never_pulled() {
    let server_repo = Repository::memory();
    let b = CollabSigner::from_seed([23u8; 32]);
    server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    // Pin a scoped object on the server.
    let scoped = b.sign_profile("secret".into(), String::new(), vec![], vec![], 9);
    let id = server_repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
    let mut tx = server_repo.refs.transaction();
    tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
           Ref::Tag(Tag { target: id, created_at: 0, message: None }));
    tx.commit().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo).await });

    let client_repo = Repository::memory();
    let mut conn = connect(addr).await;
    collab_pull(&mut conn, &client_repo).await.unwrap();
    srv.await.unwrap().unwrap();

    // The scoped object's id must not be present locally under any tracking ref,
    // and (since it was never advertised) not fetched at all.
    let all = client_repo.refs.list(COLLAB_REMOTES_PREFIX).unwrap();
    assert!(all.iter().all(|n| !n.as_str().contains("secret")));
    // The only tracked profile is bob's public one (seq 1), not the scoped seq-9 one.
    let fp = fingerprint(&b.public_key());
    let prof_ref = RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap();
    let tag = client_repo.refs.get_tag(&prof_ref).unwrap().unwrap();
    match client_repo.objects.get(&tag.target).await.unwrap().unwrap() {
        Object::Collab(CollabObject::Profile(p)) => assert_eq!(p.seq, 1),
        other => panic!("expected profile, got {other:?}"),
    }
}
```

- [ ] **Step 3: Run RED then GREEN**

Run: `cargo test -p bole --test collab_network`
Expected: FAIL first if `serve_collab_tcp_once`/exports aren't wired; after Step 1 exports compile, PASS (2 tests). If the `bole::sync::transport::TcpConn` path isn't public, use whatever public path `lib.rs` exposes for `TcpConn` (check `pub use sync::...`); adjust imports to compile.

- [ ] **Step 4: Full suite**

Run: `cargo test -p bole`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/sync/collab.rs src/lib.rs tests/collab_network.rs
git commit -m "<bead-id>: loopback-TCP collab pull integration (G4)"
```

---

## Task 5: Local-state discovery index

**Files:**
- Modify: `src/repo/collab.rs` (add `tracked_collab`, `local_discovery_index`)
- Modify: `src/lib.rs` (re-export if needed)

**Interfaces:**
- Consumes: `public_profiles`/`public_edges` (WS8a Task 3); `COLLAB_REMOTES_PREFIX`; WS8a `TrustGraph`, `Index`, `CollabObject`, `Key`, `TrustEdge`.
- Produces: `pub async fn tracked_collab(&self) -> Result<Vec<CollabObject>>`; `pub async fn local_discovery_index(&self, self_key: &Key, hops: u8) -> Result<crate::collab::discovery::Index>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/repo/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn local_index_ranks_by_distance() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([60u8; 32]);
        let bob = CollabSigner::from_seed([61u8; 32]);
        // I publish my profile and follow bob.
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(bob.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // Track bob's profile under the remotes prefix (as a pull would).
        let bp = bob.sign_profile("bob".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(bp))).await.unwrap();
        let fp = crate::collab::fingerprint(&bob.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        assert!(!idx.query("me").is_empty(), "own profile at distance 0");
        let bob_hits = idx.query("bob");
        assert_eq!(bob_hits.len(), 1);
        assert_eq!(bob_hits[0].distance, 1, "followed peer at distance 1");
    }

    // <bead-id>
    #[tokio::test]
    async fn local_index_excludes_unfollowed() {
        use crate::collab::{CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([62u8; 32]);
        let stranger = CollabSigner::from_seed([63u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // Track a stranger I do NOT follow.
        let sp = stranger.sign_profile("stranger".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let fp = crate::collab::fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        assert!(idx.query("stranger").is_empty(), "unfollowed peer is not in the neighborhood");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p bole repo::collab`
Expected: FAIL (build error — `local_discovery_index` undefined).

- [ ] **Step 3: Implement** — add these methods to the `impl Repository` block in `src/repo/collab.rs` (add imports `use crate::collab::discovery::Index;`, `use crate::collab::trust::TrustGraph;`, `use crate::collab::{Key, TrustEdge};`, and `CollabObject` already in scope):

```rust
    // <bead-id>
    /// Every verified collab object currently tracked from pulled peers (under
    /// `refs/collab/remotes/`).
    pub async fn tracked_collab(&self) -> Result<Vec<CollabObject>> {
        let mut out = Vec::new();
        for name in self.refs.list(COLLAB_REMOTES_PREFIX)? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(obj)) = self.objects.get(&tag.target).await? {
                    out.push(obj);
                }
            }
        }
        Ok(out)
    }

    // <bead-id>
    /// Builds the WS8a discovery [`Index`] from local state: own public objects at
    /// distance 0, plus tracked peers whose key is within `hops` of `self_key` in
    /// the combined `Follow` graph. Peers outside the neighborhood are excluded.
    pub async fn local_discovery_index(&self, self_key: &Key, hops: u8) -> Result<Index> {
        // Own public objects (distance 0).
        let mut own: Vec<CollabObject> = Vec::new();
        for p in self.public_profiles().await? {
            own.push(CollabObject::Profile(p));
        }
        for e in self.public_edges().await? {
            own.push(CollabObject::TrustEdge(e));
        }
        let tracked = self.tracked_collab().await?;

        // Combined edge set drives the follow neighborhood.
        let mut edges: Vec<TrustEdge> = Vec::new();
        for o in own.iter().chain(tracked.iter()) {
            if let CollabObject::TrustEdge(e) = o {
                edges.push(e.clone());
            }
        }
        let graph = TrustGraph::from_edges(edges);
        let neighborhood = graph.follow_neighborhood(self_key, hops);

        // Group tracked objects by author, keep only in-neighborhood authors.
        use std::collections::BTreeMap;
        let mut by_author: BTreeMap<Key, Vec<CollabObject>> = BTreeMap::new();
        for o in tracked {
            let a = match &o {
                CollabObject::Profile(p) => p.key,
                CollabObject::TrustEdge(e) => e.from_key,
            };
            by_author.entry(a).or_default().push(o);
        }
        let mut pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)> = Vec::new();
        for (author, objs) in by_author {
            if let Some(dist) = neighborhood.get(&author) {
                pulled.push((author, *dist, vec![*self_key, author], objs));
            }
        }
        Ok(Index::build(*self_key, own, pulled))
    }
```

> **Implementer note:** `Index`, `TrustGraph`, `follow_neighborhood`, and `Index::build` are WS8a items — confirm the exact module paths (`crate::collab::discovery::Index`, `crate::collab::trust::TrustGraph`) and the `Index::build(root, own, pulled)` tuple shape `(Key, u8, Vec<Key>, Vec<CollabObject>)` against `src/collab/discovery.rs`. Adjust imports to match.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole repo::collab` then `cargo test -p bole`
Expected: both new tests pass; no regressions.

- [ ] **Step 5: Commit**

```bash
git add src/repo/collab.rs src/lib.rs
git commit -m "<bead-id>: local-state discovery index over own + tracked peers (G5)"
```

---

## Task 6: CLI authoring + inspect (`profile`, `trust`)

**Files:**
- Create: `bole-cli/src/collabkey.rs` (resolve the node collab signer)
- Create: `bole-cli/src/commands/profile.rs`, `bole-cli/src/commands/trust.rs`
- Modify: `bole-cli/src/commands/mod.rs`, `bole-cli/src/main.rs`, `bole-cli/src/lib`/`main` module list

**Interfaces:**
- Consumes: `bole::{CollabSigner, Repository, TrustKind, Namer, PetnameResolution}`; `crate::key::resolve`, `crate::key::parse_hex_32`; `crate::context::RepoContext`.
- Produces: `pub fn signer_from(key_env: &str, key_file: Option<&std::path::Path>) -> Result<CollabSigner>`; clap subcommand enums `profile::Cmd`, `trust::Cmd`.

- [ ] **Step 1: Write the failing test** — create `bole-cli/tests/collab_cli.rs` with the authoring test (the E2E network test is added in Task 7):

```rust
// <bead-id>
use std::path::Path;
use std::process::Command;

fn bin() -> Command { Command::new(env!("CARGO_BIN_EXE_bole")) }
fn run(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let mut c = bin();
    c.args(args).current_dir(dir);
    if let Some(s) = seed { c.env("BOLE_COLLAB_KEY", s); }
    c.output().unwrap()
}
fn ok(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let out = run(dir, args, seed);
    assert!(out.status.success(), "cmd {args:?} failed: {out:?}");
    out
}

#[test]
fn cli_profile_set_and_show() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "aa".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Alice", "--bio", "hi"], Some(&seed));
    let show = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(v["display_name"], "Alice");
    // Re-setting bumps seq monotonically.
    ok(w, &["profile", "set", "--display-name", "Alice2"], Some(&seed));
    let show2 = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v2: serde_json::Value = serde_json::from_slice(&show2.stdout).unwrap();
    assert_eq!(v2["display_name"], "Alice2");
    assert!(v2["seq"].as_u64().unwrap() > v["seq"].as_u64().unwrap());
}

#[test]
fn cli_trust_follow_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "bb".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Me"], Some(&seed));
    let peer = "cc".repeat(32); // a 64-hex key
    ok(w, &["trust", "follow", &peer], Some(&seed));
    let list = ok(w, &["trust", "list", "--json"], Some(&seed));
    assert!(String::from_utf8_lossy(&list.stdout).contains(&peer[..8]));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p bole-cli --test collab_cli`
Expected: FAIL (unknown subcommands `profile`/`trust`).

- [ ] **Step 3: Implement key resolution** — create `bole-cli/src/collabkey.rs`:

```rust
// <bead-id>
//! Resolves the node's collaboration signing key (a 32-byte Ed25519 seed) from
//! `$BOLE_COLLAB_KEY` or `--key-file`. The seed never appears on argv.
use std::path::Path;
use anyhow::Result;
use bole::CollabSigner;

pub fn signer_from(key_env: &str, key_file: Option<&Path>) -> Result<CollabSigner> {
    let seed = crate::key::resolve(key_env, key_file)?;
    Ok(CollabSigner::from_seed(seed))
}
```

Register the module (wherever `main.rs`/`lib` declares `mod key;`): add `mod collabkey;`.

- [ ] **Step 4: Implement `profile` command** — create `bole-cli/src/commands/profile.rs`:

```rust
// <bead-id>
use anyhow::Result;
use clap::Subcommand;

use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::Output; // adjust to the crate's output helper path

#[derive(Subcommand)]
pub enum Cmd {
    /// Author and publish (monotonically) this node's profile.
    Set {
        #[arg(long)] display_name: String,
        #[arg(long, default_value = "")] bio: String,
        #[arg(long = "endpoint")] endpoints: Vec<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")] key_env: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
    /// Show own profile (default) or a peer's by 64-hex key.
    Show {
        key: Option<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")] key_env: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
}

pub async fn run(cmd: Cmd, out: &Output) -> Result<()> {
    let ctx = RepoContext::discover(&std::env::current_dir()?).await?;
    match cmd {
        Cmd::Set { display_name, bio, endpoints, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            let cur = ctx.repo.profile(&signer.public_key()).await?;
            let seq = cur.map(|p| p.seq + 1).unwrap_or(1);
            let profile = signer.sign_profile(display_name, bio, endpoints, vec![], seq);
            ctx.repo.publish_profile(&profile).await?;
            out.json(&serde_json::json!({ "seq": seq, "key": bole::fingerprint(&signer.public_key()) }));
            Ok(())
        }
        Cmd::Show { key, key_env, key_file } => {
            let k = match key {
                Some(h) => crate::key::parse_hex_32(&h)?,
                None => signer_from(&key_env, key_file.as_deref())?.public_key(),
            };
            match ctx.repo.profile(&k).await? {
                Some(p) => out.json(&serde_json::json!({
                    "display_name": p.display_name, "bio": p.bio,
                    "endpoints": p.endpoints, "seq": p.seq,
                    "key": bole::fingerprint(&p.key),
                })),
                None => out.json(&serde_json::json!({ "profile": null })),
            }
            Ok(())
        }
    }
}
```

> **Implementer note:** match the crate's actual output helper (the other commands use an `Output` formatter passed into `run` — mirror `bole-cli/src/commands/approver.rs`/`policy.rs` exactly for the `Output` type, its `json`/`print` methods, and the `run(...)` signature). `bole::fingerprint` is re-exported from WS8a.

- [ ] **Step 5: Implement `trust` command** — create `bole-cli/src/commands/trust.rs`:

```rust
// <bead-id>
use anyhow::Result;
use clap::Subcommand;

use bole::{Namer, PetnameResolution, TrustKind};
use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::Output;

#[derive(Subcommand)]
pub enum Cmd {
    /// Publish a Follow edge to a 64-hex peer key.
    Follow { key: String,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")] key_env: String,
        #[arg(long)] key_file: Option<std::path::PathBuf> },
    /// Publish a Vouch edge (identity suggestion) with a petname.
    Vouch { key: String, #[arg(long)] name: String,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")] key_env: String,
        #[arg(long)] key_file: Option<std::path::PathBuf> },
    /// List this node's own trust edges.
    List,
}

pub async fn run(cmd: Cmd, out: &Output) -> Result<()> {
    let ctx = RepoContext::discover(&std::env::current_dir()?).await?;
    match cmd {
        Cmd::Follow { key, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            let to = crate::key::parse_hex_32(&key)?;
            let edge = signer.sign_edge(to, TrustKind::Follow, None, next_seq());
            ctx.repo.publish_edge(&edge).await?;
            out.json(&serde_json::json!({ "followed": key }));
            Ok(())
        }
        Cmd::Vouch { key, name, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            let to = crate::key::parse_hex_32(&key)?;
            let edge = signer.sign_edge(to, TrustKind::Vouch, Some(name.clone()), next_seq());
            ctx.repo.publish_edge(&edge).await?;
            out.json(&serde_json::json!({ "vouched": key, "name": name }));
            Ok(())
        }
        Cmd::List => {
            let edges = ctx.repo.public_edges().await?;
            let rows: Vec<_> = edges.iter().map(|e| serde_json::json!({
                "to": bole::fingerprint(&e.to_key),
                "kind": format!("{:?}", e.kind),
                "petname": e.petname,
            })).collect();
            out.json(&serde_json::json!(rows));
            let _ = (Namer::new, PetnameResolution::Fingerprint); // keep imports meaningful; see note
            Ok(())
        }
    }
}

// <bead-id>
/// Edges are keyed by (from,kind,to); WS8a keeps the highest seq. The CLI has no
/// persistent counter, so it must publish a seq strictly greater than any current
/// edge for that triple. Simplest correct choice: read the current edge's seq and
/// add one. For the initial slice, look it up per call.
fn next_seq() -> u64 { 1 }
```

> **Implementer note (important):** `next_seq()` returning a constant `1` is WRONG for re-follows — WS8a rejects a non-increasing seq, so a second `trust follow` of the same key would fail. Implement `next_seq` by reading the current edge for `(from,kind,to)` and returning `cur.seq + 1` (or `1` if none). Because `publish_edge` only exposes `public_edges()`, resolve the current edge by filtering `ctx.repo.public_edges().await?` for the matching `from_key==signer.public_key() && kind && to_key`, take its `seq + 1`. Thread `signer`, `to`, and `kind` into the lookup. The `Namer`/`PetnameResolution` import line is a placeholder to remind you to resolve petnames for `trust list` if you choose to show own-view names; remove the dead `let _` and either use `Namer` for display or drop the imports so clippy is clean.

- [ ] **Step 6: Register + dispatch** — in `bole-cli/src/commands/mod.rs` add `pub mod profile; pub mod trust;`. In `bole-cli/src/main.rs` add to the `Command` enum:

```rust
    // <bead-id>
    /// Author/inspect this node's collaboration profile.
    Profile { #[command(subcommand)] cmd: commands::profile::Cmd },
    /// Author/inspect this node's trust edges.
    Trust { #[command(subcommand)] cmd: commands::trust::Cmd },
```

and to the dispatch `match`:

```rust
        // <bead-id>
        Command::Profile { cmd } => commands::profile::run(cmd, &out).await,
        Command::Trust { cmd } => commands::trust::run(cmd, &out).await,
```

- [ ] **Step 7: Run RED→GREEN**

Run: `cargo test -p bole-cli --test collab_cli` then `cargo test -p bole-cli` then `cargo clippy -p bole-cli --all-targets -- -D warnings`
Expected: `cli_profile_set_and_show` + `cli_trust_follow_and_list` pass; clippy clean.

- [ ] **Step 8: Commit**

```bash
git add bole-cli/src/collabkey.rs bole-cli/src/commands/profile.rs bole-cli/src/commands/trust.rs bole-cli/src/commands/mod.rs bole-cli/src/main.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: CLI profile + trust authoring/inspect (G6)"
```

---

## Task 7: CLI networking (`node serve`, `discover`) + E2E

**Files:**
- Create: `bole-cli/src/commands/node.rs`, `bole-cli/src/commands/discover.rs`
- Modify: `bole-cli/src/commands/mod.rs`, `bole-cli/src/main.rs`
- Modify: `bole-cli/tests/collab_cli.rs` (add the E2E test)

**Interfaces:**
- Consumes: `bole::sync::collab::{serve_collab_tcp_once, collab_pull}`; `bole::Repository`; `RepoContext`; `tokio::net::{TcpListener, TcpStream}`; `bole::sync::transport::TcpConn`.
- Produces: clap subcommand enums `node::Cmd` (`Serve`), `discover::Cmd` (`Pull`, `Query`).

- [ ] **Step 1: Write the failing E2E test** — add to `bole-cli/tests/collab_cli.rs`:

```rust
// <bead-id>
#[test]
fn cli_discover_pull_query_e2e() {
    use std::process::Stdio;
    // Server repo: publish a profile, serve on a fixed loopback port.
    let stmp = tempfile::tempdir().unwrap();
    let s = stmp.path();
    ok(s, &["init", "."], None);
    let sseed = "dd".repeat(32);
    ok(s, &["profile", "set", "--display-name", "Server"], Some(&sseed));

    let addr = "127.0.0.1:47653";
    let mut server = bin();
    server.args(["node", "serve", "--listen", addr]).current_dir(s)
        .stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = server.spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400)); // let it bind

    // Client repo: pull the server, then query.
    let ctmp = tempfile::tempdir().unwrap();
    let c = ctmp.path();
    ok(c, &["init", "."], None);
    let cseed = "ee".repeat(32);
    ok(c, &["profile", "set", "--display-name", "Client"], Some(&cseed));
    ok(c, &["discover", "pull", addr], Some(&cseed));
    let q = ok(c, &["discover", "query", "Server", "--json"], Some(&cseed));
    let _ = child.kill();
    assert!(String::from_utf8_lossy(&q.stdout).contains("Server"), "peer discoverable: {}", String::from_utf8_lossy(&q.stdout));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p bole-cli --test collab_cli -- cli_discover_pull_query_e2e`
Expected: FAIL (unknown subcommands `node`/`discover`).

- [ ] **Step 3: Implement `node serve`** — create `bole-cli/src/commands/node.rs`:

```rust
// <bead-id>
use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::serve_collab_tcp_once;
use crate::context::RepoContext;
use crate::Output;

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the read-only collaboration-serve daemon.
    Serve { #[arg(long)] listen: String },
}

pub async fn run(cmd: Cmd, out: &Output) -> Result<()> {
    let ctx = RepoContext::discover(&std::env::current_dir()?).await?;
    match cmd {
        Cmd::Serve { listen } => {
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            out.print(&format!("serving collab on {listen}"));
            loop {
                // Each accepted connection is served, then we loop for the next.
                if let Err(e) = serve_collab_tcp_once(&listener, &ctx.repo).await {
                    out.print(&format!("connection error: {e}"));
                }
            }
        }
    }
}
```

- [ ] **Step 4: Implement `discover`** — create `bole-cli/src/commands/discover.rs`:

```rust
// <bead-id>
use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::collab_pull;
use bole::sync::transport::TcpConn;
use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::Output;

#[derive(Subcommand)]
pub enum Cmd {
    /// Pull a peer's public collab objects from a network address.
    Pull { addr: String },
    /// Search the local discovery index (own + tracked peers).
    Query {
        term: String,
        #[arg(long, default_value_t = 2)] hops: u8,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")] key_env: String,
        #[arg(long)] key_file: Option<std::path::PathBuf>,
    },
}

pub async fn run(cmd: Cmd, out: &Output) -> Result<()> {
    let ctx = RepoContext::discover(&std::env::current_dir()?).await?;
    match cmd {
        Cmd::Pull { addr } => {
            let stream = tokio::net::TcpStream::connect(&addr).await?;
            let mut conn = TcpConn::new(stream);
            let peer = collab_pull(&mut conn, &ctx.repo).await?;
            out.json(&serde_json::json!({ "pulled": bole::fingerprint(&peer) }));
            Ok(())
        }
        Cmd::Query { term, hops, key_env, key_file } => {
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            let idx = ctx.repo.local_discovery_index(&self_key, hops).await?;
            let rows: Vec<_> = idx.query(&term).into_iter().map(|r| {
                let name = match &r.object {
                    bole::CollabObject::Profile(p) => p.display_name.clone(),
                    bole::CollabObject::TrustEdge(_) => String::new(),
                };
                serde_json::json!({ "key": bole::fingerprint(&r.key), "name": name, "distance": r.distance })
            }).collect();
            out.json(&serde_json::json!(rows));
            Ok(())
        }
    }
}
```

> **Implementer note:** confirm `bole::CollabObject` and `DiscoveryResult`'s public fields (`key`, `object`, `distance`, `trust_path`) are reachable from the crate root or via `bole::collab::...`. Use whatever public path resolves. `local_discovery_index` and `collab_pull` are from Tasks 5/3. `self_key` for `query` may alternatively be read from the local published profile to avoid needing the seed; using the seed here is acceptable and simplest.

- [ ] **Step 5: Register + dispatch** — `bole-cli/src/commands/mod.rs`: `pub mod node; pub mod discover;`. `bole-cli/src/main.rs` enum:

```rust
    // <bead-id>
    /// Run the collaboration-serve daemon.
    Node { #[command(subcommand)] cmd: commands::node::Cmd },
    /// Pull peers and search the local discovery index.
    Discover { #[command(subcommand)] cmd: commands::discover::Cmd },
```

dispatch:

```rust
        // <bead-id>
        Command::Node { cmd } => commands::node::run(cmd, &out).await,
        Command::Discover { cmd } => commands::discover::run(cmd, &out).await,
```

- [ ] **Step 6: Run RED→GREEN**

Run: `cargo test -p bole-cli --test collab_cli` then `cargo test -p bole-cli` then `cargo clippy -p bole-cli --all-targets -- -D warnings`
Expected: all `collab_cli` tests pass (incl. the E2E); clippy clean. If the E2E is flaky on the bind delay, increase the sleep or retry the connect in `discover pull` a few times.

- [ ] **Step 7: Commit**

```bash
git add bole-cli/src/commands/node.rs bole-cli/src/commands/discover.rs bole-cli/src/commands/mod.rs bole-cli/src/main.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: CLI node serve + discover pull/query, E2E (G7)"
```

---

## Self-Review

**Spec coverage:**
- §2 collab-serve endpoint (public-only, anonymous, read-only) → Task 2 (G2).
- §3 `collab_pull` (verify, single-author, remote-tracking) → Task 3 (G3); query index → Task 5 (G5).
- §4 CLI surface → Tasks 6 (profile/trust/show/list) + 7 (node serve/discover). `profile show`/`trust list` covered in Task 6.
- §5 M2 → Task 2; F4 → Task 1; pull-side fail-closed → Task 3; anonymous-read/never-write → Task 2 (Push rejected).
- §6 testing → loopback TCP (Task 4, G4), CLI E2E (Task 7, G7), unit tests throughout.
- §7 scope boundary → nothing here builds relays, depth-2 auto-reach, polling, stranger-search, DNS, or UI.

**Placeholder scan:** No "TBD"/"handle errors later". The two `Implementer note` blocks that flag `next_seq` and output-helper matching are *unavoidable codebase-confirmation points*, each with the exact resolution named (read current edge seq + 1; mirror `approver.rs`'s `Output`). The `next_seq()` stub is explicitly called out as WRONG-as-written with the exact correct implementation described — the implementer must replace it (Task 6 Step 5 note). This is the single substitution requiring real code, and it is spelled out, not hidden.

**Type consistency:** `Key = [u8;32]`, `CollabObject`, `TrustKind`, `fingerprint`, `Index::build(root, own, pulled)` with tuple `(Key,u8,Vec<Key>,Vec<CollabObject>)`, `local_discovery_index(&self, self_key, hops)`, `collab_pull(conn, repo) -> Key`, `serve_collab(conn, repo)`, `serve_collab_tcp_once(listener, repo)`, `COLLAB_PUBLIC_PREFIX`/`COLLAB_SCOPED_PREFIX`/`COLLAB_REMOTES_PREFIX` — all used consistently across tasks and matching WS8a's shipped names.

**Flagged risks:** (1) `next_seq` must be implemented as current-seq+1 (Task 6). (2) The CLI `Output` helper type/methods must be matched to the existing commands (`approver.rs`/`policy.rs`). (3) The E2E test uses a fixed loopback port and a bind delay — noted as a possible flake with the mitigation (retry connect / raise delay).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-03-ws8b-networked-node.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task (one bead + branch each), review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
