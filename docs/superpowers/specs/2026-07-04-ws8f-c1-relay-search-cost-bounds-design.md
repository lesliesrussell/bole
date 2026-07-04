# WS8f-c1 — Relay-Side Search Query-Cost Bounds

- **Status:** design (approved 2026-07-04), not yet implemented
- **Depends on:** WS8f-b server-side Search (`Message::Search { term, max_hops }`, `serve_collab` Search arm,
  `search_ball`, `collab_search`/`collab_search_authenticated`, `query_relay_set`); WS8f-a relay-auth +
  `discover relay` CLI; WS8e `rank_strangers`.
- **Successor slices (out of scope here):** WS8f-c2 rate limiting / request-frequency / connection budgets
  (needs the deferred concurrent-serve story); WS8f-c3 relay-side author denylists; WS8f-c4 querier-side
  result filters; operator-configurable bound values. WS8f-d relay reputation/weights. Node liveness and
  DNS alias verification remain free-standing.

WS8f-b moved the discovery filter server-side: a relay answers `Search { term, max_hops }` by scanning its
aggregate and computing a directed reverse-reachability edge ball per matching profile. That surface has
two attacker-controlled amplifiers. `term` is matched with `.contains`, so an empty or 1-character term
matches **every** profile — firing a reverse-BFS ball from every node. `max_hops` is a `u8` (≤255), turning
each ball from O(local) into O(whole reachable graph). Together — `term=""`, `max_hops=255` — a single
request costs **O(corpus²)**. WS8f-c1 caps both amplifiers so relay work stays ~O(corpus): the relay
**clamps** `max_hops` to a ceiling and **rejects** a too-short term before doing any corpus work. A clamped
or rejected request degrades **availability/completeness**, never soundness — the same framing that runs
through the whole relay track.

---

## 1. Thesis & invariants

The relay bounds the two amplifiers the WS8f-b Search verb exposed, at the serve edge, as **relay-local
policy**. The pure `search_ball` mechanism and the wire `Message::Search` are untouched. Soundness stays
entirely client-side (per-edge fail-closed verification); these bounds only limit *how much work a relay
will do*, never what an object means.

**Hard invariants (all carried forward):**

1. **Relays never authoritative over objects.** A bound only *withholds or limits work* (reject a request,
   clamp a depth). It never forges, mutates, re-attributes, or filters *which signed objects are trusted* —
   the client still verifies every returned object fail-closed.
2. **Bounds degrade availability, never soundness.** A rejected Search (too-short term) or a clamped
   `max_hops` reduces what a relay returns; it never causes anything false to be accepted. Completeness may
   drop; correctness cannot.
3. **Endpoint read-only; no new verbs.** No wire message is added or changed. `Search` keeps its shape.
4. **Local depth-2 query and the whole-aggregate path untouched.** `discover query`/`follow_*` are
   unchanged. The `HaveWant` whole-aggregate exchange carries no `term`/`max_hops`; its cost is already
   bounded by the advertised set, so it is unaffected.
5. **Keys canonical / raw hex.** No key handling changes.
6. **Transient query, no local mutation.** Search stays transient; nothing persists.

---

## 2. The two bounds (fixed constants)

Two public constants, sane fixed defaults (operator-configurable values are a deferred WS8f-c follow-on —
they would need a config surface):

```
/// Six hops is the maximum we consider meaningful for a trust path; deeper
/// searches are clamped to this. The client default (4) sits below it, so
/// honest queries never clamp.
pub const MAX_SEARCH_HOPS: u8 = 6;

/// Three characters is the minimum we consider meaningful for a search term.
/// Shorter terms match (nearly) every profile and are rejected before any work.
pub const MIN_SEARCH_TERM_LEN: usize = 3;
```

- **`MAX_SEARCH_HOPS = 6`** — the relay clamps any incoming `max_hops` down to this. Trust decays with
  distance; paths beyond ~6 hops carry little meaning. The client default is 4, so legitimate queries never
  clamp; only a client explicitly requesting a deeper-than-serviceable search is bounded (its results stay
  correct, just capped at depth 6).
- **`MIN_SEARCH_TERM_LEN = 3`** — measured in bytes of `term`. A Search below it is rejected before any
  corpus scan. This kills the O(corpus²) empty/short-term amplifier while matching the common search floor.

---

## 3. Enforcement point (serve `Search` arm)

The bounds live in `serve_collab`'s existing `Search` arm, applied before the WS8f-b work:

1. **Reject too-short term (fail-fast, zero work).** If `term.len() < MIN_SEARCH_TERM_LEN`, send
   `Message::Error("search term too short")` and return — **before** the `for a in &refs { objects.get }`
   corpus load and before any BFS. No object is read; no ball is computed.
2. **Clamp `max_hops`.** Otherwise `let max_hops = max_hops.min(MAX_SEARCH_HOPS);` then run the existing,
   unchanged path: load the served corpus, `search_ball(&corpus, &term, max_hops)`, pack the selected ids,
   `Pack` + `Done`.

The reverse ball is still computed — with the **clamped** `max_hops`. `search_ball` itself is not modified;
the serve arm simply passes it a bounded depth and never calls it at all for a too-short term.

---

## 4. Client behavior (fail-fast + skip; no CLI surface change)

Two small changes, neither a new command or flag:

- **Library.** `collab_search` / `collab_search_authenticated` already treat any non-`Pack` reply after a
  `Search` as `Err` (the `_ => Err("expected Pack")` arm). A relay's `Message::Error` for a too-short term
  therefore surfaces as `Err`, and `query_relay_set`'s existing skip-and-continue drops that relay while
  still returning hits from healthy relays. No library logic change is required beyond confirming the
  `Error` reply maps to `Err` (add an explicit `Message::Error(e) => Err(...)` arm after the `Search` send
  if the current arm doesn't already cover it, matching how the Welcome step handles `Error`).
- **CLI.** `discover relay <term> …` rejects a `term` shorter than `MIN_SEARCH_TERM_LEN` **locally, before
  connecting**, with a clear error (`search term must be at least 3 characters`). This is stricter input
  validation — not a new flag or command, and an empty/1-char relay search was never useful — so it is not
  a CLI surface regression. It gives the user a precise local message instead of a silent "no strangers
  matched" and avoids a pointless connection. The ad-hoc `--endpoint` and pinned-set paths share the
  check.

---

## 5. Testing

**Unit (serve policy)**

- **Clamp:** a `Search { term: "target", max_hops: 255 }` against a relay whose corpus has edges deeper than
  6 hops from a match returns exactly the ball a `max_hops = 6` request would — i.e. the served edge set
  equals `search_ball(corpus, "target", 6)`, not the `max_hops = 255` ball. (Assert equality to the
  6-bounded ball, proving the clamp took effect.)
- **Reject + zero work:** a `Search { term: "ab", max_hops: 4 }` (2 bytes) yields `Message::Error` and no
  `Pack`. Assert the reply is `Error`. (Where observable, assert no corpus objects were served — e.g. the
  connection receives only `Error`, never a `Pack`.)

**Loopback TCP**

- A relay rejects a too-short-term Search: the client's `collab_search` returns `Err`.
- A `max_hops`-above-ceiling Search returns the clamped, correct, bounded result (the matching stranger is
  still found; the returned ball matches the depth-6 ball).
- `query_relay_set` over two relays where the query term is valid returns merged hits; and a relay that
  errors on a too-short term is skipped while a healthy relay's hits still appear (skip-and-continue).

**CLI E2E (real `bole` binary)**

- `discover relay "ab"` (2 chars) fails locally with a clear "at least 3 characters" message and never
  connects (assert non-zero exit / error output; assert no relay round-trip is needed).
- `discover relay "Pat"` still returns the stranger unchanged (the bound does not affect normal queries).

---

## 6. Scope boundary (→ WS8f-c2 / -c3 / -c4 / later)

WS8f-c1 defines: the two fixed Search cost bounds (`MAX_SEARCH_HOPS` clamp, `MIN_SEARCH_TERM_LEN` reject),
relay-side enforcement in the serve `Search` arm, and client fail-fast + skip-and-continue with a CLI
pre-check. Explicitly out:

- **Rate limiting / request-frequency / connection budgets** (repeated well-formed base scans; a client
  monopolizing the sequential serve loop) → WS8f-c2, which is intertwined with the deferred concurrent-serve
  work.
- **Relay-side author denylists** (operator withholds specific author keys from aggregate/serve —
  availability/hygiene, withhold-only) → WS8f-c3.
- **Querier-side result filters** (client mutes keys/terms in `discover` output) → WS8f-c4.
- **Operator-configurable bound values**, a **matched-set cap** (bounding how many matches spawn balls even
  for a valid term), and any **per-client identity/quota** (the relay does not authenticate clients) — all
  later refinements.

WS8f-c1 is the tightest, most concrete protection for the exact surface server-side Search opened: it caps
the O(corpus²) amplifier back to ~O(corpus) with two constants and a fail-fast check, changing only how much
work a relay will do — never what a discovered stranger means.
