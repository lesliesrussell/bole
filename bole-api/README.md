<!-- bole-3xj5 -->
# bole-api

`bole-api` is an HTTP/JSON read API server over a `bole` store. It exposes a
small set of read-only endpoints (status, timelines, snapshots, blobs, repos,
profiles) backed directly by the `bole` library — the same ACL/accessor logic
used by `bole-cli` and the sync path governs every response, so the API server
adds no new authorization semantics of its own.

There is no write surface: this crate is a read API only.

## Usage

```bash
bole-api --store <path/to/.bole> --listen 127.0.0.1:8080 --config auth.toml
```

- `--store <path>` (required): path to the `.bole` store directory to serve.
- `--listen <addr>` (default `127.0.0.1:8080`): address to bind.
- `--config <path>` (optional): path to an auth config TOML file. If omitted,
  every request resolves to the `Anonymous` principal (i.e. only whatever is
  ACL-visible to anonymous access is served).

## Auth config (TOML)

The config maps credentials presented on a request to a `bole` actor name,
plus (for the mTLS arm) which proxy peers are trusted to assert a client
identity. All four sections are optional; an absent section behaves as empty.

```toml
[tokens]
# bearer-token -> actor
"t-secret-abc123" = "alice"

[mtls]
# client-cert-subject -> actor (only honored via the proxy header; see below)
"CN=bob,O=example" = "bob"

[keys]
# named ed25519 signing key -> { pubkey (64 hex chars), actor }
[keys.carol-laptop]
pubkey = "6f...<64 hex chars>...c1"
actor  = "carol"

[proxy]
# peer IPs allowed to set X-Bole-Client-Subject (see mTLS note below)
trusted = ["127.0.0.1", "10.0.0.5"]
```

Resolution order per request: `Authorization: Bearer <token>` →
`Authorization: Signature keyId="...",sig="..."` (ed25519 signed request) →
`X-Bole-Client-Subject` (only if the immediate peer is in `[proxy].trusted`) →
`Anonymous`.

This order is **strict precedence, not fall-through**: the first presented
credential class decides the outcome. An `Authorization` header is resolved on
its own — a valid one authenticates, an unrecognized scheme or unmapped
credential is a **401** — and it never falls through to the
`X-Bole-Client-Subject` mTLS arm. So a request that carries both an
`Authorization` header and a trusted-proxy subject authenticates as the
`Authorization` identity, and a request whose `Authorization` header is
malformed is rejected rather than silently demoted to mTLS or anonymous.

Presented-but-unknown credentials are **401**, never a silent downgrade to
anonymous: a bearer token or trusted-proxy mTLS subject that maps to no
configured actor is rejected, so a stale or typo'd credential surfaces as an
auth failure instead of quietly serving only anonymous-visible data. Only a
request that presents no credential at all resolves to `Anonymous`.

## Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/v1/status` | Service name, version, and ref count. Anonymous-readable. |
| GET | `/v1/repos` | Lists the single store this server hosts (one entry, forward-compatible shape). |
| GET | `/v1/timelines` | Lists all refs (timelines and tags) with head/policy. |
| GET | `/v1/timelines/{name}` | A single ref's detail, including `created_at`. |
| GET | `/v1/snapshots/{id}` | ACL-filtered snapshot metadata (author, message, parents, `visible_paths`). |
| GET | `/v1/snapshots/{id}/blob?path=` | Raw bytes of a blob at `path`, restricted to paths visible to the caller. |
| GET | `/v1/profiles/{key}` | A published `Profile` by 64-hex collab key, after signature verification. |
| GET | `/v1/proposals` | Open change proposals (PRs), verified fail-closed. |
| GET | `/v1/proposals/{id}` | One proposal by object id, with its review-comment thread; 404 if unknown. |
| GET | `/v1/boards/{board}` | A discussion board's posts (each with its `parent` for threading), verified fail-closed. |

All endpoints except `/v1/status` run through the `RequestAuth` extractor and
thus require (or default to) a resolved principal; `/v1/status` does not
require auth and is always anonymous-readable.

## Error envelope

Every error response is JSON of the shape:

```json
{ "error": { "code": "not_found", "message": "no such snapshot" } }
```

`code` is one of `bad_request`, `unauthorized`, `not_found`,
`method_not_allowed`, or `internal`, with a matching HTTP status code. Every
error uses this envelope — handler errors (e.g. a well-formed snapshot id that
does not exist yields a 404 envelope), unmatched routes and wrong methods (via
`Router::fallback` / `method_not_allowed_fallback`), and extractor rejections
(via the `ApiPath`/`ApiQuery` wrappers, which preserve the rejection's real
status — a malformed query is a 400, a route/handler param-arity mismatch is a
500 — without leaking deserializer detail).

## ACL model

There is no bespoke API-layer authorization. `RequestAuth` resolves the
request's `Principal` (`Token`, `SshKey`, `Mtls`, or `Anonymous`), maps it to
an actor name via the auth config's `ActorMap`, and calls
`bole::sync::authn::accessor_for` to build the same `Accessor` the CLI and
sync path use against the repo's ACLs. Handlers then call ACL-aware
repository methods (e.g. `get_snapshot_filtered`) with that `Accessor` — a
path or snapshot the accessor cannot see simply doesn't appear
(`visible_paths` omits it, or the whole snapshot 404s), so hidden data reads
as a plain 404 rather than a distinguishable 403.

## Deployment assumptions

**mTLS via trusted proxy.** This server does not terminate TLS or verify
client certificates itself. It expects to sit behind a TLS-terminating proxy
that performs mTLS client-cert verification and forwards the verified
subject in an `X-Bole-Client-Subject` header. That header is only honored
when the *immediate* TCP peer's IP is listed in `[proxy].trusted` — from any
other peer the header is ignored and the request falls through to the next
auth arm (or `Anonymous`). Never expose this port directly to untrusted
clients if you rely on the mTLS arm: any client that can reach the port
directly (bypassing the trusted proxy) could otherwise spoof the header.

**Signed-request canonical string.** The `Authorization: Signature
keyId="...",sig="..."` scheme verifies an ed25519 signature over:

```
"bole-http-req-v1\0" + METHOD + "\n" + TARGET + "\n" + X-Bole-Date + "\n" + hex(sha256(body))
```

where `METHOD` is the request method and `TARGET` is the full request target —
the path **and** query string (e.g. `/v1/snapshots/{id}/blob?path=src/main.rs`),
so query parameters cannot be altered in transit without breaking the signature.
`X-Bole-Date` is a unix-seconds timestamp header, and the body hash is the
hex-encoded SHA-256 of the request body (empty string for the GET-only
endpoints this server currently exposes). The `X-Bole-Date` value must be
within ±300 seconds of the server's clock or the request is rejected as
`unauthorized`, to bound replay of captured signatures.

**Troubleshooting a `signed request rejected` 401.** Every failure in the
signed arm returns that one generic message on purpose — distinct texts would
let a caller probe which `keyId`s are registered. So a single 401 covers all
of: missing/malformed `keyId` or `sig`; a missing or non-numeric
`X-Bole-Date`; a timestamp outside the ±300s window; an unknown `keyId`; and a
signature that does not verify. When debugging your own client, check them in
that order — clock skew and a canonical-string mismatch (wrong method, or a
`TARGET` that omits the query string) are the usual causes. The server treats
an unknown key exactly like a bad signature, including running a verify against
a dummy key, so response timing does not reveal whether a `keyId` exists.
