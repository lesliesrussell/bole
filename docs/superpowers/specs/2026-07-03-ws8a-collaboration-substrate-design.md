# WS8a — Collaboration Substrate: Identity, Trust & Discovery

- **Status:** design (approved 2026-07-03), not yet implemented
- **Depends on:** WS1 access-policy core (`bole-fo2`), WS5 distributed sync (transports,
  `apply_push_ops`), `src/acl/authority.rs` (`TrustAnchor` / `TrustStore`, Ed25519),
  the label lattice and `list_refs_filtered` public/bottom short-circuit.
- **Successor specs (out of scope here):** WS8b relay implementation + stranger-reach
  search; WS8c PR / change-proposal + review-thread objects; later message boards,
  landing-page rendering, capability-scoped private discovery.

This is the **foundation slice** of the bole collaboration hub. It builds identity,
trust, and *local* discovery, and it *defines* — without implementing — the relay
interface that later stranger-reach search will plug into. It deliberately does not
build PRs, boards, landing pages, a web UI, or relays.

---

## 1. Thesis & architecture

A bole collaboration network is a set of **sovereign nodes** that author **signed,
content-addressed collaboration objects**, replicate the *public-labeled* subset among
the keys they `Follow`, and resolve identity through a **typed, depth-bounded trust
graph**. There is no global registry and no central index; truth lives in signed
objects under local trust roots.

A **sovereign node** is nothing more than an existing bole repository with a sync
endpoint. A node can represent:

- a single human developer,
- an organization, or
- an agent (or agent-swarm) endpoint.

The architecture must not bias toward human-only nodes: humans, orgs, and agents are
all just keys operating nodes, differing only in their grant and trust sets.

Identity is resolved through the typed trust graph, but **the `TrustStore` / trust
roots remain the ultimate authority** for "whom do I treat as a root of trust." The
trust graph (`Vouch` / `Follow` / `Review` edges) is layered *on top* of the roots; it
never replaces them. A key you refuse to root can never be promoted to authority by any
number of graph edges.

Everything reuses bole's existing spine: BLAKE3 object IDs, the label lattice, Ed25519
(`authority.rs` / `AttestationSigner`), and the WS5 sync transports.

### Hard invariants

These are carried verbatim and enforced by tests:

1. **Keys are identity. Petnames are local. DNS aliases are hints.** No profile, trust
   edge, discovery result, or future feature may treat a petname or DNS alias as
   authoritative; all resolve *to a key*.
2. **Private/scoped by default.** Only objects explicitly carrying the public/bottom
   label are eligible for discovery.
3. **Discovery reads only the public set.** Scoped objects never surface in any
   discovery result, for any querier.
4. **Trust is typed and depth-bounded.** No fuzzy scores, no unbounded crawls.
5. **Trust roots are authoritative; the graph is layered on top.** The graph suggests;
   the `TrustStore` decides who is a root.

---

## 2. Object model

One new `Object` variant — `Object::Collab(CollabObject)` — mirroring how
`Object::Policy` wraps `PolicyObject`. It lives in the same content-addressed store,
under the same lattice, signed with the same Ed25519 machinery. For this slice
`CollabObject` has exactly two variants.

### 2.1 `Profile`

A self-signed, **per-key, monotonic, append-only** description authored by a single key:

- self-asserted display name, bio,
- node / sync endpoints (how to reach this key's node),
- declared DNS-alias claims,
- a monotonic `seq`.

Semantics:

- Only the **highest `seq`** profile for a key is *current*; older profiles remain in
  history but are never consulted for resolution.
- A `Profile` is **metadata only**. It does not override the lattice or any ACL. A
  profile claiming a capability grants nothing; grants come from the access model.
- Canonical identity is the signing key; the profile is merely what that key *says
  about itself*.

### 2.2 `TrustEdge`

`{ from_key, to_key, kind: Vouch | Follow | Review, petname?: String, seq }`, signed by
`from_key`.

- **`Vouch`** — identity trust. "I trust `from_key`'s petname for `to_key` as a credible
  identity suggestion." Carries the optional `petname` the voucher uses for `to_key`;
  this is how naming suggestions propagate.
- **`Follow`** — content/discovery trust. Defines which keys' public objects are pulled
  into this node's discovery neighborhood.
- **`Review`** — **reserved.** Not yet consulted by any subsystem, but it **must still be
  signed, validated, and stored now**, so later PR/review workflows can rely on
  historical trust edges without a migration.

Rules:

- `petname` is **optional** and is only meaningful on `Vouch` edges; `Follow` and
  `Review` edges may omit it. A `petname` on a non-`Vouch` edge is ignored.
- Edges are monotonic per `(from_key, to_key, kind)` via `seq`; the highest `seq` signed
  by `from_key` is current.

### 2.3 Local-only state

*My* petname map — what **this** node calls keys — is **never content-addressed** and
never replicated. It lives in CLI/node-local state (alongside `cli-state.json`). It is
private by construction. Publishing a name is done by authoring a `Vouch` edge, not by
leaking the local map.

### 2.4 Not in this slice

PR / change-proposal, review-thread, message-board, and landing-page objects are **not**
defined here. They are later `CollabObject` variants that ride on this exact machinery.

---

## 3. Node topology & replication

A **node** is a bole repo whose sync server additionally exposes a **`PublicObjectSource`**
interface: "serve my public-labeled collaboration objects" plus "answer a discovery
query."

`PublicObjectSource` **serves only**:

- public-labeled `CollabObject`s (`Profile`, `TrustEdge`), and
- public-labeled *future* `CollabObject` variants (PRs, boards, …) once those specs land,

and **never** any scoped/non-public object of any kind. This reuses the lattice's
public/bottom short-circuit already present in `list_refs_filtered`.

Replication is **pull-based, trust-scoped, periodic, and opportunistic** — explicitly
*not* strongly consistent:

- As a **server**, a node serves its public collab objects to anyone (anonymous public
  read).
- As a **client**, a node periodically pulls the public collab set from the nodes of
  keys it `Follow`s (endpoints resolved from those keys' `Profile`s), out to the bounded
  hop limit (§4), and folds them into its local index.
- An unreachable peer yields only a staler index; it is never a hard error (§6).

**Relays** are defined as *just another `PublicObjectSource`*: a node that aggregates
many publishers and re-serves their public objects. A relay **never becomes
authoritative** over objects — it only caches, aggregates, and re-serves signed objects
that still verify against their authors' keys. v1 implements only the **sovereign-node**
side of the interface; a client pointing at a future relay needs no code change. This is
the "relay interface defined, not implemented" commitment.

---

## 4. Trust graph & naming resolution

`TrustGraph` is built from the `TrustEdge` objects in the local index, indexed by
`(from_key, kind)`. Traversal is **deterministic and explicitly depth-bounded**.

### 4.1 Petname resolution for a key K

Precedence, in order:

```
local petname  >  depth-1 vouch suggestion  >  depth-2 vouch hint  >  fingerprint-only fallback
```

- **Local petname**: what this node calls K (local map). Always wins.
- **Depth-1 vouch**: a `Vouch` edge from a key I directly trust. Strong suggestion.
- **Depth-2 vouch**: friends-of-friends. Weak hint, and **visibly tagged with its trust
  path** ("via X → Y") so the user can see why the name is suggested.
- **No deeper than depth-2** by default.
- **Fallback**: key fingerprint only.

**Collisions are never merged.** Two different keys sharing the same petname from
different vouchers are always disambiguated by key fingerprint. We never collapse two
keys into one conceptual identity.

### 4.2 Discovery neighborhood

Keys reachable via `Follow` edges within the configured hop limit (**default 2**:
your follows plus their follows). Configurable but bounded — never an unbounded crawl.
Changing the hop limit changes only *what is indexed*, never a key's canonical identity.

### 4.3 DNS alias verification

A `Profile` may declare DNS-alias claims (e.g. `alice@bole.dev`). Verification is
NIP-05-style: fetch a `/.well-known/bole-key` endpoint (or a TXT record) on the claimed
domain and confirm it asserts the key's fingerprint.

- Verified → displayed as a **"verified alias"** hint.
- Unverifiable / conflicting → displayed as **"claimed, unverified."**

> **Invariant: a DNS alias is never authoritative and never a resolution key.** Failure
> to verify never prevents use of the underlying key; it only changes how the alias is
> *displayed*. Keys remain canonical regardless of any DNS state.

---

## 5. Discovery / index + query model

The v1 index is **per-node only** — there is no global or relay index, and **all queries
run locally** against:

- public `CollabObject`s held by this node, plus
- public `CollabObject`s pulled from `Follow` neighbors within the hop limit.

The index is keyed by: key fingerprint, petname, DNS alias, profile-metadata terms, and
(future-friendly) repo/domain tags. It may be a simple on-disk/in-memory structure — no
ranking heuristics.

**Ordering rule:**

- primary sort: **trust distance** (direct follow before depth-2),
- secondary sort: **recency** (`seq` / timestamp),
- **no numeric trust scores, no fuzzy ranking.**

**Every result payload includes:**

- the **publishing key**,
- the **object** itself,
- the **trust path** (which node/edges led you to it).

The trust path is what makes discovery "find projects/devs you can *trust*," not merely
"see" — every result is auditable back to a key and an explicit trust reason.

---

## 6. Security invariants & failure handling

### 6.1 Signature- and schema-gated inclusion

An object is indexed only if **all** hold:

- its signature verifies against the author key,
- it carries the public/bottom label,
- it passes **basic schema sanity** (recognized `kind`, monotonic `seq`, well-formed
  fields), and
- its author is within the `Follow` neighborhood.

Fail-closed on trust: anything failing these is dropped, not indexed.

### 6.2 Conflict resolution

Key is canonical; the highest-`seq` signed object **by that key** wins (monotonic). Older
`Profile`/`TrustEdge` versions are ignored for resolution.

### 6.3 Graceful degradation

Unreachable peers/relays **must not** cause sync/session failures. Discovery degrades
gracefully (staler index) and logs diagnostics, but bole's **core repo sync must remain
entirely unaffected** by discovery-layer failures.

### 6.4 Headline safety invariant

> **No scoped object is ever returned by discovery, for any querier.**

Modeled on the existing git-export ACL-filtering test. Mirrored by both an explicit test
case (§7) and an internal audit assertion in the indexing path.

---

## 7. Testing

Built primarily on the in-memory backend (`Repository::memory()`).

**Unit**

- signature verification (accept valid, reject forged/altered),
- label-gating (scoped object never eligible),
- petname precedence (local > depth-1 > depth-2 > fingerprint),
- depth-1 / depth-2 and hop-limit cutoffs,
- DNS-alias verify stub (verified vs claimed),
- highest-`seq`-wins for both `Profile` and `TrustEdge`.

**Invariant / property**

- scoped objects never appear in any discovery result (monotonic-visibility analog),
- petname collisions are never merged.

**Trust-graph transitivity**

- build a small graph mixing `Vouch` and `Follow` edges; assert depth-1 vs depth-2
  behavior matches the spec exactly,
- assert that changing the hop limit changes only what is indexed, never any key's
  canonical identity.

**DNS-alias abuse**

- simulate conflicting DNS claims (two keys claiming the same alias; an alias pointing at
  the wrong key),
- assert keys remain canonical and aliases show only as hints, never as identity.

**Integration (in-memory)**

- 3 `Repository::memory()` nodes + `Follow` edges; publish a public `Profile` on one;
  assert it is discoverable on a follower within depth and **invisible** beyond the hop
  limit; assert a scoped object is never discoverable.

---

## 8. Scope boundary (explicitly out → later beads)

WS8a deliberately does **not** include:

- **No web UI** — all surfaces are CLI/TUI for now.
- **Relay implementation** and stranger-reach search → WS8b.
- **PR / change-proposal + review-thread semantics** → WS8c (the reserved `Review` edge
  is stored now precisely so this needs no migration).
- Message boards, landing-page rendering.
- Capability-scoped private discovery.

Each is a separate spec, to keep this slice from ballooning into "build better GitHub,
completely." This slice gets the substrate correct and locks the invariants; every later
feature sits on top of this identity + trust + discovery foundation without room to
quietly re-centralize on names or fuzzy trust.
