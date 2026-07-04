# WS8d — Relay Role + Relay-Query (Stranger Search Keystone)

- **Status:** design (approved 2026-07-03), not yet implemented
- **Depends on:** WS8a substrate (`Profile`, `TrustEdge`, `verify_profile`/`verify_edge`, `CollabObject`);
  WS8b networked node (`serve_collab`/`collab_adverts`/`collab_pull`, `node serve`/`discover` CLI,
  `refs/collab/{public,remotes,scoped}/`); WS8c cache-and-forward (serve horizon, multi-author pull).
- **Successor specs (out of scope here):** WS8e relay-trust + trust-aware stranger ranking +
  trust-path-to-stranger + server-side search verb; WS8f abuse/moderation. Node liveness and DNS
  alias verification remain free-standing.

WS8d delivers the headline — **cold-discover a trustworthy stranger** — in its minimal, honest form.
A **relay** is a node that pulls a set of publishers (reusing `collab_pull`) and serves its *whole*
aggregate with the WS8c follow-horizon turned off. A client runs `discover relay <endpoint> <term>`
to fetch that aggregate **transiently**, verify every object fail-closed, filter for matches, and show
verified *strangers* — mutating no local state. Trust annotations, trust-aware ranking, and moderation
are deliberately deferred; WS8d proves the surface.

---

## 1. Thesis & invariants

A relay pulls publishers (existing `collab_pull`) and serves its whole aggregate horizon-off. A client
queries a relay transiently, verifies fail-closed, filters, and shows verified strangers without
touching local state.

**Hard invariants (all carried forward):**

1. **Relays are never authoritative.** Every returned object is self-signed; the client verifies
   against the *embedded* author key and displays the publisher key. A relay can only *withhold or
   include* signed objects — never forge, re-attribute, or mutate them.
2. **Endpoint stays read-only.** Relays add no write/announce path. Aggregation is the relay *pulling*
   publishers, never publishers pushing.
3. **Strangers are transient.** A relay query persists nothing. `refs/collab/remotes/` keeps meaning
   "my trust neighborhood"; a stranger enters it only when the user deliberately `trust follow`s them
   (then WS8c takes over via the stranger's `Profile.endpoints`).
4. **Depth-2 neighborhood untouched.** `discover query` remains confined to the ≤2-hop follow-graph;
   `discover relay` is a distinct surface with a distinct result shape.

---

## 2. Relay role (server)

Relay mode is a serve-time toggle: `bole node serve --relay`. It threads a `relay: bool` into
`collab_adverts(repo, relay)`:

- `relay = false` (default, WS8c behavior): advertise `public/**` + the `remotes/` of authors the node
  directly follows; never `scoped/`.
- `relay = true`: advertise `public/**` + **all** of `remotes/**` (follow-horizon filter off); still
  never `scoped/`.

Same adverts/pack flow, no new wire verb — a larger advert set is the only difference. `serve_collab`
and `serve_collab_tcp_once` gain the `relay` parameter; `node serve --relay` passes it through.

**Aggregation reuses existing machinery.** A relay's "publisher set" in WS8d is simply *whatever
authors it has pulled via `discover pull` so far*. The operator populates the aggregate by running the
existing `bole discover pull <addr>` against each publisher (each object lands under
`remotes/<intrinsic-author-fp>/`, verified). There is **no new aggregation command and no automation**
in the keystone — a relay is "a node that has pulled some publishers and serves with `--relay`."
Configured publisher-sets and auto-refresh are later niceties.

---

## 3. Transient client fetch (library)

A new `collab_fetch_transient(conn: &mut dyn Conn) -> Result<Vec<CollabObject>>` in `src/sync/collab.rs`:

- runs the same Hello → Welcome → HaveWant → Pack exchange a pull uses (want = advertised targets,
  have = empty);
- decodes the pack and, for each object, **verifies its signature fail-closed** (`verify_profile` /
  `verify_edge`), dropping any that fail;
- returns the surviving verified `CollabObject`s.

It needs **no `Repository`** — decode + verify is pure — and **writes nothing** to any store. This is
the transient corpus the CLI searches; tampered/unsigned objects never surface.

---

## 4. CLI `discover relay <endpoint> <term>`

`bole discover relay <relay-endpoint> <term> [--json]`:

1. `TcpConn::connect(endpoint)` → `collab_fetch_transient`.
2. Filter to `Profile`s whose `display_name`, `bio`, `dns_aliases`, or key-fingerprint match `<term>`
   (the same profile fields the local discovery index matches).
3. Rank (§5) and display each hit: `key` (raw 64-hex via `key::hex32`), `display_name` (self-asserted,
   a hint), and `reach: "stranger"` (relay-sourced; **no trust annotation** — that is WS8e).
4. **No state change.** To adopt a stranger, the user runs `trust follow <key>` (WS8c then pulls them
   via their `Profile.endpoints` into the neighborhood).

`discover relay` is clearly distinct from `discover query`: different command, different result shape,
honest "these are unvetted strangers." `--json` is the stable contract; keys shown as raw hex, never
fingerprint.

---

## 5. Ranking (minimal, honest)

WS8a `Profile` carries no timestamp — only a per-key monotonic `seq`, which is not comparable *across*
authors. True cross-stranger "recency" is therefore unavailable, and faking it would be worse than
nothing. The keystone ranks by **match, then a deterministic tiebreak** (`display_name`, then key
fingerprint) — stable and reproducible.

Trust-aware ranking ("trustworthy stranger" via the relay's aggregated vouch graph and
trust-path-to-stranger) is WS8e's job. A genuine cross-author recency signal would require a new
`Profile` timestamp field (future). WS8d is explicit that its ranking is minimal and carries no trust
meaning.

---

## 6. Testing

**Unit**

- `collab_adverts(repo, relay = true)` advertises a **non-followed** author's `remotes/` objects, while
  `relay = false` still excludes them (WS8c horizon intact); both modes still exclude `scoped/`.
- `collab_fetch_transient` returns verified objects, drops a tampered one, and writes nothing to any
  repo (assert an accompanying store is untouched / the function takes no repo).

**Loopback TCP**

- A relay node has pulled B and C (neither followed by the querier). The querier runs the transient
  fetch and finds a stranger by `display_name`.
- Assert the stranger is **not** written to the querier's `refs/collab/remotes/` and **not** returned
  by the querier's `discover query` (neighborhood).
- Assert `discover relay` causes **no change to the querier's on-disk `refs/collab/` layout at all**,
  and that a **second** `discover relay` behaves identically (no hidden cache).
- Then `trust follow` the stranger and confirm they now appear in `discover query` (WS8c takes over).

**CLI E2E (real `bole` binary)**

- A `bole node serve --relay` node with pulled strangers; `bole discover relay <addr> <term> --json`
  surfaces a stranger marked `"stranger"`; the querier's local `refs/collab/` state is unchanged until
  an explicit `trust follow`.

---

## 7. Scope boundary (→ WS8e / WS8f)

WS8d defines: the relay role, horizon-off serving, and transient relay-query with fail-closed
verification and an honest minimal ranking. Explicitly out:

- **Relay-trust** (which relays you trust), **trust-aware stranger ranking**, **trust-path-to-stranger**,
  and a **server-side `Search` verb** for scale (the keystone downloads the whole aggregate to grep it
  — acceptable now) → WS8e.
- **Abuse/moderation**: relay-side and querier-side filters, denylists, rate limits → WS8f.
- Configured publisher-sets + auto-refresh; a `Profile` recency timestamp; relay-to-relay gossip; node
  liveness; DNS alias verification — all remain outside WS8d.

WS8d is the minimal, honest, working "cold-discover a stranger" keystone; WS8e makes the stranger
*trustworthy*, and WS8f keeps the relay clean.
