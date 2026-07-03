# WS8c — Cache-and-Forward + Depth-2 Reach

- **Status:** design (approved 2026-07-03), not yet implemented
- **Depends on:** WS8a collaboration substrate (`Profile`, `TrustEdge`, `verify_profile`/`verify_edge`,
  `Index`, `TrustGraph`, `Namer`); WS8b networked node (`serve_collab`/`collab_adverts`/`collab_pull`
  in `src/sync/collab.rs`, `local_discovery_index`/`tracked_collab` in `src/repo/collab.rs`,
  `refs/collab/public/` + `refs/collab/remotes/<fp>/` layout, CLI `node serve`/`discover`).
- **Successor specs (out of scope here):** WS8d relays + stranger-reach search; WS8e node liveness
  (concurrent serve daemon + background poll); WS8f DNS `.well-known`/TXT verification + local
  petname-set command.

WS8c makes the follow-graph traversable over the wire: a node re-serves the *verified* public
objects it pulled from peers it directly follows, so a follower gains its follows' follows
transitively — **depth-2 reach with no address bootstrap** (B forwards C's state; A never dials C).
It also lands the two logged items that only become meaningful once cached state can flow: the
depth-2 `trust_path` intermediary and petname-aware `discover query`.

---

## 1. Thesis & invariants

A node re-serves the verified public objects it pulled from peers it directly follows. Pull B and
you receive B's own state **plus** B's cache of the peers B follows (C) — so C is reachable and
indexable at distance 2 without A ever connecting to C.

**Hard invariants:**

1. **Cached ≠ authored.** Every collaboration object is self-signed by its intrinsic author (the key
   embedded in `Profile.key` / `TrustEdge.from_key`). A receiver verifies against that *embedded*
   key, files the object by its *true author* regardless of which ref namespace it arrived under,
   and fails closed on any signature/authorship mismatch. A server therefore cannot claim authorship
   of another's object — a wrong-key signature simply fails to verify — so lying about namespace is
   harmless.
2. **Serve horizon:** *a node may re-serve verified public state for authors it directly follows, and
   for no others.* Never scoped state; never non-followed cache.
3. **Fail-closed at pull *and* index** (unchanged from WS8b): `collab_pull` verifies each object
   before creating a tracking ref; `tracked_collab` re-verifies before an object enters the index.
4. **Depth-2 is bounded in BOTH layers.** It is a *transport* rule (the serve horizon, §2) *and* a
   *discovery/index* rule (`follow_neighborhood` capped at hops = 2, §4). WS8d's job is explicitly to
   **lift** this bound via relays — not to secretly relax WS8c. Any request for reach or paths deeper
   than 2 is out of scope until WS8d.

---

## 2. Serve semantics (`collab_adverts` / `serve_collab`)

`collab_adverts` today advertises only `refs/collab/public/**`. WS8c extends it to advertise:

- `refs/collab/public/**` — all of the node's own authored public objects; plus
- `refs/collab/remotes/<fp>/**` — but **only** for each `<fp>` whose author key is in the node's
  **direct-follow set**, computed at advertise time from the node's own `Follow` edges
  (`public_edges()` filtered to `TrustKind::Follow`, collecting `to_key`s).

Clarifications:

- The filter is on **author fingerprints**, not namespaces alone: serve a cached object iff its
  intrinsic author key is in the direct-follow set. (In WS8c the `remotes/<fp>/` ref segment already
  equals the author's fingerprint, so the filter is "is `<fp>` a followed author's fingerprint,"
  which is equivalent.)
- **Never scoped.** `refs/collab/scoped/` remains strictly local and is never advertised — even if it
  belongs to a followed author. This continues WS8b's M2 rule verbatim.
- The existing `want`-constraint-before-`missing_closure` (the WS8b/`bole-yl2` protection) still gates
  every transfer, so a client cannot name an unadvertised object id.

This is serve-side *hygiene* (amplification control), not the puller's security boundary — the puller
is protected by signature verification regardless of what a server chooses to advertise.

---

## 3. Pull semantics (`collab_pull`)

WS8b's single-author assumption ("serve-own-only ⇒ exactly one peer key") is **dropped**. The server
now advertises two namespaces — `public/` (its own) and `remotes/` (its followed-cache) — spanning
multiple authors. `collab_pull`:

1. verifies **every** advertised object against its embedded author key (fail-closed; a failure drops
   that object, no ref created);
2. files each survivor under the puller's `refs/collab/remotes/<intrinsic-author-fp>/…` keyed by
   *true author* — the server's own objects land under `remotes/<serverfp>/`, the server's cached C
   objects land under `remotes/<Cfp>/`;
3. still identifies and returns the **server's own key** — the author of the object advertised under
   `public/profile` — so `discover pull` can report it and `trust follow` it. If the server advertises
   no valid `public/` profile, pull errors (unchanged from WS8b's `pull_errors_with_no_valid_profile`).

**Storage story (single, unambiguous):**

- `refs/collab/public/**` = objects **this node authored** (its published state). A node's own objects
  are never written to its own `remotes/`.
- `refs/collab/remotes/<fp>/**` = this node's **by-author cache of pulled state** (provenance-free:
  filed by intrinsic author, not by who served it).

Indexing (§4) reads own `public/` as distance-0 and tracked `remotes/` as distance ≥ 1 — the WS8b
model, unchanged. Because A re-serves only authors *A* follows (§2), and A follows B but not C, A
stores C for its own discovery yet never rebroadcasts C: propagation is a bounded 2 hops (an object is
re-served only by nodes that directly follow its author).

---

## 4. Index & depth-2 `trust_path`

`TrustGraph::follow_neighborhood` returns only `BTreeMap<Key, u8>` (distances), which is why WS8b's
`trust_path` was stubbed `[self, author]`. WS8c adds a **predecessor-tracking traversal** to
`TrustGraph` (BFS recording each node's predecessor) and reconstructs the real path, so a depth-2
author C reached via B yields `trust_path = [A, B, C]`.

With cache-and-forward, C's profile and C's `Follow` edges are now in A's tracked set (A pulled them
via B), so the combined graph (A's own edges + tracked edges) contains A→B and B→C, and
`follow_neighborhood(A, 2)` genuinely reaches C at distance 2 via B.

Design choices to document:

- `trust_path` is **minimal-hop by construction** — BFS yields shortest paths. WS8c deliberately
  ignores multi-path aggregation and weighted trust; it chooses first/shortest path for
  explainability.
- The hop limit for both neighborhood and path derivation is **exactly 2** in WS8c. Deeper paths are
  out of scope until WS8d.
- Indexing stays **fail-closed** (`tracked_collab` re-verifies every object's signature before it
  enters the index) and **depth-bounded** (hops = 2).

---

## 5. Petname-aware `discover query`

`discover query` results today emit `key` (raw hex) + `name` (self-asserted `display_name`) +
`distance`. WS8c runs the WS8a `Namer` over the trust graph to resolve a **trust-scoped petname** per
result and emits, per hit:

- `key` — raw 64-hex (round-trips into `trust follow`);
- `display_name` — the author's self-asserted name, clearly a **hint**;
- `petname` + its `depth` — the name the *trust graph* gives this key via `Vouch` edges (fingerprint
  fallback when none);
- `reach` — `self` / `direct` / `transitive`, **defined by graph distance** (0/1/2), not by transport
  provenance (so it stays consistent when WS8d introduces new intermediate hops);
- `trust_path` — the real `[A, …]` path from §4.

Constraints:

- Petnames in WS8c are **derived from the graph** (`Vouch` edges); WS8c introduces **no new storage
  format and no new commands**. "How do I manually nickname someone?" (a local petname-set command) is
  explicitly deferred (WS8f / later).
- Self-asserted `display_name` is never treated as authoritative identity — it is surfaced only as a
  hint alongside the trust-scoped petname.

---

## 6. Testing

**Unit**

- `collab_adverts` includes a followed author's `remotes/` objects and **excludes** a non-followed
  author's; always excludes `scoped/`.
- `collab_pull` files a multi-author set by intrinsic author (server-own → `remotes/<serverfp>/`,
  cached C → `remotes/<Cfp>/`) and drops a tampered cached object (no ref).
- `TrustGraph` predecessor traversal yields `[A, B, C]` for a depth-2 target.
- `Namer` surfaces a `Vouch` petname (and fingerprint fallback) in a resolved query result.

**Loopback-TCP integration**

- A follows B; B follows and has pulled C; A pulls B and receives B-own **plus** B-cached-C; A's index
  lists C at distance 2 with `trust_path = [A, B, C]`.
- A scoped object on B is never forwarded to A.
- A non-followed author's cache on B is never advertised or served to A.
- **Negative (tamper):** a deliberately tampered cached C object on B (bad signature / wrong author
  key) is dropped at pull *and* would be dropped at index — C never surfaces at distance 2.
- **Negative (over-depth):** wire A→B→C→D; assert A's index never surfaces D, and B never advertises
  its cached D to A (D's author is outside B's direct-follow set), even though B has D cached from C.

**CLI E2E (real `bole` binary)**

- Three `bole node serve` nodes wired A→B→C; `discover query` from A surfaces C as `transitive` via B
  with a resolved petname and `trust_path = [A, B, C]`.

---

## 7. Scope boundary (→ WS8d / WS8e / WS8f)

WS8c stops at: cache-and-forward, depth-2 reach, minimal-hop trust paths, and petname-aware query.
Explicitly out:

- **Dedicated relays / aggregation, stranger-reach search (anyone beyond depth-2), relay-trust /
  abuse / ranking** → WS8d.
- **Concurrent spawned-serve daemon + background/periodic poll** → WS8e.
- **Real DNS `.well-known`/TXT alias verification + a local petname-set command** → WS8f / later.

Depth stays hard-capped at 2 in both transport and index.

**Product alignment (Grove):** WS8c defines the maximum reach Grove can safely assume for depth-2
discovery and funding surfaces *without relying on relays*; WS8d will be the first time Grove's UX is
allowed to talk about strangers beyond friends-of-friends.
