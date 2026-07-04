# bole / Grove — Project STATUS

> Living map of where the project stands: what's shipped, what's deferred, what was never started.
> Regenerate the backlog view with `bd list` (beads are the source of truth for open work;
> this file is the human-readable narrative). Last consolidated: 2026-07-04.

## What this is

**bole** — a content-addressed, distributed, access-controlled VCS with humans *and* agents as
first-class actors. It is the **headless backend / API** for Grove.

**Grove** — the user-facing collaboration hub frontend ("a better GitHub than GitHub"). **Built later,
in a separate repo. Out of scope here.** No UI, rendering, or web app lives in bole.

**bole's job:** expose *every operation a great hub frontend needs* — profiles, repos, timelines, PRs,
discussions, discovery, trust, access — as a clean, consumable, JSON-serializable API, and enforce the
rules. Grove renders; bole serves the data and owns the invariants.

Scale today: ~23.7k LOC, 492 tests (green), broad CLI. **"All the API endpoints" splits in two:**
(1) **domain operations** — the backend features themselves (transport-agnostic; PR/board/profile-bundle
still missing); (2) **transport** — how Grove calls them. Today the machine surface is the Rust library
+ CLI `--json` (~24 command groups). There is **no product-facing API server** (HTTP/JSON-RPC); the only
network surface is the p2p `node serve` wire protocol, which is not what a web frontend calls.

## Pillar scorecard

| Pillar | Built | Missing / deferred |
|---|---|---|
| **VCS core** | object store (BLAKE3), tags/timelines, packs/GC, virtual repos, git projection/import | thin-pack deltas (fwd-compat), submodules, annotated-tag export gap, `--prune` |
| **Secure (access)** | label-lattice ACL (`acl/`, largest module), clearances, policy hooks, authority/signing | WS1-O2 approval surfacing, O4 attestation format, O5 unknown-hook fail-closed, audit logging, timeline-policy *enforcement* (verify) |
| **Distributed** | WS5 sync (pack-delta wire), WS8b networked node, relays, cache-and-forward | node liveness (concurrent serve daemon + poll), GC-lease-on-stream |
| **Discoverable** | WS8a–e substrate + trust-paths; WS8f-a/b/c1 relays, multi-relay, server-side search, cost bounds | WS8f-c2 rate-limit/budgets, c3 denylists, c4 querier filters, WS8f-d reputation, DNS alias verify, Profile recency timestamp, persistent search index |
| **PR system** (API) | — (only the reserved `Review` trust-edge kind) | change-proposal + review-thread objects + operations (create/list/comment/resolve/merge) — no UI |
| **Message board** (API) | — | discussion/board objects + operations (post/list/reply) — no UI |
| **Dev landing page** (API) | `Profile{…}` object (data only) | a *profile-bundle query* — profile + a dev's public repos/timelines + trust graph — as JSON. Grove renders it |
| **Transport / API server** | Rust library + CLI `--json` | product-facing HTTP/JSON-RPC layer Grove calls (only p2p `node serve` exists today) |

## Shipped tags (chronological)

`ws8b-networked-node` → `ws8c-cache-and-forward` → `ws8d-relay-and-stranger-search` →
`ws8e-trust-path-and-ranking` → `ws8f-a-trusted-relay-set-and-multi-relay-query` →
`ws8f-b-server-side-search-verb` → `ws8f-c1-relay-search-cost-bounds`

(Foundations gate1–8 and WS1–7 predate tagging; they live in `git log` + the `06-26`…`06-29` specs.)

## Strategic read

We went **very deep on one pillar — discoverability** (10 slices, WS8a→f-c1: a sophisticated,
well-verified trust-discovery substrate). The other product pillars a human would *see and use* —
**PRs, discussions, landing pages — are unbuilt.** We have excellent plumbing for surfaces that don't
exist yet. That is the "in the weeds" feeling, and it's real.

**Closest-to-shippable surface:** the dev landing page. `Profile` already exists and is already
*discovered* by the WS8 machinery; the gap from "we have profiles" to "a dev has a discoverable hub
page" is small. **Highest-value-but-heaviest:** the PR system (the headline feature; builds on
timelines + ACL + approvals + the `Review` edge).

**Process note:** three subagent code-reviews died on infra during the WS8f work; each was
controller-verified on small diffs. Fine at this size, but a reason to prefer smaller, self-contained
next slices until that's understood.

## Deferred backlog

Tracked in beads (`bd list`). Grouped by track; see each spec's scope-boundary section for detail.

- **Discovery hardening (WS8f tail):** c2 rate-limit/connection-budgets (entangled with concurrent
  serve), c3 relay author denylists, c4 querier-side result filters, d relay reputation/weights.
- **Discovery quality:** DNS `.well-known`/TXT alias verification + local petname-set command;
  `Profile` recency timestamp (cross-author ranking); persistent server-side search index.
- **Distribution:** node liveness (concurrent spawned-serve daemon + background poll).
- **Security follow-ups:** WS1-O2/O4/O5; WS8b M2 scoped-ref gating + F4 publish TOCTOU; audit logging
  of access decisions and agent state transitions; verify timeline-policy enforcement (Gate2→Gate6).
- **VCS round-trip gaps:** annotated-tag export, submodules, `--prune`, thin-pack deltas.
- **Product surfaces (unstarted, headline):** PR / change-proposal + review-thread; message board /
  discussions; dev landing-page / profile-hub render+serve.

## Suggested next decision

bole is a **backend API**; Grove (frontend) comes later in a separate repo. So the target is
**API completeness**, in two layers:

- **Domain operations** (transport-agnostic backend features): PR objects + ops, board objects + ops,
  and a profile-bundle query. These are real substance a frontend needs and don't exist yet. Build as
  library API + CLI `--json` (JSON-clean by construction, so any transport can wrap them).
- **Transport / API server**: an HTTP/JSON-RPC surface Grove calls. Recommended to **defer** the concrete
  server until Grove's stack/needs are known (avoid building an HTTP API for a client that doesn't exist
  yet and may want a specific shape) — but keep every operation JSON-serializable so the veneer is thin
  when it lands. Alternatively, stand up a minimal `bole serve --api` now to pin the contract early.

Recommended first build: the **profile-bundle query** (cheapest — `Profile` + repos/timelines/trust all
exist, just needs an aggregating read op + `--json`), then the **PR system** (headline, heavier). Pick,
then brainstorm → spec → plan → subagent-driven build.
