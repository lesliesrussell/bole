# WS5 — Locked Implementation Decisions (bole-cy6)

Resolves the open questions in
[specs/2026-06-29-ws5-distributed-sync.md](../specs/2026-06-29-ws5-distributed-sync.md).
Maintainer-approved 2026-07-01.

| OQ | Decision |
|----|----------|
| **Scope** | Build the **core sync engine, in-process**: negotiate (have/want + missing-closure), pack transfer (reuse WS4), `fetch` (remote-tracking refs), `push` (CAS on heads via sk6 `RefTransaction`, fast-forward gated), `clone_from`. Defer the wire codec + `Transport` trait + HTTP/SSH (**bole-6qy**), the signed policy-verification chain + `TrustStore` (**bole-0tp**), and authn/authz mapping (**bole-6h7**). |
| **O1 — transport** | HTTP first when built (bole-6qy). |
| **O2 — policy-admin keys** | Direct-anchor signing in v1 (bole-0tp). |
| **O3 — grant distribution** | Reachable-from-`PolicyRoot` (bole-0tp). |
| **O4 — read filtering** | Ref-granularity via `list_refs_filtered`; whole closure of readable refs transfers. Object-level label filtering deferred. |
| **O5 — forked policy** | Refuse + manual re-anchor (bole-0tp). |
| **O6 — have encoding** | Exact ids in v1; Bloom behind a cap later. |
| **O7 — pack read-lease** | Rely on grace + POSIX unlinked-open in v1. |
| **O8 — per-ref-kind CAS** | Timeline = fast-forward/policy-gated CAS; tags create-only; policy via §5.3 (later). |
| **O9 — server statefulness** | One logical exchange per verb (relevant once bole-6qy adds transports). |

## Core engine shape (this pass)

`src/sync/` (new, additive):
- `sync::negotiate` — `have_set(repo)`, `missing_closure(src, wants, have)` walking
  the WS4 §6.2 + WS1 policy edges, pruning any id the receiver already has (the
  content-addressing pruning invariant: having X ⇒ having X's whole closure).
- `Repository` gains in-process peer methods (peer is a `&Repository`; the
  spec's `InProcessTransport` backbone, also its test backbone):
  - `fetch(remote_name, from, accessor)` — advertise `from.list_refs_filtered`,
    transfer the missing closure, set `refs/remotes/<remote>/<name>` (plain set,
    no CAS — fetch owns these).
  - `push(to, timelines, accessor)` — land the missing closure on `to` FIRST,
    then CAS each head via `advance_head_if` (expected_old = `to`'s current head),
    fast-forward-gated by the timeline policy; per-ref `Ok` /
    `NonFastForward { server_head }` / `Denied`.
  - `clone_from(from, accessor)` — maximal fetch into an empty repo + create local
    timelines + remote-tracking refs.

Transfer exercises the real WS4 pack path: sender builds a pack from the closure
(`PackBuilder` over canonical bytes), receiver `decode_pack`-verifies each frame
(BLAKE3 id) then lands objects (`put_raw`). Objects-before-refs: objects land
before any CAS, so a failed push leaves harmless orphans.

Supporting additions:
- `RefOp::Set` + `RefTransaction::set` — unconditional upsert for remote-tracking refs.
- `ObjectStore::get_raw`/`put_raw` — canonical-bytes access for pack transfer.

## Deferred (with beads)
- **bole-6qy** — wire `Message` codec + `Transport`/`Conn` trait + `SyncSession` + HTTP/SSH.
- **bole-0tp** — `TrustStore` + `PolicyVerifier` + signed policy chain + TOFU clone (subsumes bole-fz1).
- **bole-6h7** — `PeerIdentity`/`ActorMap` → `Accessor`, server push authz, `SIGNED_REFS`.
