# bole-api — Walking-Skeleton Read API Server — Design

**Date:** 2026-07-04
**Status:** Approved, pending implementation plan

## Problem

bole is the headless backend for Grove (the future hub frontend, separate repo).
Today the only machine surfaces are the Rust library, the CLI `--json`, and the
p2p `node serve` / `sync/http.rs` wire protocol (git-style pack fetch/push).
None of those is a **product-facing HTTP/JSON API** a web frontend calls. Grove
cannot be built against bole until such a surface exists.

## Goal

Stand up a minimal HTTP/JSON API **server** that pins every cross-cutting
contract — framework, authentication, error/JSON envelope, versioning — on a
small, safe, **read-only** surface over operations that already exist. Once this
walking skeleton lands, each future domain operation (profile-bundle, PRs,
board) is just another handler and gets its own spec.

## Non-goals (this slice)

- Any write/mutation endpoint (snapshot create, timeline advance, PR ops).
- Tree-walk / path listing within a snapshot; pagination; filtering DSLs.
- The still-missing domain operations themselves (profile-bundle query, PR /
  change-proposal objects, message board). Each is a separate spec built on this
  skeleton.
- Native TLS termination with client-certificate verification (see Auth §mTLS).
- Changes to the `bole` core library beyond additive read helpers, if any.

## Architecture

### Crate & process

New workspace member `bole-api/` — a **binary** crate depending on the `bole`
library plus `axum`, `tokio`, `serde` / `serde_json`, and `tracing`. The core
`bole` library stays HTTP-free; no web-framework dependency leaks into it or the
CLI.

```
bole-api --store <path-to-.bole> --listen 127.0.0.1:8080 [--config auth.toml]
```

On start it opens `Repository::disk(store)` (used read-only in this slice),
loads the auth config into an `authn::ActorMap` and a key registry, builds the
axum `Router`, and serves. A single shared `Arc<Repository>` backs all handlers.

### Module layout

Small, single-purpose units:

- `main.rs` — arg parsing, build `AppState`, bind and serve.
- `state.rs` — `AppState { repo: Arc<Repository>, actors: ActorMap, keys: KeyRegistry }`.
- `auth.rs` — the only genuinely new auth logic: HTTP request → `Principal` →
  (via `authn::accessor_for`) → `Accessor`. Exposed as an axum extractor.
- `error.rs` — `ApiError`: maps `bole::Error` and ACL denials to an HTTP status
  and the JSON error envelope.
- `router.rs` — route table.
- `handlers/{status,repos,timelines,snapshots,profiles}.rs` — thin handlers that
  call existing lib read ops and serialize. The `snapshots` handler serves both
  the metadata route and the `blob?path=` sub-route.

## Authentication & authorization

The authorization core **already exists** and is reused unchanged:
`sync::authn::accessor_for(store, actor_map, principal) -> Accessor` turns a
`Principal` into the same ACL-checked `Accessor` the sync path and CLI use.
There is no second authorization model — every handler passes its `Accessor`
into the existing read op, so ACL is enforced identically to the rest of bole.

The **only** new auth code is extracting a `Principal` from the HTTP request.
Channels, concretely over HTTP (v1):

- **Token** — `Authorization: Bearer <t>` → `Principal::Token(t)`.
- **mTLS** — v1 assumes a TLS-terminating reverse proxy verifies the client
  certificate and forwards its subject in a trusted header
  `X-Bole-Client-Subject`, honored **only** when the immediate peer is an
  allowlisted proxy address → `Principal::Mtls(subject)`. Native rustls
  client-cert termination is a documented follow-up, not this slice.
- **Signed request** (the `SshKey` channel — there is no SSH over HTTP, so this
  is bole-native ed25519): `Authorization: Signature keyId="…",sig="…"` over a
  domain-tagged canonical string:

  ```
  bole-http-req-v1\0
  <METHOD>\n
  <PATH>\n
  <X-Bole-Date>\n
  <hex(sha256(body))>
  ```

  verified against the registered ed25519 public key for `keyId` →
  `Principal::SshKey(keyId)`. `X-Bole-Date` must fall within a bounded
  clock-skew window (reject otherwise) to prevent replay. This reuses bole's
  existing ed25519 verification and domain-separation-tag convention (cf.
  `authn::verify_ref_op`, `acl::authority`).

- **No or unrecognized credential** → `Principal::Anonymous` → public
  (lattice-bottom) readable data only. Unauthenticated public reads still work;
  the ACL decides.

### Auth config

A TOML file loaded at startup:

```toml
[tokens]
"<opaque-token>" = "alice"

[mtls]
"<cert-subject>" = "bob"

[keys]
"<keyId>" = { pubkey = "<64-hex-ed25519>", actor = "carol" }

[proxy]
trusted = ["127.0.0.1"]   # peers whose X-Bole-Client-Subject is honored
```

`[tokens]` / `[mtls]` / `[keys]` populate `ActorMap` (+ a `KeyRegistry` mapping
`keyId → pubkey` for signature verification). Absent config ⇒ everything is
anonymous.

## Endpoints (read-only)

All under the `/v1` version prefix. Each is access-checked via the request's
`Accessor`.

| Method & path | Returns | Backed by |
|---|---|---|
| `GET /v1/status` | server + repo info (version, ref count, store id) | `refs.list` + `repo info` |
| `GET /v1/repos` | single-element list describing the one store this server hosts (id, counts) | `repo info` (no multi-repo primitive exists; forward-compatible collection shape) |
| `GET /v1/timelines` | timelines/tags (name → head, kind) | `refs.list("")` + `refs.get` |
| `GET /v1/timelines/{name}` | one ref's head + metadata | `refs.get` |
| `GET /v1/snapshots/{id}` | snapshot metadata + **ACL-filtered** `visible_paths` (path → blob id, only paths this accessor may read) | `Repository::get_snapshot_filtered(id, accessor)` |
| `GET /v1/snapshots/{id}/blob?path=<p>` | raw blob bytes for `path`, **only if** `path` is in that snapshot's `visible_paths`; else 404 | `get_snapshot_filtered` + `objects.get` |
| `GET /v1/profiles/{key}` | a `Profile` object by 64-hex collab key (verified) | `Repository::profile(&Key)` + `verify_profile` |

**Access-control note.** Reads go through path/timeline-aware primitives, never
raw object-by-hash. `get_snapshot_filtered` returns only the paths the request's
`Accessor` may read, so `snapshots/{id}` and the `blob` sub-route are ACL-correct
for anonymous and authenticated callers alike (the ACL decides visibility).
There is deliberately **no** `GET /v1/objects/{id}`: a content-addressed by-hash
read carries no path context and would bypass path-label ACL.

## JSON contract & errors

- **Success** bodies reuse the CLI `--json` serde types wherever one already
  exists, so the API and CLI cannot drift. New response types are added only
  where the CLI has no equivalent.
- **Error envelope**: `{ "error": { "code": "<slug>", "message": "<text>" } }`.
  Status mapping: `400 bad_request`, `401 unauthorized`, `404 not_found`,
  `500 internal`.
- **ACL-hidden resources return `404`, not `403`** — bole's model hides what an
  actor is not cleared to see, so the API must not leak existence. A `403` is
  used only where existence is already known to the caller and the *action* is
  refused (not reachable on the read-only surface, so effectively all denials on
  hidden resources are 404 in this slice).
- `/v1` is the version boundary; a breaking change bumps to `/v2`.
- Content-type `application/json` everywhere except raw blob bytes
  (`application/octet-stream`).

## Testing

In-process axum tests via `tower::ServiceExt::oneshot` against a temporary
`.bole` repository seeded through the library. Because handlers are thin (domain
logic is already lib-tested), tests target routing, auth, serialization, and
status mapping:

- Each endpoint happy path.
- Anonymous read of public (lattice-bottom) data succeeds.
- Token that maps to a cleared actor reads scoped data.
- ACL-hidden resource → `404` (no existence leak).
- Unknown id → `404`; bad token → `401`; malformed request → `400`.
- Signed-request happy path; replay / out-of-skew `X-Bole-Date` → rejected.
- `X-Bole-Client-Subject` honored only from an allowlisted proxy peer; ignored
  otherwise.

## Files added

- `bole-api/Cargo.toml`
- `bole-api/src/{main,state,auth,error,router}.rs`
- `bole-api/src/handlers/{mod,status,repos,timelines,snapshots,profiles}.rs`
- `bole-api/tests/…`
- workspace `Cargo.toml` — add `bole-api` member.

## Follow-ups (explicitly deferred)

- Native rustls client-cert termination (remove the proxy-header assumption).
- Write endpoints and the domain operations they need.
- Profile-bundle aggregating query (its own spec) as the first domain op on this
  skeleton.
- Pagination / tree listing / richer query params.
