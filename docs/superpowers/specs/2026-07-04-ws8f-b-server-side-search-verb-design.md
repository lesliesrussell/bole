# WS8f-b — Server-Side `Search` Verb

- **Status:** design (approved 2026-07-04), not yet implemented
- **Depends on:** WS8a substrate (`Profile`, `TrustEdge`, `verify_profile`/`verify_edge`, `fingerprint`,
  `key_hex`); WS8b networked node (WS5 `Conn`/`Message`, `CapSet`, `Pack`/`Done`, `collab_adverts`,
  `serve_collab`); WS8d transient fetch (`collab_fetch_transient`, `discover relay`); WS8e
  `TrustGraph`/`trust_path`, `rank_strangers`; WS8f-a authenticated relay set (relay-auth handshake,
  `collab_fetch_authenticated`, `query_relay_set`, `rank_strangers_multi`, `StrangerHit.relays`).
- **Successor slices (out of scope here):** WS8f-c abuse/moderation (relay-side rate limits/denylists,
  querier-side filters); WS8f-d relay reputation/weights. Node liveness and DNS alias verification remain
  free-standing.

Today every `discover relay` — ad-hoc (`collab_fetch_transient`) or over the authenticated pinned set
(`collab_fetch_authenticated` in `query_relay_set`) — downloads a relay's **whole** aggregate and greps
profiles locally. That was an explicit scaling compromise flagged since WS8d. WS8f-b adds a
capability-negotiated **`Search`** request: the client sends the term (and its hop bound) and the relay
returns only **matching profiles + the bounded edge-set the client needs to compute trust-paths** —
instead of the whole corpus. Trust stays entirely client-side: every returned object is verified
fail-closed, so a relay can only *withhold* or *inject* (dropped on verify) — never *forge*. Filtering is
a **completeness/efficiency** concern, never a soundness one.

---

## 1. Thesis & invariants

A relay that advertises `CAP_SEARCH` answers a `Search { term, max_hops }` request with a pack of the
profiles matching `term` plus the **directed reverse-reachability ball** of edges around those matches
(the exact edge set a forward ≤`max_hops` trust-path into a match can traverse). The client verifies every
object fail-closed and feeds the same `rank_strangers_multi` merge/ranking path — so results are identical
to the whole-aggregate flow, only cheaper.

**Hard invariants (all carried forward):**

1. **Relays never authoritative over objects.** Every returned profile and edge is self-signed and
   verified against its embedded author key. Server-side *filtering* selects which signed objects to send;
   it never re-attributes, mutates, or blesses them. A relay can only withhold or inject (dropped) — never
   forge.
2. **Soundness from per-edge verification, not relay honesty.** A lying relay's filter (wrong matches,
   withheld edges, injected fakes) degrades **completeness** — never **soundness**. Injected objects fail
   `verify_*` and are dropped; withheld objects only reduce what is found.
3. **Endpoint read-only.** `Search` is a read request; it adds no write/announce path.
4. **Relay-auth gates bytes, not trust.** WS8f-a's handshake authenticates *which relay* you are talking
   to and whether to process its bytes. It is orthogonal to and composes with `Search`; object trust still
   comes only from per-object verification.
5. **Transient query, no local mutation.** A `Search` query persists nothing (same as WS8f-a). The relay
   set (`refs/collab/relays/`) is unchanged and written only by `relay add`/`remove`.
6. **Local depth-2 query untouched.** WS8c/WS8a `discover query` and `follow_*` are unchanged; `Search`
   affects only the relay path.
7. **Keys canonical / raw hex.** All keys on the wire and in output are raw bytes / 64-hex.

---

## 2. Capability negotiation

Add one bit to the existing `CapSet(u32)`:

```
pub const CAP_SEARCH: CapSet = CapSet(1 << 0);
```

- The client sets `CAP_SEARCH` in `Hello.caps` on a relay query.
- A relay that supports server-side search sets `CAP_SEARCH` in `Welcome.caps`.
- The negotiated capability is `Hello.caps.intersect(Welcome.caps)` (the existing method). If it contains
  `CAP_SEARCH`, the client issues a `Search`; otherwise it falls back (§5).

No new negotiation machinery — `CapSet`/`intersect` already exist and are currently only ever `EMPTY`.
Non-relay flows continue to send `CapSet::EMPTY` and are unaffected.

---

## 3. Wire verb (additive)

A new message variant, sent by the client **after** `Welcome`, in place of `HaveWant`:

```
Message::Search { term: String, max_hops: u8 }
```

The relay answers with the existing `Pack(Vec<u8>)` carrying the selected objects, then `Done` — reusing
the WS4 pack build/decode path and the client's existing decode-and-verify loop unchanged. Adding one
variant is additive; all existing exchanges (fetch, push, pull) are byte-unchanged.

**Exchange (authenticated relay):**

```
client → Hello { caps: CAP_SEARCH, intent: Fetch, client_nonce: Some(nonce), .. }
relay  → Welcome { caps: CAP_SEARCH, relay_sig: Some(sig), .. }
client   (verify sig vs pinned key — WS8f-a; abort this relay on failure)
client → Search { term, max_hops }
relay  → Pack( matching profiles + reverse-ball edges )
relay  → Done
```

Ad-hoc (`--endpoint`, unpinned) is identical without `client_nonce`/`relay_sig`.

---

## 4. Relay-side filter + directed reverse-reachability ball

On receiving `Search { term, max_hops }`, the relay computes from its **served corpus** (exactly the
objects `collab_adverts(repo, relay=true)` would serve — `public/**` + all `remotes/**`, never `scoped/`
or `relays/`):

1. **Match set** — profiles whose `display_name`, `bio`, any `dns_aliases` entry, or **raw key hex**
   (`key_hex`) contains `term`. This is the **exact same field set and match rule as the client's
   `rank_strangers`**, so server-side Search and client-side fallback are semantically identical. (Keeping
   this parity is a correctness requirement, not an optimization.)
2. **Directed reverse-reachability ball** — for each matched stranger, BFS the relay's aggregate trust
   graph **backward** (traverse edges from `to_key` to `from_key`) up to `max_hops` levels, collecting
   every edge traversed. When multiple matches are found, the returned edge set is the **union of their
   reverse balls, deduped by object-id**.
3. **Pack** — the matching profiles + that deduped edge set. Intermediate nodes on paths need **no
   profile** (a trust-path is a chain of keys + edge-kinds, `TrustHop { key, via }`), so only matched
   strangers' profiles are sent.

**Completeness proof obligation (state in tests):** every forward directed path of length ≤ `max_hops`
from any root into a matched stranger consists solely of edges in that stranger's reverse ball. (An edge
`a → b` lies on such a path iff `b` reaches the stranger within `max_hops − 1` forward hops, i.e. iff `a`
is reached by the reverse BFS within `max_hops` — so the ball contains exactly the edges a client's
forward BFS could traverse.) Completeness is **relative to the relay's aggregate**, the same bound
WS8f-a's whole-aggregate fetch carries; the client's own root→frontier edges are client-side and never
needed from the relay.

---

## 5. Fallback — never skip

If the negotiated caps lack `CAP_SEARCH` (an older or minimal relay), the client falls back **for that
relay** to the WS8f-a whole-aggregate fetch (`collab_fetch_authenticated` / `collab_fetch_transient`) plus
the existing local filter. In a mixed pinned set, Search-capable relays use the efficient path and others
fall back; the corpora merge identically in `rank_strangers_multi`. A relay is **never silently skipped**
for lacking Search — that would sacrifice completeness. Lack of Search support affects efficiency only,
never correctness or command semantics.

---

## 6. Client integration (library) + CLI (unchanged)

Two new library functions mirror the WS8f-a fetch pair, in `src/sync/collab.rs`:

```
pub async fn collab_search(conn, term: &str, max_hops: u8) -> Result<Vec<CollabObject>>
pub async fn collab_search_authenticated(conn, pinned_key: &Key, term: &str, max_hops: u8) -> Result<Vec<CollabObject>>
```

Each runs the §3 exchange (the `_authenticated` variant performs the WS8f-a nonce/sig verification first),
sends `Hello` with `CAP_SEARCH`, and — **only if `Welcome.caps` contains `CAP_SEARCH`** — sends `Search`
and decodes+verifies the pack fail-closed (identical `verified()` drop as the fetch path). If the relay
did not advertise `CAP_SEARCH`, the function transparently completes the WS8f-a whole-aggregate exchange
instead (fallback), returning the same verified `Vec<CollabObject>`.

`query_relay_set` (WS8f-a) is updated to call the search variant per relay, passing the query's `term` and
`max_hops`. It still feeds `rank_strangers_multi`, so trust-path, ranking, and per-relay attribution are
**byte-identical** to WS8f-a — Search only changes *how much* each relay ships.

**No CLI change.** `discover relay <term> [--max-hops N] [--endpoint <addr>] [--json]` already carries the
term and hop bound; WS8f-b is a transparent, capability-negotiated upgrade beneath it. Users keep one
mental model; a relay lacking Search is slower, never invisible or different.

---

## 7. Testing

**Unit (relay filter + ball)**

- The relay match set uses the same fields as `rank_strangers` (`display_name`/`bio`/`dns_aliases`/raw key
  hex) — a profile matching only on key hex is selected; a non-matching profile is excluded.
- Reverse-ball completeness: a chain `a → b → c → stranger` in the relay corpus, `Search` for the stranger
  at `max_hops = 3` returns all three edges; at `max_hops = 2` the far edge `a → b` is dropped (and a
  4-hop path would be incomplete — as intended by the bound).
- Multi-match union: two matched strangers with overlapping reverse balls return the deduped union (a
  shared edge appears once).
- Directedness: an edge pointing *away* from a match (not on any forward path into it) is **not** returned.

**Loopback TCP**

- A `CAP_SEARCH` relay: `collab_search` returns only matching profiles + ball edges — assert a
  non-matching profile and an out-of-ball edge are **absent** from the pack, yet the client still computes
  the correct `trust_path` + `hops` for a connected stranger.
- Fallback: a relay that does **not** advertise `CAP_SEARCH` causes `collab_search` to complete the
  whole-aggregate exchange; assert the resulting hits are **identical** to `collab_fetch_transient` + local
  filter over the same corpus.
- Authenticated Search: over a pinned relay, `collab_search_authenticated` verifies the relay-auth
  signature first, then Searches; a bad relay-auth signature aborts that relay (WS8f-a behavior), and an
  injected unsigned/tampered object in a Search pack is dropped on verify.
- Multi-relay parity: `query_relay_set` against one Search-capable and one fallback relay yields the same
  merged, attributed hits as if both used the whole-aggregate path.

**CLI E2E (real `bole` binary)**

- `bole discover relay <term> --json` against a `node serve --relay` that supports Search returns the same
  stranger, `reach: "stranger"`, `trust_path`, `hops`, and `relays` attribution as the WS8f-a
  whole-aggregate path — proving the optimization is transparent end-to-end.

---

## 8. Scope boundary (→ WS8f-c / -d)

WS8f-b defines: the `CAP_SEARCH` capability bit, the additive `Search { term, max_hops }` verb, the
relay-side match-parity filter + directed reverse-reachability ball (deduped union across matches), the
`collab_search`/`collab_search_authenticated` client functions with transparent fallback, and the
`query_relay_set` integration — all with unchanged object-trust semantics and no CLI change. Explicitly
out:

- **Abuse/moderation** — relay-side rate limits, denylists, query-cost caps; querier-side result filters →
  WS8f-c.
- **Relay reputation/weights** → WS8f-d.
- **Ranked/paginated server-side results, relevance scoring, fuzzy/tokenized matching** beyond the current
  substring rule; a server-side result cap (top-N) — a later refinement once abuse controls exist.
- **Index structures** (the relay may scan its corpus linearly in this slice; a persistent search index is
  a future optimization).
- User-global relay set, node liveness, DNS alias verification — all remain outside WS8f-b.

WS8f-b is the scale keystone: it moves the filter to the relay while keeping every trust property on the
client under fail-closed verification — a slower relay (no Search) stays correct, and a hostile relay
(bad filter) stays sound.
