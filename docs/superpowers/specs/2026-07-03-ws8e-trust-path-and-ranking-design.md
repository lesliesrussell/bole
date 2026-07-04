# WS8e — Trust-Path-to-Stranger + Trust-Aware Ranking

- **Status:** design (approved 2026-07-03), not yet implemented
- **Depends on:** WS8a substrate (`Profile`, `TrustEdge`, `TrustKind`, `verify_*`, `Namer`); WS8c
  `TrustGraph` (`follow_paths` BFS, `Follow`/`Vouch` edges), `tracked_collab`; WS8d relay +
  `collab_fetch_transient` + `discover relay` CLI.
- **Successor specs (out of scope here):** WS8f relay-trust (persistent trusted-relay set,
  multi-relay query+merge, relay reputation) + server-side search + abuse/moderation. Node liveness
  and DNS alias verification remain free-standing.

WS8e completes the headline. WS8d discovers strangers via a relay but shows them as raw, unvetted
keys. WS8e computes a **verifiable trust-path** connecting the querier to a stranger through the
relay's aggregated `Follow`/`Vouch` graph, and ranks stranger results by that path — turning "here are
some strangers" into "here is a stranger, and here is *why* you might trust them." The trust signal is
cryptographically sound **regardless of relay honesty**: every edge in a path is a self-signed object
the querier verifies fail-closed, so a relay can only withhold or (futilely) inject edges — never
forge a path.

---

## 1. Thesis & invariants

From the WS8d transient relay corpus (Profiles + all aggregated `Follow`/`Vouch` edges, verified), the
querier builds a combined trust graph and computes a bounded path from itself to each matching
stranger, then ranks strangers by that path.

**Hard invariants:**

1. **Sound regardless of relay honesty.** Every edge traversed is a self-signed object the querier
   verified fail-closed. A relay can only *withhold* edges (hide a real connection) or *inject fakes*
   (dropped on verify) — it can never *forge* a trust-path. Soundness comes from per-edge
   verification, not from trusting the relay.
2. **Bounded and shown, never scored.** Paths are capped (default 4 hops) and displayed with node keys
   plus edge kinds (`you --follow--> X --vouch--> Y --follow--> stranger`). There is no opaque trust
   number; the user judges trust from the visible chain.
3. **Relay-trust not required** (deferred to WS8f). WS8e queries whatever relay endpoint the user
   points at, exactly as WS8d; the trust-path is sound either way.
4. **Local neighborhood untouched.** WS8c's depth-2 `discover query` is unchanged. WS8e's deeper
   (≤ `max_hops`) search runs *only* over the relay corpus for stranger-connection — a distinct
   surface. This is where WS8c's "WS8d/e lift the depth-2 bound" is realized, and only here.

---

## 2. Combined trust graph construction

The querier assembles one `TrustGraph` from three **verified** edge sources:

- its own published `Follow`/`Vouch` edges (`Repository::public_edges`),
- its WS8c tracked-cache edges (`Repository::tracked_collab`, already re-verified at index time),
- the relay's transient edges (`collab_fetch_transient`, verified fail-closed on fetch).

`collab_fetch_transient` already returns edges alongside profiles, so WS8e adds no new fetch: it
partitions the corpus into **hits** (Profiles whose fields match the search term, as in WS8d) and
**graph** (all `TrustEdge`s). The querier's own key is the path root. All edges are `TrustEdge`s whose
signatures verified; nothing unverified enters the graph.

---

## 3. Bounded path search

A new method on `TrustGraph`:

```
pub fn trust_path(&self, root: &Key, target: &Key, max_hops: u8) -> Option<Vec<TrustHop>>
```

with `pub struct TrustHop { pub key: Key, pub via: TrustKind }` — `via` is the edge kind that led
*into* `key` from its predecessor on the path. The returned vector is the ordered path from the first
hop after `root` through `target` (root itself is the implicit start).

It generalizes WS8c's `follow_paths` BFS: traverse **both** `Follow` and `Vouch` out-edges (`from_key
→ to_key`), shortest path by construction (first-visit-wins), recording each node's predecessor and
the edge kind traversed. When both a `Follow` and a `Vouch` edge connect the same two nodes, record
`Vouch` (show the stronger link). Returns `None` when `target` is unreachable from `root` within
`max_hops` (default **4**, configurable).

This is a *separate*, deeper search from WS8c's depth-2 neighborhood cap — it lifts the bound only for
relay stranger-connection, not for the local `discover query` index.

---

## 4. Trust-aware ranking

Term-matched stranger hits are ordered:

1. **has-path before no-path** — connected strangers first;
2. among connected, **shorter path first**;
3. tiebreak on equal length: **a path containing ≥ 1 `Vouch` edge before a pure-`Follow` path**;
4. then WS8d's honest tiebreak — `display_name`, then key fingerprint.

Unconnected strangers are still shown, ranked last, with `trust_path: null`. Nothing is hidden; the
ranking never fabricates a signal it doesn't have.

---

## 5. CLI `discover relay` output

Each hit keeps its WS8d fields — `key` (raw 64-hex), `display_name`, `reach: "stranger"` — and gains:

- `trust_path`: an ordered JSON array of `{ "key": <hex>, "via": "follow" | "vouch" }`, or `null` when
  the stranger is unconnected within `max_hops`;
- `hops`: the path length (number of edges), or `null`.

A `--max-hops <n>` flag (default 4) tunes the search depth. Human output prints the chain inline
(e.g. `Pat [stranger, 3 hops]  you -> X -> Y -> Pat`); `--json` carries the structured `trust_path`.
Keys everywhere are raw hex; edge kinds are shown so trust is transparent, never collapsed to a score.
Still no local state mutation (WS8d invariant): `discover relay` remains a transient query.

---

## 6. Testing

**Unit (`TrustGraph::trust_path`)**

- finds a mixed path: root→X (`Follow`), X→Y (`Vouch`), Y→target (`Follow`) yields
  `[{X,follow},{Y,vouch},{target,follow}]`;
- returns `None` when the target is beyond `max_hops`;
- prefers `Vouch` when both a `Follow` and a `Vouch` edge connect the same pair;
- returns the shortest path when two routes of different length exist.

**Unit (ranking)**

- connected before unconnected; shorter before longer; a vouch-containing path before an equal-length
  follow-only path; WS8d `display_name`/fingerprint tiebreak underneath.

**Loopback TCP**

- a relay aggregates a chain (querier follows X, X vouches Y, Y follows the stranger); `discover
  relay`'s transient graph yields the stranger with the correct `trust_path` + `hops`;
- an unconnected stranger returns `trust_path: null`;
- a relay that **withholds** the middle edge yields `null` (completeness degrades, soundness holds);
- a relay that **injects a forged edge** (bad signature) has it dropped — the path is unaffected.

**CLI E2E (real `bole` binary)**

- a real relay with a connecting chain; `discover relay <addr> <term> --json` shows a connected
  stranger with a populated `trust_path` and an unconnected one with `null`.

---

## 7. Scope boundary (→ WS8f / later)

WS8e defines: the combined verified trust graph, bounded combined-edge path search, and trust-aware
ranking with a shown path. Explicitly out:

- **Relay-trust**: persistent trusted-relay set, multi-relay query + merge, relay reputation → WS8f.
- **Server-side search** (scale — the keystone still fetches-then-searches) → WS8f.
- **Abuse/moderation** (relay-side and querier-side filters, denylists, rate limits) → later.
- **Numeric/weighted trust scoring** beyond the shown path + the simple vouch-preference; **unbounded**
  path search; a `Profile` recency timestamp; node liveness; DNS alias verification — all outside WS8e.

WS8e is the "trustworthy stranger" keystone: it shows *why* you might trust a discovered stranger,
soundly and transparently, without first building relay-trust.
