# WS8f-b — Server-Side `Search` Verb Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a relay filter by search term server-side — returning only matching profiles + the bounded edge-ball needed for client trust-paths — instead of shipping its whole aggregate, with a transparent fallback and byte-identical results.

**Architecture:** A pure library algorithm (`search_ball`) computes the match set + directed reverse-reachability edge ball. A new capability-negotiated `Search { term, max_hops }` wire verb lets a relay run that algorithm over its served corpus and answer with a `Pack`. Client `collab_search*` functions negotiate `CAP_SEARCH`, send `Search`, verify the pack fail-closed, and transparently fall back to the WS8f-a whole-aggregate exchange when the relay lacks the capability. `query_relay_set` and the ad-hoc CLI path swap to the search functions; results feed the unchanged `rank_strangers_multi`.

**Tech Stack:** Rust, tokio, `postcard`, `ed25519-dalek`, `blake3`, loopback `TcpConn` + real-binary CLI tests.

## Global Constraints

- **ZERO code without a bead.** Each Gate is one bead; branch = bead ID exactly; each contiguous added block gets a `// <bead-id>` comment (ID only, one per contiguous block). Use `bd` for tracking.
- **Relays never authoritative over objects.** Every returned profile/edge is verified fail-closed (`verify_profile`/`verify_edge` via the existing `verified()`), against its embedded author key. Server-side filtering selects which signed objects to send; it never forges/re-attributes/mutates.
- **Soundness from per-edge verification, not relay honesty.** A lying relay's filter degrades **completeness**, never **soundness**. Injected objects fail verify and are dropped; withheld objects only reduce what is found.
- **Endpoint read-only.** `Search` is a read; no write/announce path.
- **Relay-auth gates bytes, not object trust.** The WS8f-a handshake (nonce in `Hello`, sig in `Welcome`, verified vs pinned key) is unchanged and composes with Search; it never blesses objects.
- **Transient query, no local mutation.** Search queries persist nothing. `refs/collab/relays/` is written only by `relay add`/`remove`.
- **Local depth-2 query untouched.** `discover query` and `follow_*` behavior unchanged.
- **Match-rule parity.** The relay's server-side filter matches EXACTLY the fields `rank_strangers` matches: `display_name`, `bio`, any `dns_aliases` entry, and raw key hex (`key_hex`). No divergence — else Search and fallback would return different results.
- **Directed reverse-reachability ball.** Per match, BFS the aggregate graph BACKWARD (`to_key → from_key`) up to `max_hops`, collecting traversed edges; the returned edge set is the **union across matches, deduped by object-id**. Edges pointing away from a match (not on any forward ≤`max_hops` path into it) are NOT returned.
- **Served corpus = `collab_adverts(repo, relay)` objects.** `public/**` + (relay) all `remotes/**`; NEVER `scoped/` or `relays/`.
- **No CLI surface change.** `discover relay <term> [--max-hops N] [--endpoint <addr>]` is unchanged; Search is a transparent optimization beneath it.
- **Keys raw hex.**

---

## File Structure

- **Create** `src/collab/search.rs` — pure `search_ball(corpus, term, max_hops) -> Vec<CollabObject>` (match set + directed reverse-reachability ball). One responsibility: the server-side selection algorithm, testable without any I/O.
- **Modify** `src/collab/mod.rs` — `mod search; pub use search::search_ball;`.
- **Modify** `src/sync/wire.rs` — `CAP_SEARCH` const + `CapSet::contains`; `Message::Search { term, max_hops }` variant; round-trip test.
- **Modify** `src/sync/collab.rs` — `serve_collab` advertises `CAP_SEARCH` in relay mode + handles the `Search` arm; new `collab_search` / `collab_search_authenticated`; `query_relay_set` switches to `collab_search_authenticated`.
- **Modify** `src/lib.rs` — re-export `search_ball`, `collab_search`, `collab_search_authenticated` as needed.
- **Modify** `bole-cli/src/commands/discover.rs` — the ad-hoc `--endpoint` branch swaps `collab_fetch_transient` → `collab_search`.
- **Modify** `tests/collab_network.rs` — loopback Search + fallback + authenticated + multi-relay parity tests.
- **Modify** `bole-cli/tests/collab_cli.rs` — E2E: `discover relay` against a Search node returns the same result.

Gate order: G1 (pure algorithm) → G2 (wire + serve) → G3 (client + fallback + query_relay_set) → G4 (CLI ad-hoc swap + E2E). Each Gate is one bead.

---

## Gate 1 (bead: search algorithm) — pure `search_ball`

**Files:**
- Create: `src/collab/search.rs`
- Modify: `src/collab/mod.rs`, `src/lib.rs`
- Test: unit tests in `src/collab/search.rs`

**Interfaces:**
- Consumes: `CollabObject` (`Profile`/`TrustEdge`), `Key`, `key_hex` (`src/collab/mod.rs`), `codec::object_id`/`codec::serialize` (`src/codec.rs`, `pub(crate)`/`pub`), `Object` (`src/object`).
- Produces: `pub fn search_ball(corpus: &[CollabObject], term: &str, max_hops: u8) -> Vec<CollabObject>` — the matching profiles + the deduped directed reverse-reachability edge ball (`max_hops` = number of edges of reverse depth).

- [ ] **Step 1: Write the failing tests** (in `src/collab/search.rs`, `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabSigner, CollabObject, TrustKind, key_hex};

    // Helper: sign a profile / edge into CollabObjects.
    fn prof(s: &CollabSigner, name: &str, bio: &str) -> CollabObject {
        CollabObject::Profile(s.sign_profile(name.into(), bio.into(), vec![], vec![], 1))
    }
    fn edge(from: &CollabSigner, to: &CollabSigner) -> CollabObject {
        CollabObject::TrustEdge(from.sign_edge(to.public_key(), TrustKind::Follow, None, 1))
    }

    #[test]
    fn matches_same_fields_as_rank_strangers_incl_key_hex() {
        let a = CollabSigner::from_seed([1u8; 32]);
        let corpus = vec![prof(&a, "zzz", "")]; // name/bio do NOT contain the term
        // Match on raw key hex substring only.
        let hex = key_hex(&a.public_key());
        let hits = search_ball(&corpus, &hex, 4);
        assert_eq!(hits.len(), 1, "profile found by its raw key hex");
        // A non-matching term selects nothing.
        assert!(search_ball(&corpus, "no-such-term", 4).is_empty());
    }

    #[test]
    fn reverse_ball_completeness_and_bound() {
        // Chain a -> b -> c -> target (Follow edges). Search for "target".
        let a = CollabSigner::from_seed([2u8; 32]);
        let b = CollabSigner::from_seed([3u8; 32]);
        let c = CollabSigner::from_seed([4u8; 32]);
        let t = CollabSigner::from_seed([5u8; 32]);
        let e_ab = edge(&a, &b);
        let e_bc = edge(&b, &c);
        let e_ct = edge(&c, &t);
        let corpus = vec![prof(&t, "target", ""), e_ab.clone(), e_bc.clone(), e_ct.clone()];

        // max_hops >= 3: all three edges are within the reverse ball of target.
        let hits = search_ball(&corpus, "target", 3);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert_eq!(edges.len(), 3, "all 3 chain edges returned at max_hops=3");

        // max_hops = 2: the far edge a->b (reverse depth 3) is dropped.
        let hits2 = search_ball(&corpus, "target", 2);
        let edges2: Vec<_> = hits2.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert_eq!(edges2.len(), 2, "only c->t and b->c within reverse depth 2");
    }

    #[test]
    fn directedness_edge_away_from_match_excluded() {
        // target -> x (edge points AWAY from target; not on any forward path INTO target).
        let t = CollabSigner::from_seed([6u8; 32]);
        let x = CollabSigner::from_seed([7u8; 32]);
        let e_tx = edge(&t, &x);
        let corpus = vec![prof(&t, "target", ""), e_tx];
        let hits = search_ball(&corpus, "target", 4);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert!(edges.is_empty(), "an edge leaving the match is not reverse-reachable");
    }

    #[test]
    fn multi_match_union_deduped() {
        // Two matches share a common predecessor edge s -> shared; shared -> t1, shared -> t2.
        let s = CollabSigner::from_seed([8u8; 32]);
        let shared = CollabSigner::from_seed([9u8; 32]);
        let t1 = CollabSigner::from_seed([10u8; 32]);
        let t2 = CollabSigner::from_seed([11u8; 32]);
        let e_s_shared = edge(&s, &shared);
        let e_shared_t1 = CollabObject::TrustEdge(shared.sign_edge(t1.public_key(), TrustKind::Follow, None, 1));
        let e_shared_t2 = CollabObject::TrustEdge(shared.sign_edge(t2.public_key(), TrustKind::Follow, None, 1));
        let corpus = vec![
            prof(&t1, "target-one", ""), prof(&t2, "target-two", ""),
            e_s_shared, e_shared_t1, e_shared_t2,
        ];
        // Term "target" matches both t1 and t2.
        let hits = search_ball(&corpus, "target", 4);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        // e_s_shared appears exactly once (shared reverse ball), plus the two shared->tN edges.
        assert_eq!(edges.len(), 3, "union of reverse balls, shared edge deduped once");
        let profs = hits.iter().filter(|o| matches!(o, CollabObject::Profile(_))).count();
        assert_eq!(profs, 2, "both matching profiles returned");
    }
}
```

- [ ] **Step 2: Run them, verify they fail** — `cargo test -p bole --lib collab::search` → FAIL (module/fn absent).

- [ ] **Step 3: Implement `src/collab/search.rs`**

```rust
// <bead-id>
//! Server-side relay search: given a served corpus of verified collaboration
//! objects, select the profiles matching a term and the DIRECTED
//! reverse-reachability edge ball around them — the exact edge set a client's
//! forward (`from_key -> to_key`), `<= max_hops` trust-path BFS into a match can
//! traverse. Filtering only; trust stays client-side (the client re-verifies).
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::collab::{key_hex, CollabObject, Key, Profile, TrustEdge};
use crate::object::{Object, ObjectId};

// <bead-id>
/// True iff `p` matches `term` on the SAME fields as `rank_strangers`:
/// display_name, bio, any dns_alias, or the raw key hex.
fn profile_matches(p: &Profile, term: &str) -> bool {
    p.display_name.contains(term)
        || p.bio.contains(term)
        || p.dns_aliases.iter().any(|a| a.contains(term))
        || key_hex(&p.key).contains(term)
}

// <bead-id>
/// The content id of an edge, matching how `ObjectStore::put` addresses objects.
fn edge_id(e: &TrustEdge) -> ObjectId {
    let bytes = crate::codec::serialize(&Object::Collab(CollabObject::TrustEdge(e.clone())))
        .expect("postcard serialization is infallible for owned data");
    crate::codec::object_id(&bytes)
}

// <bead-id>
/// Matching profiles + the deduped union of directed reverse-reachability edge
/// balls around each match. `max_hops` bounds the reverse BFS depth (in edges).
pub fn search_ball(corpus: &[CollabObject], term: &str, max_hops: u8) -> Vec<CollabObject> {
    // Partition the corpus.
    let mut profiles: Vec<&Profile> = Vec::new();
    let mut edges: Vec<&TrustEdge> = Vec::new();
    for o in corpus {
        match o {
            CollabObject::Profile(p) => profiles.push(p),
            CollabObject::TrustEdge(e) => edges.push(e),
        }
    }

    // Reverse adjacency: for a node `n`, the edges whose `to_key == n` (i.e. edges
    // pointing INTO `n`), so BFS backward walks predecessors.
    let mut incoming: BTreeMap<Key, Vec<&TrustEdge>> = BTreeMap::new();
    for e in &edges {
        incoming.entry(e.to_key).or_default().push(e);
    }

    let mut out: Vec<CollabObject> = Vec::new();
    let mut ball: BTreeSet<ObjectId> = BTreeSet::new();

    for p in &profiles {
        if !profile_matches(p, term) {
            continue;
        }
        out.push(CollabObject::Profile((*p).clone()));
        // Reverse BFS from the matched stranger's key, bounded by max_hops edges.
        // Frontier holds (node, depth-of-node-from-match).
        let mut visited: BTreeSet<Key> = BTreeSet::new();
        let mut q: VecDeque<(Key, u8)> = VecDeque::new();
        q.push_back((p.key, 0));
        visited.insert(p.key);
        while let Some((node, depth)) = q.pop_front() {
            if depth >= max_hops {
                continue; // no edge at this depth would be within max_hops
            }
            if let Some(preds) = incoming.get(&node) {
                for e in preds {
                    // This edge (from -> node) is at reverse depth `depth + 1` <= max_hops.
                    let id = edge_id(e);
                    if ball.insert(id) {
                        out.push(CollabObject::TrustEdge((*e).clone()));
                    }
                    if visited.insert(e.from_key) {
                        q.push_back((e.from_key, depth + 1));
                    }
                }
            }
        }
    }
    out
}
```

- [ ] **Step 4: Register the module** — in `src/collab/mod.rs` add (near the other `mod`/`pub use`):

```rust
// <bead-id>
mod search;
pub use search::search_ball;
```

- [ ] **Step 5: Run tests, verify pass** — `cargo test -p bole --lib collab::search` → PASS (4 tests).

- [ ] **Step 6: Re-export + commit** — add `search_ball` to the `src/lib.rs` collab re-export line.

```bash
cargo test -p bole --lib collab::search
cargo clippy --workspace
git add src/collab/search.rs src/collab/mod.rs src/lib.rs
git commit -m "<bead-id>: pure search_ball — match set + directed reverse-reachability edge ball"
```

---

## Gate 2 (bead: wire + serve) — `CAP_SEARCH`, `Search` verb, relay answers

**Files:**
- Modify: `src/sync/wire.rs` (CAP_SEARCH, contains, Message::Search, round-trip test)
- Modify: `src/sync/collab.rs` (serve advertises CAP_SEARCH; Search arm)
- Test: unit round-trip in `wire.rs`; loopback in `tests/collab_network.rs`

**Interfaces:**
- Consumes: `search_ball` (G1), `collab_adverts`, `build_pack`, `repo.objects.get`, `codec::object_id`/`serialize`, `CapSet`.
- Produces:
  - `pub const CAP_SEARCH: CapSet = CapSet(1 << 0);`
  - `impl CapSet { pub fn contains(self, other: CapSet) -> bool { (self.0 & other.0) == other.0 && other.0 != 0 } }`
  - `Message::Search { term: String, max_hops: u8 }`
  - `serve_collab` advertises `CAP_SEARCH` in relay mode and answers a `Search` with `Pack` + `Done`.

- [ ] **Step 1: Add `CAP_SEARCH` + `contains`** in `src/sync/wire.rs` (near `CapSet::EMPTY`):

```rust
    // <bead-id>
    /// Server-side term search (WS8f-b). A relay advertises this in `Welcome.caps`;
    /// a client requests it in `Hello.caps`.
    pub const CAP_SEARCH: CapSet = CapSet(1 << 0);
    // <bead-id>
    /// True iff `self` contains every bit of the non-empty `other`.
    pub fn contains(self, other: CapSet) -> bool {
        other.0 != 0 && (self.0 & other.0) == other.0
    }
```

- [ ] **Step 2: Add the `Search` variant** to `Message` (in `src/sync/wire.rs`):

```rust
    // <bead-id>
    /// client → relay (after Welcome, in place of HaveWant): server-side term
    /// search bounded by `max_hops`. Answered with a `Pack` of matching profiles
    /// + the directed reverse-reachability edge ball, then `Done`.
    Search { term: String, max_hops: u8 },
```

- [ ] **Step 3: Update the wire round-trip test** (`src/sync/wire.rs` test module) to cover `Search`:

```rust
    // <bead-id>
    let s = Message::Search { term: "pat".into(), max_hops: 4 };
    assert_eq!(decode_message(&encode_message(&s).unwrap()).unwrap(), s);
```

Run `cargo build -p bole 2>&1` and fix any now-non-exhaustive `match` on `Message` the compiler names (add a `Search` arm or a `_` where appropriate — do NOT change `..`-based patterns).

- [ ] **Step 4: Write the failing loopback test — relay answers Search with matches+ball** (`tests/collab_network.rs`, reusing the file's relay/serve helpers)

```rust
// <bead-id>
#[tokio::test]
async fn relay_search_returns_only_matches_and_ball() {
    // Relay corpus: a matching stranger "Pat" reachable via one edge, PLUS a
    // non-matching profile "Zed" and an out-of-ball edge. Serve with relay=true.
    // Client connects, sends Hello{caps: CAP_SEARCH}, expects Welcome.caps to
    // contain CAP_SEARCH, sends Search{"Pat", 4}, decodes the pack.
    // Assert: Pat's profile present; Zed's profile ABSENT; the edge into Pat present;
    // the out-of-ball edge ABSENT. (Use the existing loopback serve helper; drive the
    // client end with raw Message sends mirroring collab_fetch_transient's exchange.)
}
```

> Build the relay repo exactly as the WS8d/e/f-a loopback tests do (pull authors into `remotes/`, or publish under `public/`). Stand the listener up with `serve_collab_tcp_once(&listener, &repo, true, Some(&relay_signer))`. On the client side, connect a `TcpConn`, send `Hello { caps: bole::sync::wire::CAP_SEARCH, intent: Intent::Fetch, client_nonce: None, .. }`, read `Welcome`, assert `welcome_caps.contains(CAP_SEARCH)`, send `Message::Search { term: "Pat".into(), max_hops: 4 }`, read `Pack`, decode via the same path the file's other tests use, then read `Done`. Assert presence/absence by author key.

- [ ] **Step 5: Advertise CAP_SEARCH + add the Search arm in `serve_collab`** (`src/sync/collab.rs`).

Change the Welcome to advertise search in relay mode:

```rust
    // <bead-id>
    let caps = if relay { CAP_SEARCH } else { CapSet::EMPTY };
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps, refs, relay_sig }).await?;
```

Replace the single post-Welcome `HaveWant` match with a branch that also accepts `Search`:

```rust
    // <bead-id>
    match conn.recv().await? {
        Message::HaveWant { want, have } => {
            // Existing whole-aggregate path (unchanged).
            let want: Vec<_> = want.into_iter().filter(|w| authorized.contains(w)).collect();
            let have: HashSet<_> = have.into_iter().collect();
            let missing = negotiate::missing_closure(repo, &want, &have).await?;
            let pack = build_pack(repo, &missing).await?;
            conn.send(&Message::Pack(pack)).await?;
            conn.send(&Message::Done).await?;
        }
        // <bead-id>
        Message::Search { term, max_hops } => {
            // Load the served corpus (exactly what collab_adverts covers: public +,
            // for a relay, all remotes; never scoped/relays), run the pure ball
            // algorithm, and pack the selected objects by their content ids.
            let mut corpus = Vec::new();
            for a in &refs {
                if let Some(Object::Collab(o)) = repo.objects.get(&a.target).await? {
                    corpus.push(o);
                }
            }
            let selected = crate::collab::search_ball(&corpus, &term, max_hops);
            let ids: Vec<_> = selected
                .iter()
                .map(|o| {
                    let bytes = crate::codec::serialize(&Object::Collab(o.clone()))
                        .expect("postcard serialization is infallible for owned data");
                    crate::codec::object_id(&bytes)
                })
                .collect();
            let pack = build_pack(repo, &ids).await?;
            conn.send(&Message::Pack(pack)).await?;
            conn.send(&Message::Done).await?;
        }
        _ => return Err(Error::Storage("collab: expected HaveWant or Search".into())),
    }
    Ok(())
```

> `refs` is the `Vec<RefAdvert>` already computed before Welcome; reuse it so the search corpus is exactly the served corpus. Keep the `authorized` set for the HaveWant path. Ensure `Object` is in scope (`use crate::object::Object;` if not already).

- [ ] **Step 6: Run tests, verify pass** — `cargo test -p bole --test collab_network relay_search` and `cargo test -p bole --lib sync::` → PASS. `cargo clippy --workspace` clean.

- [ ] **Step 7: Commit**

```bash
git add src/sync/wire.rs src/sync/collab.rs
git commit -m "<bead-id>: CAP_SEARCH + Search verb; relay answers with matches + ball pack"
```

---

## Gate 3 (bead: client search + fallback) — `collab_search*`, `query_relay_set`

**Files:**
- Modify: `src/sync/collab.rs` (collab_search, collab_search_authenticated, query_relay_set)
- Modify: `src/lib.rs` (re-exports)
- Test: loopback in `tests/collab_network.rs`

**Interfaces:**
- Consumes: `CAP_SEARCH`/`CapSet::contains` (G2), `verify_relay_challenge`, `verified`, `decode_pack`, `rank_strangers_multi`, `RelayPin`.
- Produces:
  - `pub async fn collab_search(conn: &mut dyn Conn, term: &str, max_hops: u8) -> Result<Vec<CollabObject>>`
  - `pub async fn collab_search_authenticated(conn: &mut dyn Conn, pinned_key: &Key, term: &str, max_hops: u8) -> Result<Vec<CollabObject>>`
  - `query_relay_set` uses `collab_search_authenticated`.

- [ ] **Step 1: Write the failing loopback tests** (`tests/collab_network.rs`)

```rust
// <bead-id>
#[tokio::test]
async fn client_search_computes_trust_path() {
    // A CAP_SEARCH relay caches a chain reaching stranger "Pat". Client runs
    // collab_search("Pat", 4); feed the result + own edges to rank_strangers_multi;
    // assert Pat is found with a Some(trust_path) and correct hops.
}

// <bead-id>
#[tokio::test]
async fn client_search_falls_back_when_cap_absent() {
    // A serve that does NOT advertise CAP_SEARCH (relay=false). collab_search must
    // detect the missing cap, complete the whole-aggregate HaveWant exchange, and
    // return the SAME verified objects as collab_fetch_transient against that serve.
}

// <bead-id>
#[tokio::test]
async fn authenticated_search_rejects_bad_relay_sig() {
    // A CAP_SEARCH relay served with a signer whose key != pinned key. 
    // collab_search_authenticated(conn, pinned_key, "Pat", 4) returns Err (bad sig),
    // before any Search is issued.
}
```

> Reuse the file's loopback helpers. For `client_search_computes_trust_path`, mirror how the WS8f-a `multi_relay_*` tests build `own_edges` and call `rank_strangers_multi(&me, &own_edges, &[(relay_key, objs)], "Pat", 4)`. For the fallback test, stand up `serve_collab_tcp_once(&l, &repo, false, None)` (no CAP_SEARCH) and compare `collab_search` output to `collab_fetch_transient` over an equivalent connection. For the bad-sig test, serve with `Some(&wrong_signer)` and pin `right_signer.public_key()`.

- [ ] **Step 2: Run them, verify they fail** — `cargo test -p bole --test collab_network client_search authenticated_search` → FAIL (fns absent).

- [ ] **Step 3: Implement `collab_search` and `collab_search_authenticated`** (`src/sync/collab.rs`). Factor the shared post-negotiation body into a helper.

```rust
// <bead-id>
/// Shared tail: given the negotiated caps and the advertised refs, either issue a
/// Search (when CAP_SEARCH negotiated) or complete the whole-aggregate HaveWant
/// fallback, then decode + verify the pack fail-closed. Writes nothing.
async fn search_or_fallback(
    conn: &mut dyn Conn,
    negotiated: CapSet,
    refs: &[RefAdvert],
    term: &str,
    max_hops: u8,
) -> Result<Vec<CollabObject>> {
    if negotiated.contains(CAP_SEARCH) {
        conn.send(&Message::Search { term: term.to_string(), max_hops }).await?;
    } else {
        // Fallback: request the whole advertised aggregate (WS8f-a behavior).
        let want: Vec<_> = refs.iter().map(|r| r.target).collect();
        conn.send(&Message::HaveWant { want, have: vec![] }).await?;
    }
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

// <bead-id>
/// Transient server-side search against an unpinned relay. Requests CAP_SEARCH;
/// falls back to the whole-aggregate exchange if the relay does not advertise it.
/// Verifies every object fail-closed. Writes nothing.
pub async fn collab_search(conn: &mut dyn Conn, term: &str, max_hops: u8) -> Result<Vec<CollabObject>> {
    let client_caps = CAP_SEARCH;
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: client_caps,
        intent: Intent::Fetch, client_nonce: None,
    }).await?;
    let (refs, welcome_caps) = match conn.recv().await? {
        Message::Welcome { refs, caps, .. } => (refs, caps),
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let negotiated = client_caps.intersect(welcome_caps);
    search_or_fallback(conn, negotiated, &refs, term, max_hops).await
}

// <bead-id>
/// Authenticated server-side search against a pinned relay: verifies the relay-auth
/// signature (WS8f-a) before any Search, then searches (or falls back). Verifies
/// every object fail-closed. Writes nothing.
pub async fn collab_search_authenticated(
    conn: &mut dyn Conn,
    pinned_key: &Key,
    term: &str,
    max_hops: u8,
) -> Result<Vec<CollabObject>> {
    use rand::RngCore;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let client_caps = CAP_SEARCH;
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: client_caps,
        intent: Intent::Fetch, client_nonce: Some(nonce),
    }).await?;
    let (refs, welcome_caps, relay_sig) = match conn.recv().await? {
        Message::Welcome { refs, caps, relay_sig, .. } => (refs, caps, relay_sig),
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let sig = relay_sig.ok_or_else(|| Error::Storage("relay did not authenticate".into()))?;
    if !crate::collab::verify_relay_challenge(pinned_key, &nonce, &sig) {
        return Err(Error::Storage("relay auth signature invalid".into()));
    }
    let negotiated = client_caps.intersect(welcome_caps);
    search_or_fallback(conn, negotiated, &refs, term, max_hops).await
}
```

> `Intent`, `PROTO_VERSION`, `CapSet`, `RefAdvert`, `decode_pack`, `verified`, `Object` are already imported/used in this file (they appear in `collab_fetch_authenticated`). Reuse them.

- [ ] **Step 4: Switch `query_relay_set` to authenticated search** (`src/sync/collab.rs`): replace the `collab_fetch_authenticated(&mut conn, &pin.key)` call with `collab_search_authenticated(&mut conn, &pin.key, term, max_hops)`. Everything else (skip-and-continue, `rank_strangers_multi`) is unchanged.

- [ ] **Step 5: Run tests, verify pass** — `cargo test -p bole --test collab_network` and `cargo test -p bole --lib sync::` → PASS. `cargo clippy --workspace` clean.

- [ ] **Step 6: Re-export + commit** — add `collab_search`, `collab_search_authenticated` to `src/lib.rs` if the CLI needs them at `bole::` (the ad-hoc CLI path in G4 calls `collab_search`).

```bash
git add src/sync/collab.rs src/lib.rs
git commit -m "<bead-id>: client collab_search(+authenticated) with transparent fallback; query_relay_set uses search"
```

---

## Gate 4 (bead: CLI ad-hoc swap + E2E) — transparent search under `discover relay`

**Files:**
- Modify: `bole-cli/src/commands/discover.rs` (ad-hoc `--endpoint` branch)
- Test: `bole-cli/tests/collab_cli.rs` (E2E)

**Interfaces:**
- Consumes: `bole::collab_search` (G3), the existing `Cmd::Relay` handler.

- [ ] **Step 1: Swap the ad-hoc branch to search** (`bole-cli/src/commands/discover.rs`). In the `Cmd::Relay` handler's `Some(addr)` (ad-hoc `--endpoint`) branch, replace `collab_fetch_transient(&mut conn).await?` + `rank_strangers` with `bole::collab_search(&mut conn, &term, max_hops).await?` fed to the same `rank_strangers`:

```rust
                // <bead-id>
                Some(addr) => {
                    let stream = tokio::net::TcpStream::connect(&addr).await?;
                    let mut conn = TcpConn::new(stream);
                    let corpus = bole::collab_search(&mut conn, &term, max_hops).await?;
                    bole::rank_strangers(&self_key, &own_edges, &corpus, &term, max_hops)
                }
```

> The pinned-set (`None`) branch already routes through `bole::query_relay_set`, which G3 switched to authenticated search — no change needed there. Keep the `collab_fetch_transient` import only if still used elsewhere; otherwise remove it to avoid an unused-import warning.

- [ ] **Step 2: Write the failing E2E** (`bole-cli/tests/collab_cli.rs`), mirroring `cli_discover_relay_trust_path`'s structure:

```rust
// <bead-id>
#[test]
fn cli_discover_relay_search_transparent() {
    // A `node serve --relay` node (advertises CAP_SEARCH) with a discoverable
    // stranger "Pat". Query via `discover relay "Pat" --endpoint <addr> --json`.
    // Assert the same result shape as the whole-aggregate path: reach "stranger",
    // non-null trust_path, correct hops. (Search is transparent — same output.)
    // Use valid-hex seeds and a port distinct from the other tests.
}
```

- [ ] **Step 3: Run it, verify it fails, then passes after Step 1** — `cargo test -p bole-cli --test collab_cli cli_discover_relay_search_transparent`. (If Step 1 is already committed, it should pass; if you wrote the test first, confirm RED before wiring.)

- [ ] **Step 4: Full CLI suite + build + clippy**

```bash
cargo build --workspace
cargo test -p bole-cli
cargo clippy --workspace
```

Expected: all `collab_cli` tests pass (the migrated WS8f-a ones still green — the ad-hoc path now runs search with fallback, transparently), clippy clean.

- [ ] **Step 5: Commit**

```bash
git add bole-cli/src/commands/discover.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: discover relay ad-hoc path uses server-side search (transparent); E2E"
```

---

## Self-Review

**Spec coverage:**
- §2 `CAP_SEARCH` capability negotiation (bit + `intersect`/`contains`) → G2. ✅
- §3 additive `Search { term, max_hops }` verb, answered with `Pack`+`Done` after Welcome → G2. ✅
- §4 relay-side match parity (display_name/bio/dns_aliases/key_hex) + directed reverse-reachability ball + deduped union across matches → G1 (algorithm) + G2 (served-corpus wiring). ✅
- §5 transparent fallback when `CAP_SEARCH` absent (whole-aggregate HaveWant on same connection) → G3. ✅
- §6 `collab_search`/`collab_search_authenticated` + `query_relay_set` integration + no CLI surface change → G3 + G4. ✅
- §7 tests: match-parity incl key-hex-only, reverse-ball completeness/bound, multi-match dedup, directedness (G1); loopback Search returns matches+ball / fallback parity / authenticated bad-sig abort (G2+G3); CLI E2E transparent (G4). ✅ Note: the "tampered object dropped in a Search pack" case is covered by the unchanged `verified()` loop that both G2's serve and G3's client reuse (same drop path proven by the existing `transient_fetch_drops_tampered`); the final review should confirm no separate regression is needed.
- Invariants (relays not authoritative, endpoint read-only, soundness from per-edge verify, relay-auth gates bytes, transient no-mutation, depth-2 untouched, keys raw hex) → Global Constraints + carried per gate. ✅

**Placeholder scan:** Loopback/E2E test bodies (G2 S4, G3 S1, G4 S2) describe assertions in prose plus the exact library calls under test, because the loopback/process harness already exists in those files and must be matched, not reinvented; the concrete assertions and the functions under test are fully specified. All library implementation steps carry complete code.

**Type consistency:** `search_ball(corpus: &[CollabObject], term: &str, max_hops: u8) -> Vec<CollabObject>`; `CAP_SEARCH: CapSet`; `CapSet::contains(self, other) -> bool`; `Message::Search { term: String, max_hops: u8 }`; `collab_search(conn, term: &str, max_hops: u8)`; `collab_search_authenticated(conn, pinned_key: &Key, term: &str, max_hops: u8)`; `search_or_fallback(conn, negotiated: CapSet, refs: &[RefAdvert], term, max_hops)` — consistent across gates and matching the live signatures (`serve_collab`, `Hello`/`Welcome` fields, `intersect`).

**Open verification items for implementers (named in-step):** whether `Object`/`RefAdvert` are already imported in `src/sync/collab.rs` (G2 S5, G3 S3 — they are used by neighboring fns; add `use` only if the compiler complains); the exact loopback/`decode_pack` helper the collab_network tests use (G2 S4); whether `collab_fetch_transient` becomes unused in discover.rs after the swap (G4 S1 — remove import if so).
