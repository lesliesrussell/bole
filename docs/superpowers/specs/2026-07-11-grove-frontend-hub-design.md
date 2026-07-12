<!-- planning sketch -->
# Grove — Frontend Hub Design (v0 sketch)

> **Status: sketch.** A first spec for Grove, the user-facing frontend that
> renders a bole node's data. Grove is a **separate repo, built later**; this
> doc lives in the bole repo only to record the backend contract Grove needs
> and to drive the API-gap beads. Grounded in the `bole-api` surface as of
> 2026-07-11.

## 1. What it is

A web frontend + thin API-gateway for a bole node: the human hub over bole's
headless backend — "a better GitHub than GitHub." Grove renders identity, code,
history, proposals, and discussion; **bole owns all state, crypto, and access
decisions.** Grove holds no source of truth — it is a view + interaction layer
over `bole-api`'s `/v1/…` HTTP/JSON endpoints.

## 2. Non-goals (v0)

- No VCS logic, no crypto, no ACL decisions in Grove — those stay in bole.
- No write path until bole ships write endpoints (`bole-suw5`); **v0 is
  read-only render.**
- Not a multi-tenant hosting platform — one Grove instance fronts one bole
  node/store.
- Grove never re-implements signature verification: everything bole returns is
  already verified fail-closed and `Accessor`-filtered.

## 3. Architecture

```
Browser ── Grove (SSR/SPA) ── bole-api (/v1, HTTP+JSON) ── bole store
                  │
             session/auth: maps a logged-in user → a bole credential
             (bearer token / ed25519 signed request / mTLS-subject header)
```

Grove is stateless with respect to domain data; its only local state is the UI
session plus the bole credential it presents per request. Credential precedence
is bole's: an `Authorization` header wins, strictly, with no fallthrough
(`bole-261x`); presented-but-unknown credentials are 401.

## 4. Surfaces → data (mapped to real endpoints)

| Grove page | Renders | bole-api source | Status |
|---|---|---|---|
| Dev landing / profile | identity, bio, endpoints, own trust out-edges, their timelines | `Repository::profile_bundle` | **gap #1** — HTTP endpoint missing (only `GET /v1/profiles/{key}` + CLI exist) |
| Repo / branches | timelines & tags, heads, policy | `GET /v1/timelines`, `/v1/timelines/{name}` | live |
| Snapshot / file tree | ACL-filtered tree, blob view | `GET /v1/snapshots/{id}`, `/v1/snapshots/{id}/blob?path=` | live |
| PR list / PR detail | proposals + review threads | `GET /v1/proposals`, `/v1/proposals/{id}` | live |
| Discussions / board | threaded posts | `GET /v1/boards/{board}` | live |
| Discovery / people | trust-ranked search results | library `local_discovery_index` + CLI `discover` | **gap #3** — HTTP endpoint missing |

## 5. Backend gaps Grove needs from bole

1. **Profile-bundle HTTP endpoint** — `GET /v1/profiles/{key}/bundle` exposing
   `Repository::profile_bundle` (library + CLI already exist). The landing-page
   primitive; unblocks the whole read-only hub.
2. **Write endpoints** (`bole-suw5`, deferred) — `POST /v1/proposals`,
   `/v1/proposals/{id}/comments`, `/v1/proposals/{id}/merge`,
   `POST /v1/boards/{board}` — so Grove is interactive, reusing
   `RequestAuth → Accessor` + the JSON envelope.
3. **Discovery endpoint** — `GET /v1/discover?term=&hops=` over the local
   trust-ranked index.
4. **Auth-to-user pattern** — a documented story for how a Grove login becomes a
   bole credential (session → per-user token, or a signed-request proxy). Today
   the mapping is entirely the deployer's job; Grove needs a blessed pattern.

## 6. Tech shape (proposals, not decisions)

- SSR-first (fast, shareable profile/repo pages) with progressive
  interactivity.
- A generated typed client from bole-api's JSON contract — the shapes are
  already stable (raw-hex keys, `null`-when-absent, `[]`-when-empty).
- Grove trusts bole's `Accessor`-filtered, signature-verified responses.

## 7. Phased build

- **P0 — read-only hub:** profile landing, repo/timeline browse, snapshot/file
  view, PR read, board read. Needs only gap #1. Everything else is live today.
- **P1 — interactive:** open/comment/merge PRs, post to boards — needs bole
  write endpoints (gap #2).
- **P2 — social:** discovery/people search, trust-graph visualization — needs
  gap #3.
