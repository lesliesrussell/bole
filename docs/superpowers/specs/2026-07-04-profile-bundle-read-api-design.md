# Profile-Bundle Read API

- **Status:** design (approved 2026-07-04), not yet implemented
- **Bead:** `bole-k93a`
- **Context:** bole is the **headless backend / API** for Grove; Grove (the user-facing frontend) is built
  later in a separate repo and is out of scope here. This is the **first Grove-backend domain operation**:
  a transport-agnostic aggregated read that a frontend consumes directly. It proves the
  "aggregate-read-as-API / hub-ready bundle" pattern that PR and board read-surfaces will reuse.
- **Depends on:** WS8a substrate (`Profile`, `TrustEdge`, `TrustKind`, `verify_profile`/`verify_edge`,
  `Key`, `key_hex`); `Repository::{public_profiles, public_edges, tracked_collab}`; the timelines/refs
  layer (`Ref::Timeline`, `Snapshot`); the `profile` CLI command group.
- **Successor work (out of scope here):** in-edges ("who trusts this dev"); a multi-repo-per-identity
  registry (the real "public repos" list); remote/relay fetch of a stranger's bundle; deeper per-timeline
  history walks; the HTTP/JSON-RPC transport (`bole-yd56`). All later.

## Thesis

One read operation — `Repository::profile_bundle(key)` — aggregates everything bole can **locally verify**
about a developer key into a single, stable, JSON-clean bundle: their identity (Profile), their own trust
statements (out-edges), and — when this repo is that key's hub — the repo's timelines. The library returns a
typed struct; the CLI renders it as `bole profile bundle [<key-hex>] [--json]`. No HTTP, no mutation, no UI.
Keys are raw hex throughout; every signed object is verified fail-closed before it is emitted.

## 1. Scope & invariants

Chosen scope: **by-key with layered availability.** Collab-layer content (Profile, trust edges) is
key-attributed and available for the local identity *and* any tracked peer; repo-layer content (timelines)
is repo-scoped and included only for the local repository's own identity.

**Hard invariants:**

1. **Read-only.** `profile_bundle` takes `&self`, performs only reads, and persists nothing.
2. **Fail-closed verification.** Every `Profile` and `TrustEdge` emitted is verified
   (`verify_profile`/`verify_edge`) regardless of source — `public_profiles`/`public_edges` do **not**
   verify on read, so the bundle does — and anything that fails to verify is **dropped**, never emitted.
3. **Transport-agnostic.** The library returns typed data (no JSON/HTTP dependency in its shape); the CLI
   owns hex-rendering and `--json`, mirroring `discover`/`trust`. A thin HTTP veneer can wrap it later.
4. **Keys canonical / raw hex.** All keys in output are raw 64-hex (`key_hex`/`key::hex32`), never
   fingerprint.
5. **Local depth-2 / discovery untouched.** No change to `discover`/`follow_*`/relay code.

## 2. Identity resolution & `is_local`

`profile_bundle(key)`; the CLI defaults `key` to the caller's own key via `signer_from` when no key is
given (mirrors `profile show`).

- **`is_local`** — true iff `key` has a published **public** profile in this repo (i.e. `public_profiles()`
  contains a profile whose `key == key`). This is the operational definition of "this repo is that key's
  hub."
- **profile** — if `is_local`, the key's own published profile; else the tracked-peer profile for `key`
  from `tracked_collab` (a `CollabObject::Profile` with that key); else `None`. The emitted profile is
  verified fail-closed; if verification fails it is treated as absent (`None`).
- **unknown key** — `profile: None`, `edges: []`, `timelines: []`, `is_local: false`.

## 3. Trust slice (out-edges, verified)

The key's own published trust statements — edges where `from_key == key`:

- **local key:** `public_edges()` filtered to `from_key == key`.
- **peer key:** `tracked_collab()` `TrustEdge`s with `from_key == key`.

Each edge is verified (`verify_edge`) and dropped if it fails. v1 is **out-edges only** (who the key
follows / vouches / reviews — self-signed, unambiguous). In-edges ("who trusts this dev") are deferred.

## 4. Timelines (local only, head summary)

When `is_local`, the repo's timelines (its projects/branches). Enumerate `Ref::Timeline` entries; for each,
read its head `Snapshot` and emit `{ name, head, author, created_at }` where `head` is the head snapshot's
`ObjectId`, and `author`/`created_at` are that `Snapshot`'s fields. This is the "latest activity" — the head
snapshot per timeline. When `!is_local` (peer or unknown), `timelines: []` (we do not host their repo).
Deeper per-timeline history walks are deferred.

> Note: `Snapshot.author` is free-text (not a `Key`), and timelines are repo-scoped, not key-attributed —
> hence timelines belong to "this repository" and are surfaced only for the repo's own identity. A real
> multi-repo-per-dev "public repos" list requires a registry that does not yet exist (future).

## 5. Library API + CLI + JSON contract

**Library** (`src/repo/collab.rs` or a focused `src/repo/bundle.rs`):

```rust
pub struct TimelineView {
    pub name: String,
    pub head: ObjectId,
    pub author: String,
    pub created_at: u64,
}

pub struct ProfileBundle {
    pub key: Key,
    pub is_local: bool,
    pub profile: Option<Profile>,     // verified, or None
    pub edges: Vec<TrustEdge>,        // verified out-edges (from_key == key)
    pub timelines: Vec<TimelineView>, // repo timelines when is_local, else empty
}

impl Repository {
    /// Aggregate the locally-verifiable hub view of `key`: identity + own trust
    /// out-edges (+ this repo's timelines when `key` is the repo's own identity).
    /// Read-only; every emitted object is verified fail-closed.
    pub async fn profile_bundle(&self, key: &Key) -> Result<ProfileBundle>;
}
```

Pure types — no `serde_json` in the library shape; the CLI renders hex + JSON, exactly as `discover`/`trust`
do.

**CLI** — a third `profile` subcommand (`bole-cli/src/commands/profile.rs`, alongside `Set`/`Show`):

```
bole profile bundle [<key-hex>] [--json]
```

`<key-hex>` optional (defaults to own key via `--key-env`/`--key-file` → `signer_from`, like `Show`). `--json`
emits the contract below; human output prints a compact summary (display name, is_local, edge count, timeline
count). Keys everywhere raw hex; seeds from env/file, never argv.

**JSON contract (what Grove consumes):**

```json
{
  "key": "<64-hex>",
  "is_local": true,
  "profile": {
    "key": "<64-hex>",
    "display_name": "…",
    "bio": "…",
    "endpoints": ["…"],
    "dns_aliases": ["…"],
    "seq": 3
  },
  "trust": {
    "edges": [
      { "to": "<64-hex>", "kind": "follow", "petname": "…", "seq": 1 }
    ]
  },
  "timelines": [
    { "name": "main", "head": "<hex-objid>", "author": "…", "created_at": 1720000000 }
  ]
}
```

**Null/empty conventions (stable, explicit):**

- `key` — always present (echoes the resolved key, raw hex).
- `is_local` — always present (bool).
- `profile` — `null` (never omitted) when unknown/unverifiable; otherwise the object above. `bio` is `""`
  when empty; `endpoints`/`dns_aliases` are `[]` when empty. `seq` is the profile's `seq`.
- `trust.edges` — `[]` (never `null`) when none. Each edge: `to` (raw hex), `kind` ∈
  `"follow"|"vouch"|"review"`, `petname` (`string` or `null`), `seq`.
- `timelines` — `[]` (never `null`) when `!is_local` or the repo has none.

## 6. Testing

**Unit (library, `profile_bundle`)**

- **own identity:** publish a profile + `public_edges` (some `from_key == own`) + create a timeline with a
  snapshot; `profile_bundle(own)` returns `is_local: true`, `Some(profile)`, the own out-edges, and the
  timeline with the correct head/author/created_at.
- **peer from cache:** a tracked peer's profile + their out-edges in `tracked_collab`;
  `profile_bundle(peer)` returns `is_local: false`, `Some(profile)`, their out-edges, `timelines: []`.
- **unknown key:** `profile_bundle(random)` → `is_local: false`, `profile: None`, `edges: []`,
  `timelines: []`.
- **fail-closed:** a cached profile (or edge) whose signature does not verify is **dropped** — a tampered
  peer profile yields `profile: None`; a tampered edge is absent from `edges`.
- **out-edges only:** an edge where `to_key == key` (someone else vouching for the key) is **not** in the
  bundle (v1 emits only `from_key == key`).

**CLI E2E (real `bole` binary)**

- `profile set` + create a timeline + `trust follow <peer>` → `profile bundle --json` shows the populated
  contract (`is_local: true`, profile fields, one follow edge, one timeline) with keys as raw hex.
- `profile bundle <peer-hex> --json` (peer pulled into cache) shows `is_local: false`, the peer profile +
  their edges, `timelines: []`.
- `profile bundle <random-hex> --json` shows `is_local: false`, `profile: null`, `trust.edges: []`,
  `timelines: []`.

## 7. Scope boundary (→ future)

In: the by-key aggregated read (identity + own out-edges + local timelines), fail-closed verification, the
typed library API + CLI `--json` contract with explicit null/empty semantics. Out (later slices): in-edges;
a multi-repo-per-identity registry (the real "public repos" list); remote/relay fetch of a stranger's
bundle (v1 is local-cache only); deeper per-timeline history; the HTTP/JSON-RPC transport (`bole-yd56`);
PR/board read-bundles (which reuse this pattern).

This is the first "bole as backend API" surface: a single, stable, verified, JSON-clean read a Grove
frontend can consume directly — establishing the aggregate-read pattern the rest of the hub API will follow.
