# WS8b — Networked Sovereign Node + CLI

- **Status:** design (approved 2026-07-03), not yet implemented
- **Depends on:** WS8a collaboration substrate (`src/collab/` — `Profile`, `TrustEdge`,
  `verify_profile`/`verify_edge`, `Index`, `TrustGraph`, `PublicObjectSource`,
  `refs/collab/public/` publication); WS5 distributed sync (`src/sync/` — the `Conn`
  trait, `TcpConn`, framing/wire, object transfer, `serve`).
- **Successor spec (out of scope here):** WS8c — relays / aggregation, depth-2
  friend-of-follow auto-reach, background polling, stranger-reach search UX, real DNS
  alias verification.

WS8b makes WS8a real over the wire with the smallest honest slice: a node publishes
signed collab objects, serves its own public ones over a dedicated endpoint, and a
stateless client pulls a peer's public objects by address into per-peer tracking refs,
then builds the WS8a `Index` from local state and queries it. It also closes the two
network-facing security items WS8a deferred (M2 scoped-ref gating, F4 publish TOCTOU).

---

## 1. Thesis & architecture

A bole node **publishes** signed collaboration objects (WS8a), **serves** its own
public ones over a dedicated collab endpoint built on WS5's `Conn`/wire/object-transfer,
and a **stateless client** pulls a peer's public objects by network address into per-peer
remote-tracking refs, then builds the WS8a discovery `Index` from local state and queries
it.

- **One long-running process:** `bole node serve --listen <addr>`. It must listen to be
  pullable. Everything else is per-invocation, honoring bole's "the CLI has no session"
  principle.
- **Reuse, don't reinvent:** the transport, framing, and object transfer come from WS5;
  the object model, signatures, trust graph, and index come from WS8a. WS8b is wiring +
  CLI porcelain + two security fixes.

**Locked decisions (from brainstorming):**

1. **Dedicated collab-serve endpoint** — advertises only `refs/collab/public/**`, nothing
   else in the repo, regardless of labels. This is the single M2 enforcement point.
2. **Serve-daemon + stateless client** — the daemon only serves (read-only); pull and
   query are stateless per-invocation commands.
3. **Serve-own-only, depth-1 direct reach** — a node serves only objects it authored;
   discovery reach is peers whose address you have. Depth-2 friend-of-follow
   auto-resolution is deferred to WS8c.
4. **Pulled objects are attributable and prunable** — stored under
   `refs/collab/remotes/<peerkey-fp>/…`, never merged into the node's own published set.

---

## 2. Collab-serve endpoint (server side)

A new `serve_collab(conn: &mut dyn Conn, repo: &Repository) -> Result<()>` in `src/sync/`
(sibling to `serve`). It reuses the WS5 `Conn`, framing, and object-transfer machinery,
but **advertises only refs under `refs/collab/public/`** — no accessor, no other refs, no
labels consulted.

- **Anonymous read by construction.** Public collab objects are world-readable by WS8a
  design, so the endpoint requires no identity and performs no access check beyond "is
  this ref under the public prefix." Because the endpoint's entire surface *is* the public
  collab namespace, there is nothing else to leak.
- **Read-only.** The endpoint never accepts ref-update / push operations. Publishing is
  always a local operation (§4); a remote peer can only read.
- **Server command:** `bole node serve --listen <addr>` binds a TCP listener and hands
  each accepted connection to `serve_collab` (reusing the WS5 TCP accept path). Serves
  until interrupted.

**M2 is enforced here and only here:** the endpoint enumerates refs by the
`refs/collab/public/` prefix, so `refs/collab/scoped/` (or any non-public ref) can never
be advertised or transferred through it — independent of the label short-circuit in the
general sync path.

---

## 3. Discovery client — pull + remote-tracking storage

`collab_pull(conn: &mut dyn Conn, repo: &Repository) -> Result<Key>` (client side):

1. Fetch the peer's advertised public objects over the connection (WS5 transfer).
2. **Verify every signature** with WS8a's `verify_profile`/`verify_edge`; drop any object
   that fails (fail-closed).
3. Confirm all surviving objects are authored by a **single key** (serve-own-only ⇒ one
   identity). If a peer serves objects under mixed authorship, keep only those authored by
   the peer's advertised profile key and drop the rest.
4. Store each surviving object in the local repo, pinned under
   `refs/collab/remotes/<authorkey-fp>/…` (profile → `.../profile`, edges →
   `.../edge/<kind>/<tokey-fp>`), mirroring the public-prefix ref layout but per-peer.
5. Return the peer's `Key`.

Command: `bole discover pull <addr>` opens a `TcpConn::connect(addr)`, runs `collab_pull`,
and reports the peer key + object counts. The peer's `Profile.endpoints` are then present
in local tracking state for re-pulls.

`bole discover query <term> [--hops N]` builds the index from **local state only**:

- Read own `public_objects()` → distance-0 results.
- Read every tracked remote peer's objects (under `refs/collab/remotes/`).
- Build a `TrustGraph` from the combined edge set (own + tracked).
- Compute each authored object's distance via `follow_neighborhood(self_key, hops)`
  (default hops = 2, per WS8a).
- Assemble via WS8a `Index::build` and run `Index::query(term)`.

This reuses WS8a's `Index`/`TrustGraph` verbatim; no live `gather` is needed at query time
(live `gather` remains for WS8c's background poller). `--json` output carries key, resolved
petname, distance, and trust path per result.

---

## 4. CLI surface

A node has one collab identity: its `CollabSigner` seed, sourced from `$BOLE_COLLAB_KEY`
or `--key-file` (mirrors `BOLE_APPROVER_KEY`; the seed never appears on argv). The derived
public key is the node's canonical identity.

| Command | Behavior |
|---------|----------|
| `bole profile set --display-name <s> [--bio <s>] [--endpoint <addr>]…` | Author + publish a monotonic `Profile` (auto-increments `seq` from the current profile). |
| `bole trust follow <keyhex>` | Author + publish a `Follow` `TrustEdge`. |
| `bole trust vouch <keyhex> --name <petname>` | Author + publish a `Vouch` `TrustEdge`. |
| `bole node serve --listen <addr>` | Run the read-only collab-serve daemon. |
| `bole discover pull <addr>` | Pull a peer into `refs/collab/remotes/<key>/`. |
| `bole discover query <term> [--hops N] [--json]` | Local trust-ranked search. |
| `bole profile show [<keyhex>]` | Show own (default) or a tracked peer's profile. |
| `bole trust list` | List own + tracked trust edges with resolved petnames. |

All parse-critical output honors `--json` (the stable contract) and `--quiet`.

---

## 5. Security & hardening

- **M2 — scoped-ref gating:** enforced structurally by §2 (the endpoint enumerates only
  the public prefix). Tested by pinning an object under `refs/collab/scoped/` on the
  server and asserting it is never advertised or pulled over a real connection.
- **F4 — publish TOCTOU:** `publish_profile`/`publish_edge` currently read the current seq,
  check it, then write in separate steps — two concurrent publishes could both pass a
  stale check. Serialize the read-check-write under a mutex (mirroring
  `RefStore.commit_lock`) so monotonicity holds under concurrent publication. Tested with
  concurrent same-key publishes asserting exactly one wins and the survivor has the higher
  seq.
- **Pull-side fail-closed:** every pulled object is signature-verified before storage
  (§3.2); a peer serving a forged/tampered object has it dropped. Tested with a tampered
  profile served over a real `Conn`.
- **Anonymous-read boundary:** the endpoint serves only public objects, requires no
  identity, and never accepts writes. Publishing is always local and key-gated.

---

## 6. Testing

**Unit**

- `serve_collab` advertises only `refs/collab/public/**` (a scoped-pinned object never
  appears in its advertisement).
- `collab_pull` stores surviving objects under `refs/collab/remotes/<key>/` and drops
  unsigned/tampered objects; mixed-authorship objects beyond the peer key are dropped.
- Local-index distance computation: own objects at distance 0, a directly-followed tracked
  peer at distance 1.
- Monotonic publish under concurrency (F4): concurrent same-key publishes leave exactly
  one current profile with the higher seq.

**Integration (loopback TCP, real `TcpConn`)**

- Node A publishes a profile + a `Follow` edge to B; B publishes its own profile; B serves;
  A pulls B over loopback; A `query` finds B at distance 1.
- A scoped object pinned on B is never pulled.
- A tampered object served by B is rejected on pull.

**CLI E2E (drives the real `bole` binary, à la `bole-cli/tests/approvals.rs`)**

- Two temp repos; `bole node serve` one on a loopback port in the background; from the
  other, `bole discover pull <addr>` then `bole discover query --json`; assert the peer is
  discoverable, and that scoped/tampered content is absent.

---

## 7. Scope boundary (→ WS8c)

WS8b deliberately excludes:

- **Relays / aggregation** and any cache-and-forward re-serving of others' objects.
- **Depth-2 friend-of-follow auto-reach** (resolving a followee's followee's address).
- **Background / periodic polling** (the live `gather` poller).
- **Stranger-reach search UX.**
- **Real DNS alias verification** (the `AliasResolver` trait exists; wiring a live
  `.well-known`/TXT resolver is WS8c).
- **Any web UI.**

WS8b delivers networked publish → serve → direct pull → local index/query with trust
ranking over what you have pulled, plus the two security fixes — and hands depth-2 reach,
relays, and background replication to WS8c.
