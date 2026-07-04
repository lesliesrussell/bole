# WS8f-a — Trusted Relay Set + Authenticated Multi-Relay Query

- **Status:** design (approved 2026-07-04), not yet implemented
- **Depends on:** WS8a substrate (`Profile`, `TrustEdge`, `verify_profile`/`verify_edge`, `CollabObject`,
  `fingerprint`, `key_hex`); WS8b networked node (`serve_collab`/`collab_adverts`, WS5 `Conn`/`Message`,
  `node serve`/`discover` CLI, `refs/collab/{public,remotes,scoped}/`); WS8d relay + `collab_fetch_transient`
  + `discover relay` CLI; WS8e `TrustGraph::trust_path`, `rank_strangers`/`StrangerHit`.
- **Successor slices (out of scope here):** WS8f-b server-side `Search` verb (query pushdown; avoid whole-
  aggregate download); WS8f-c abuse/moderation (relay-side + querier-side denylists, rate limits, filters);
  WS8f-d relay reputation/weights (using the authenticated relay identity this slice establishes). Node
  liveness and DNS alias verification remain free-standing.

WS8d/e deliver a **single-relay, manually-pointed, transient** stranger search that is cryptographically
sound, bounded, and explainable — but the user types one `host:port` per query and there is no notion of
*which* relays they trust or how to combine several. WS8f-a is the keystone of the "relay quality &
scalability" layer: it persists a **key-pinned set** of relays, proves at connect time that each relay
holds its pinned key (a challenge-response handshake), queries the whole set, and merges the verified
results into one trust-aware ranking — attributing each hit to the relays that served it. Soundness is
unchanged: every object and every trust edge is still verified fail-closed before use. The handshake adds
**accountability** (you know which relay served what, and that it is the relay you pinned), not a new
object-trust root.

---

## 1. Thesis & invariants

A client persists a set of `{relay-key, endpoint}` pins. `discover relay <term>` (no endpoint) iterates
the set: for each relay it runs a possession-proof handshake against the pinned key, and only if that
passes does it fetch the relay's aggregate transiently, verify every object fail-closed, and merge it into
a combined corpus. It builds the WS8e combined trust graph over the union and ranks once, attributing each
stranger to the relays that served it.

**Hard invariants (all carried forward):**

1. **Relays are never authoritative over objects.** Every returned object is self-signed and verified
   against its *embedded* author key. The handshake authenticates the *relay*, never re-attributes or
   blesses the objects it serves. A relay can only *withhold* or *inject* (dropped on verify) — never
   forge.
2. **Endpoint stays read-only.** The relay-auth handshake adds a challenge/response but no write or
   announce path. The client still only reads.
3. **Soundness from per-edge verification, not relay-trust.** Relay authentication gates *whether the
   client bothers to process a relay's bytes*, never whether an object is trusted. A relay that fails
   auth, serves a bad object, or is offline degrades **completeness** (fewer strangers found), never
   **soundness** (nothing false is ever accepted).
4. **Transient query mutates no local state.** `discover relay` persists nothing about results.
   `refs/collab/remotes/` still means "my trust neighborhood"; a stranger enters it only on a deliberate
   `trust follow`. The *relay set* itself (`refs/collab/relays/`) is the one new persisted surface, and it
   is written only by the explicit `relay add`/`relay remove` verbs, never by a query.
5. **Local depth-2 query untouched.** WS8c/WS8a `discover query` and `follow_*` are unchanged; the deeper
   (≤ `max_hops`) search runs only over the merged relay corpus, exactly as in WS8e.
6. **Keys canonical / raw hex.** Relay keys and all output keys are raw 64-hex. Signing seeds (client and
   relay) come from env/file, never argv.

---

## 2. Relay-set storage & CRUD

The trusted-relay set is **local, per-repo, unsigned config** — not a published collaboration object, not
content-addressed identity. It is stored to reuse existing ref/tag machinery without inventing a config-
file surface:

- A library type `RelayPin { key: Key, endpoint: String }` (the relay's pinned Ed25519 public key and its
  transport `host:port`).
- Each pin is serialized via `codec` into an `Object::Blob` and tagged at
  `refs/collab/relays/<relay-fp>`, where `<relay-fp>` is `fingerprint(&key)`. This mirrors how a `Profile`
  is pinned (object + ref) but uses a plain `Blob` — **not** a `CollabObject` — so the "every
  `CollabObject` is self-signed and verifiable" invariant is untouched.
- **One endpoint per key.** The relay *key* is the identity; the endpoint is merely transport. Because the
  ref name is keyed by `fingerprint(&key)`, `relay add <key> <endpoint>` is an **upsert**: adding a key
  that already exists replaces its endpoint (deterministic, no ambiguity, no duplicate entries). A single
  key can never map to two endpoints in the set.

`Repository` gains:

```
pub async fn add_relay(&self, pin: RelayPin) -> Result<()>      // upsert by fingerprint(&pin.key)
pub async fn remove_relay(&self, key: &Key) -> Result<bool>     // true if a pin was removed
pub async fn relays(&self) -> Result<Vec<RelayPin>>             // all pins, deterministic order (by fp)
```

**Never served.** `collab_adverts` is an allowlist — it advertises `public/**` plus (in relay mode)
`remotes/**`, and never anything else. `refs/collab/relays/` is therefore excluded by construction; a unit
test asserts that neither `relay=false` nor `relay=true` adverts ever include a `relays/` ref.

---

## 3. Relay-auth handshake (WS5 wire, additive)

A relay proves possession of its pinned private key before the client processes its bytes. The change to
the WS5 protocol is **additive and optional**, so ordinary `node serve` / `discover pull` flows are
byte-unchanged:

- **Client challenge.** When querying a *pinned* relay, the client generates a fresh **single-use,
  per-connection-attempt** 32-byte nonce (`OsRng`) and includes it in its opening `Hello`
  (`client_nonce: Option<[u8; 32]>`, `None` for all non-relay flows).
- **Relay response.** A node serving with `--relay` **and** a signer returns, in `Welcome`, an Ed25519
  detached signature over a **domain-separated** message — `b"bole-relay-auth-v1" || client_nonce` — not
  the bare nonce (`relay_sig: Option<[u8; 64]>`). The domain separator ensures this signature can never be
  confused with a signature any future feature produces over an arbitrary 32-byte challenge.
- **Client verification.** The client verifies `relay_sig` against the **pinned key** (from the
  `RelayPin`) over the same domain-separated message. On success it proceeds to
  `collab_fetch_transient` + fail-closed object verification. On any failure — no signature, bad
  signature, wrong key — the client **drops that relay** and continues with the rest (§4). The relay never
  sends its key on the wire for trust purposes; the client already holds the pinned key it verifies
  against.
- **Relay identity.** `node serve --relay` now **requires** `--key-env`/`--key-file`: the relay's signer.
  Its public key is the identity operators publish for clients to `relay add`. (A non-relay `node serve`
  needs no key and is unaffected.)

Freshness of the per-attempt nonce gives replay resistance: a recorded `relay_sig` is bound to one nonce
and is useless for any other connection. This is a possession/anti-replay proof, not a bound secure
session — session-transcript binding is deliberately out of scope (YAGNI) and would not change the
accountability property.

---

## 4. Multi-relay query, merge & ranking

`bole discover relay <term>` with **no endpoint** drives the set:

1. **Per relay, independently:** authenticate (§3). If auth fails or the endpoint is unreachable
   (connect error, timeout), **skip and annotate** — never fatal. Else `collab_fetch_transient` and
   verify every object fail-closed (unchanged WS8d behavior).
2. **Merge — union of verified objects across authenticated relays:**
   - **Profiles** deduped by `key`, keeping the highest `seq` (the same rule profile pinning already
     applies), so a stranger seen via several relays appears once at its freshest profile.
   - **Trust edges** deduped by object-id (content address), so identical edges collapse and the combined
     graph is the union of all distinct verified edges.
3. **Rank once over the merged corpus:** build the WS8e combined trust graph from
   `own public_edges + tracked_collab edges + merged relay edges`, then call `rank_strangers` a single
   time over the merged, term-matched profiles. Ranking semantics are exactly WS8e's (has-path > shorter >
   vouch-containing > `display_name` > raw key hex).
4. **Attribution.** Each hit records the set of **relay-keys that served the matching profile**, surfaced
   as `relays: [<key-hex>, …]`. This is the accountability payoff of the handshake: the user sees not just
   the stranger and the trust-path, but *which trusted relays vouched for its presence*.
5. **No local state mutation.** The query reads the relay set and writes nothing; adopting a stranger is
   still an explicit `trust follow`.

**Completeness, not soundness.** A skipped relay (auth-fail, bad object, offline) reduces the strangers or
edges the merge can see — never the correctness of what it returns. Every merged object and every
trust-path edge is verified fail-closed regardless of how many relays participated.

---

## 5. CLI surface

Relay-set management (new `relay` command group):

- `bole relay add <relay-key-hex> <endpoint>` — upsert a pin (§2). `<relay-key-hex>` is raw 64-hex.
- `bole relay remove <relay-key-hex>` — remove a pin; reports whether one existed.
- `bole relay list [--json]` — list pins as `{ key: <hex>, endpoint }`, deterministic order.

Query:

- `bole discover relay <term> [--max-hops N] [--json]` — query the **pinned set** (§4). Each JSON hit keeps
  its WS8d/e fields — `key` (raw hex), `display_name`, `reach: "stranger"`, `trust_path` (array of
  `{ key, via }` or `null`), `hops` (or `null`) — and gains `relays: [<key-hex>, …]` (attribution). Human
  output appends the serving-relay count, e.g. `Pat [stranger, 3 hops, via 2 relays] you -> X -> Y -> Pat`.
- `bole discover relay <term> --endpoint <addr>` — **ad-hoc escape hatch**: query a single unpinned
  endpoint exactly as WS8d did (no handshake against a pin — there is none; still fail-closed object
  verification). Preserves the WS8d one-off workflow. `relays` attribution for such a hit is empty/omitted.

Relay operator:

- `bole node serve --relay --key-env <ENV>` (or `--key-file <path>`) — serve in relay mode with the
  relay's signing identity (now required for `--relay`).

Keys everywhere are raw hex; seeds are read from env/file, never argv.

---

## 6. Testing

**Unit**

- Relay-set CRUD: `add_relay` then `relays()` returns the pin; a second `add_relay` with the same key and a
  new endpoint **upserts** (still one entry, new endpoint); `remove_relay` deletes it and reports
  `true`/`false` correctly.
- `collab_adverts` excludes `refs/collab/relays/` in **both** `relay=false` and `relay=true` modes (and
  still excludes `scoped/`).
- Handshake: a relay signer signs `b"bole-relay-auth-v1" || nonce`; the client verifies against the pinned
  key and **accepts**; verification against a *different* key **rejects**; a signature over the *bare*
  nonce (no domain separator) **rejects**; a replayed signature for a *different* nonce **rejects**.

**Loopback TCP**

- Two relays each hold one slice of a chain (relay A caches `me → X` follow and `X → Y` vouch; relay B
  caches `Y → stranger` follow). A merged `discover relay` authenticates both, unions their edges, and
  yields the stranger with a full trust-path spanning both relays, `relays` listing **both** relay keys.
- One relay returns a **bad handshake signature** → it is dropped; the query still completes using the
  other relay (completeness degraded, soundness intact).
- One relay endpoint is **unreachable** → skipped; the query still completes.
- A relay that serves a **tampered object** → that object is dropped on fail-closed verify (WS8d behavior),
  unaffected by the handshake.
- A stranger served by **both** relays appears **once** (profile dedup by key/highest-seq), attributed to
  both.

**CLI E2E (real `bole` binary)**

- Two `bole node serve --relay --key-env …` nodes with distinct keys, each pulled a different publisher.
  `bole relay add <keyA> <addrA>` and `<keyB> <addrB>`, then `bole discover relay <term> --json` surfaces
  the merged stranger with a populated `trust_path`, `reach: "stranger"`, and `relays` naming the serving
  relay(s). The querier's local `refs/collab/` (beyond the deliberately-added `relays/` pins) is unchanged
  by the query.

---

## 7. Scope boundary (→ WS8f-b / -c / -d)

WS8f-a defines: the persistent key-pinned relay set (local-only, one-endpoint-per-key), the relay-auth
possession handshake (domain-separated nonce), authenticated multi-relay query with union merge + one
trust-aware ranking pass, and per-relay attribution — all with skip-and-continue resilience and unchanged
object-trust semantics. Explicitly out:

- **Server-side `Search` verb** — the client still downloads each relay's whole aggregate to grep locally
  (acceptable now; the merge is over verified objects regardless). Query pushdown → WS8f-b.
- **Abuse/moderation** — relay-side rate limits/denylists and querier-side result filters → WS8f-c.
- **Relay reputation/weights** — ranking or query-ordering by relay quality, using the authenticated relay
  identity this slice establishes → WS8f-d.
- **User-global relay set** (one set across repos, ssh-`known_hosts` style), session-transcript binding for
  the handshake, relay-to-relay gossip, node liveness, DNS alias verification — all remain outside
  WS8f-a.

WS8f-a is the "trusted set, queried and accountable" keystone: it turns "type an endpoint" into "query the
relays I trust, prove they are who I pinned, and merge what they honestly serve" — without adding a single
new object-trust assumption.
