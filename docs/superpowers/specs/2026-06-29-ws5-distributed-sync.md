# WS5 — The Distributed Story: Remote, Fetch, Push, Clone, and the Sync Protocol

- **Bead:** `bole-cy6`
- **Depends on:** `bole-fo2` (WS1 — access/policy core), `bole-81z` (WS4 — packs/idx/GC/RefTransaction)
- **Status:** design spec (not an implementation plan)
- **Conforms to:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md).
  Shared vocabulary (Label, LabelLattice, Clearance, Accessor, PolicyHook,
  content-addressed policy object, Pack/idx, atomic refs, Transfer = pack delta)
  is defined there and is not re-derived. This spec **owns** the foundations
  "Sync — the hard authority question" decision.
- **Builds on, does not re-derive:**
  - WS1 ([`2026-06-29-ws1-access-policy-core.md`](./2026-06-29-ws1-access-policy-core.md))
    fixed policy as content-addressed `Object::Policy { Lattice, RuleSet,
    Grant(signed), Root }` with a `PolicyRoot.parent` hash chain, named by a
    policy ref. WS5 specifies how those objects *transfer*, *verify*, and
    *reconcile* across replicas — it does not change their on-disk shape.
  - WS4 ([`2026-06-29-ws4-storage-packs-gc.md`](./2026-06-29-ws4-storage-packs-gc.md))
    fixed the `.pack`/`.idx` format (self-identifying frames, sorted fan-out id
    tables), `RefTransaction` (CAS + atomic multi-ref apply), the GC grace
    window, and §9 "WS5 anticipation". WS5 is the wire protocol that *consumes*
    that payload; it does not redesign the bytes.

This spec is the wire and the policy/authority/authn layer on top of WS4's
payload and WS1's objects. The protocol primitive it generalises is the existing
`copy_objects` / `copy_refs` in `src/repo/mod.rs`: a whole-store copy between two
in-process repositories. WS5 turns that O(all-objects) copy into a negotiated,
transport-agnostic, policy-aware, resumable delta.

---

## 1. Goal

Let multiple replicas of a bole repository collaborate over a network (or any
byte transport) while preserving every local invariant:

1. **`remote`** — name and manage peer endpoints (`add` / `list` / `remove`).
2. **`fetch`** — pull the missing object closure for a peer's refs into the
   local store and update **remote-tracking refs** (never local timelines).
3. **`push`** — send the missing closure for selected local timelines and
   **compare-and-swap** the peer's timeline heads, rejecting non-fast-forward
   updates unless the timeline's `TimelinePolicyHook` allows it.
4. **`clone`** — bootstrap an empty repo from a remote (objects + refs +
   policy root + trusted key set).
5. A **transport-agnostic protocol** so HTTP, SSH, or an in-process channel are
   interchangeable behind one trait.

And it makes the call the foundations doc delegated:

6. **The policy/label authority model** — how policy objects transfer, how a
   receiver verifies them against a trusted root, and how policy conflicts
   between replicas resolve (§5).
7. **Authn/authz between replicas** — who may push, and how an incoming
   connection maps to a bole **actor + clearance** so the existing WS1 write
   rules apply per path/timeline (§6).

**Non-goals (v1).** Thin packs / binary deltas (WS4 reserved `record_type 0x02`;
this spec uses whole-object packs only). Server-side merge (there is **none** —
§4.4). Partial clone / shallow history. A discovery/registry service. A
multi-master consensus protocol (CAS + fetch-merge-repush is the concurrency
story, §7). Deep CLI ergonomics (WS7 owns the verbs; §8 only notes the surface).

---

## 2. Architecture

Five layers. Each upper layer depends only on the trait below it, so a new
transport never touches negotiation logic and a negotiation change never touches
the wire encoding.

```
   ┌────────────────────────────────────────────────────────────────┐
   │ 5. PORCELAIN     remote add/list/remove · fetch · push · clone   │  (WS7 wires CLI)
   │    src/sync/porcelain.rs — drives the session, updates refs      │
   └───────────────────────────┬────────────────────────────────────┘
   ┌───────────────────────────▼────────────────────────────────────┐
   │ 4. SESSION       handshake · have/want · pack stream · ref CAS   │
   │    SyncSession<T: Transport> — the protocol state machine        │
   └───────────────────────────┬────────────────────────────────────┘
   ┌───────────────────────────▼────────────────────────────────────┐
   │ 3. AUTHORITY     policy transfer · signature-chain verification  │  (§5)
   │    PolicyVerifier · TrustStore — fail-closed                     │
   ├──────────────────────────────────────────────────────────────── ┤
   │ 3'. AUTHN/AUTHZ  connection → Actor → Accessor → WS1 write rules │  (§6)
   │    PeerIdentity · ActorMap                                        │
   └───────────────────────────┬────────────────────────────────────┘
   ┌───────────────────────────▼────────────────────────────────────┐
   │ 2. WIRE          frame codec: Message enum ⇄ bytes (postcard)    │  (§3)
   │    plus WS4 pack frames streamed verbatim (no re-encode)         │
   └───────────────────────────┬────────────────────────────────────┘
   ┌───────────────────────────▼────────────────────────────────────┐
   │ 1. TRANSPORT     trait Transport: framed request/response + body │  (§7.1)
   │    HttpTransport (v1) · InProcessTransport (tests) · SshTransport │
   └────────────────────────────────────────────────────────────────┘
```

Module layout (`src/sync/`, new; nothing existing is moved):

| Module | Responsibility |
|--------|----------------|
| `sync::transport` | `Transport` trait + `RequestCtx`; concrete `HttpTransport`, `InProcessTransport` |
| `sync::wire` | `Message` enum, postcard frame codec, capability/version constants |
| `sync::session` | `SyncSession` state machine: handshake → negotiate → transfer → ref-apply |
| `sync::negotiate` | have/want set-difference + missing-closure walk (reuses WS4 idx + WS1/WS4 edge set) |
| `sync::authority` | `TrustStore`, `PolicyVerifier`, policy reconciliation (§5) |
| `sync::authn` | `PeerIdentity`, `ActorMap`, accessor construction (§6) |
| `sync::remote` | `Remote` config record + `RemoteStore` (named endpoints) |
| `sync::server` | server-side request handler (the "receiving end" of every verb) |
| `sync::porcelain` | `fetch` / `push` / `clone` orchestration over a `SyncSession` |

`Repository` gains thin entry points (`fetch`, `push`, `clone_from`, `remotes()`)
that delegate to `sync::porcelain`. `copy_objects` / `copy_refs` are **retained**
(§9) and become the degenerate in-process transport's bulk path.

---

## 3. The wire protocol

### 3.1 Roles and framing

Every sync is between a **client** (initiates) and a **server** (responds);
fetch and push differ only in *which side streams the pack* and *which side
applies refs*. The transport (§7.1) provides ordered, reliable, framed
request/response with a streaming body — nothing more. On top of it the protocol
is a sequence of `Message` frames:

```rust
/// One control frame. Length-prefixed postcard on the wire. Pack bytes do NOT
/// travel as a Message variant — they are streamed as the transport body of a
/// PackStream phase so WS4 frames are forwarded verbatim with zero re-encode.
pub enum Message {
    Hello(Hello),                 // client → server: version + capabilities + intent
    Welcome(Welcome),             // server → client: agreed version + caps + server refs
    Auth(AuthToken),              // client → server: credential (§6); may be in Hello
    HaveWant(HaveWant),           // either direction: id sets for negotiation
    PolicyOffer(PolicyOffer),     // sender → receiver: policy root + chain to verify (§5)
    RefUpdate(Vec<RefOp>),        // push: requested CAS ref ops
    RefStatus(Vec<RefResult>),    // push result / fetch advertised heads
    PackStreaming,                // marker: the body that follows is a WS4 pack
    Done(Summary),                // end of session
    Error(WireError),            // typed failure (auth, policy, CAS, transport)
}
```

`ObjectId`, `RefName`, `Ref`, `PolicyRoot`, and all policy objects are reused
verbatim from WS1/WS4 (already `Serialize`/`postcard`), so the wire shares the
on-disk encoding. The frame codec is a 4-byte length prefix + a `u8` tag +
postcard body; this is `sync::wire`'s only responsibility.

### 3.2 Capability / version handshake

The first round trip pins the protocol so future versions negotiate rather than
misparse (mirrors WS4's versioned pack/idx headers):

```rust
pub struct Hello {
    pub proto_min: u16, pub proto_max: u16,   // client's supported range
    pub caps: CapSet,                         // bitset, see below
    pub intent: Intent,                       // Fetch | Push | Clone
    pub auth: Option<AuthToken>,              // optional inline credential (§6)
}
pub struct Welcome {
    pub proto: u16,                           // chosen = min(client.max, server.max), ≥ both mins
    pub caps: CapSet,                         // intersection the server will honour
    pub server_actor: Option<String>,        // who the server thinks it is (informational)
    pub refs: Vec<RefAdvert>,                 // advertised refs (name, target, kind) — fetch input
}
```

`CapSet` bits (v1): `MULTI_ACK` (incremental have/want rounds), `RESUME`
(checkpointed transfer, §7.3), `POLICY_V1` (policy verification chain, §5),
`SIGNED_REFS` (signed ref-update CAS, §6.4). Unknown caps are ignored; both sides
operate at the intersection. If the version ranges do not overlap the server
returns `Error(WireError::Version)` and closes. `proto` is fixed at `1` for v1.

### 3.3 have/want negotiation by ObjectId (the missing-graph closure)

The negotiation computes the exact set of objects the receiver lacks, leaning on
WS4's sorted `.idx` fan-out tables so neither side ever scans object *bodies*.

- **`have`** of a peer = the union of its loose ids and every pack's sorted
  `.idx` id table (WS4 §7). Because the tables are already in `ObjectId` order,
  membership tests and set-difference are merge-scans, not stats.
- **`want`** = the set of ref targets (timeline heads / tag targets) the
  receiver wants to obtain.

The closure is computed by the **sender of the pack** (the side that *has* the
objects), walking the object graph from each `want` head and pruning any subtree
whose root id the receiver already `have`s:

```rust
// Edge set is exactly WS4 §6.2 (GC mark) PLUS WS1's policy edges:
//   Snapshot → root (Tree) + parents             EnvOverlay → Secret ids
//   Tree     → every TreeEntry.id                 Blob/Secret → leaf
//   PolicyRoot → lattice, rules, parent(Root)     (WS1 §3.4)
fn missing_closure(store, wants: &[ObjectId], have: &IdSet) -> Result<Vec<ObjectId>> {
    let mut out = Vec::new(); let mut seen = HashSet::new(); let mut stack = wants.to_vec();
    while let Some(id) = stack.pop() {
        if have.contains(&id) || !seen.insert(id) { continue; }  // prune: peer has it ⇒ has its closure
        out.push(id);
        stack.extend(child_edges(store.get(&id)?));              // WS4 §6.2 + WS1 policy edges
    }
    out  // a valid topological-ish set; pack frames are self-contained so order is free
}
```

**The pruning invariant** (why a single `have` set suffices, no per-object
back-and-forth): every object is the BLAKE3 of its content, and an object's
content embeds the ids of its children. Therefore *if a peer has object X it has
X's entire reachable closure* — X could not have been stored otherwise (WS4
objects-before-refs ordering + self-verifying receive). So encountering a
`have` id lets the sender prune the whole subtree. This makes one `have` exchange
sufficient for correctness; `MULTI_ACK` rounds are a bandwidth optimisation
(send `have` in tranches when the head set is huge), never a correctness need.

**Negotiation round trips (the common, capability-minimal path):**

```
fetch:
  C → S : Hello{ Fetch, caps, auth? }
  S → C : Welcome{ proto, caps, refs=[advertised heads] }
  C → S : HaveWant{ want = chosen server heads, have = local id set (or bloom, §3.5) }
  S → C : PolicyOffer{ root chain for any policy refs in want }       (if POLICY_V1)
  S → C : PackStreaming + <WS4 pack body of missing closure>
  S → C : RefStatus[ advertised heads ]  ;  Done{summary}
  C     : verify pack frames, land pack, verify policy (§5), update remote-tracking refs (§4.1)

push:
  C → S : Hello{ Push, caps, auth }                                   (auth REQUIRED, §6)
  S → C : Welcome{ proto, caps, refs=[server heads for the targets] } (its 'have' advertisement)
  C → S : HaveWant{ want = (none; client sends) , have = server heads echoed }  → client computes closure
  C → S : PolicyOffer{ ... } (if client advances a policy ref)
  C → S : PackStreaming + <WS4 pack body of missing closure>
  C → S : RefUpdate[ CAS ops: (name, expected_old, new_head) ]        (§4.2)
  S     : authz each op (§6), verify pack, land pack, RefTransaction (§4.2)
  S → C : RefStatus[ per-ref Ok | NonFastForward{server_head} | Denied | PolicyDenied ]
```

Note the asymmetry: in **fetch** the *server* computes the closure from the
client's `have`; in **push** the *client* computes it from the server's
advertised heads (which the server sent in `Welcome`, serving as its `have`
summary for the targeted refs). The server may additionally send a fuller `have`
set if the client requests it (for an efficient first push into a non-empty
server). Either way, exactly one side walks the graph and exactly one pack
streams.

### 3.4 Streaming the pack & self-verifying receipt

The pack body is **WS4 frames forwarded verbatim** — the sender does not build an
`.idx` to produce the stream, the receiver does not need one to consume it (WS4
§3.2/§9). The receiver:

1. Reads each frame: `record_type`, `object_id`, `uncompressed_len`,
   `stored_len`, `zstd_frame`.
2. zstd-decodes, asserts `len == uncompressed_len` and
   `BLAKE3(bytes) == object_id`, postcard-decodes to an `Object`. Any mismatch ⇒
   `Error(WireError::CorruptFrame)`, abort, keep nothing referenced.
3. Lands frames as a **received pack**: writes `packs/.pack-<tmp>`, builds the
   sorted `.idx`, verifies the trailer `pack_digest` + `end_magic`, then renames
   into `packs/` (WS4 §5.1 crash-safe sequence). Objects-before-refs: the pack is
   durable **before** any ref CAS is attempted.
4. Only after the pack is durable does the session apply refs (§4). A transfer
   that dies mid-stream leaves an ignored tmp pack (no `.idx` → invisible to
   `PackSet`) — i.e. a collectible orphan, never corruption (WS4 invariant 4).

This is the same self-verifying property WS4 designed; WS5 adds nothing to the
frame format, only the surrounding control messages.

### 3.5 `have` set size — exact ids vs Bloom filter

For small/medium repos the client sends exact ids (it already mmaps them, free).
For very large `have` sets a `BLOOM_HAVE` capability lets the receiver send a
Bloom filter of its ids instead; the sender prunes against the filter and the
content-addressed self-verifying receive makes a false-negative (re-send of an
already-present object) harmless (idempotent `put`) and a false-positive
impossible to mis-handle (a wrongly-pruned object would surface as a missing-base
error → fall back to a corrective exact-id round). v1 ships exact ids; Bloom is
behind a capability bit (Open question O6).

---

## 4. Ref synchronization

Objects are immutable and merge by union; **refs are the only thing that can
conflict**, and they conflict exactly the way WS4's `RefTransaction` CAS was
built to handle.

### 4.1 Fetch updates remote-tracking refs only

Fetch **never** moves a local timeline. It writes **remote-tracking refs** under
a reserved namespace, mirroring the remote's advertised heads:

```
refs/remotes/<remote-name>/<their-ref-name>     # e.g. refs/remotes/origin/main
```

After a successful fetch the session opens a single WS4 `RefTransaction` and sets
each `refs/remotes/<remote>/…` to the advertised target (plain `set`, no CAS —
these mirror the remote and are owned solely by fetch). Local timelines and tags
are untouched; the user (or WS7 porcelain) decides whether/how to integrate
(merge, fast-forward) using the existing `merge_timelines` / `advance_timeline`.
This is the standard "fetch is safe, merge is a separate explicit act" split.

`RefName` already supports `/`-segmented names (`src/refs/name.rs`), so
`refs/remotes/...` needs no new ref machinery — only a reserved-prefix
convention and a `RemoteStore` to record endpoints (§7.4).

### 4.2 Push is compare-and-swap on timeline heads

Each pushed timeline becomes a CAS op using WS4's
`RefTransaction::advance_head_if`:

```rust
pub struct RefOp {
    pub name: RefName,
    pub expected_old: Option<ObjectId>,   // None ⇒ create (expect absent)
    pub new_head: ObjectId,
    pub signature: Option<Vec<u8>>,       // §6.4, if SIGNED_REFS
}
```

Server-side handling of `RefUpdate(ops)`, after the pack is durable:

1. **Authorize** each op against the connection's `Accessor` (§6): the actor
   must hold `can_write_timeline(name)` and, for every path whose effective
   label changed in `new_head`'s tree vs `expected_old`, `can_write_path` under
   the dominance rule (WS1 §4.2). A failing op ⇒ `RefResult::Denied`.
2. **Fast-forward / policy gate.** For each op, evaluate the timeline's
   `PolicyHook`s for the `Advance { old_head, new_head }` event (WS1 §5.2). The
   built-in `TimelinePolicyHook` rejects a non-fast-forward update for
   `FastForwardOnly`/`Append` timelines (`old_head` must be an ancestor of
   `new_head`); `Unrestricted` allows it. A hook `Deny` ⇒
   `RefResult::PolicyDenied(reason)`; `RequiresApproval` ⇒
   `RefResult::ApprovalRequired`. **There is no server-side override** — a
   non-fast-forward push is rejected unless policy explicitly permits it. This is
   exactly the local `advance_timeline` rule (WS1 §6) applied to a remote actor.
3. **CAS apply.** All surviving ops go into **one** `RefTransaction`, each as
   `advance_head_if(name, expected_old, new_head)` (or `create` with
   `expect(absent)`). The transaction's precondition validation (WS4 §8.1)
   re-reads current heads under the `refs/.txn/lock`; if any `expected_old` no
   longer matches (a concurrent pusher won the race), the **whole transaction
   aborts** and every op reports `RefResult::NonFastForward { server_head }`.
   Atomicity is WS4's: all ops or none.
4. Return `RefStatus(results)`.

Because the pack landed *before* the CAS, a lost CAS race leaves the pushed
objects as harmless orphans (collectible by GC), and a retry after re-fetch
re-uses them (idempotent).

### 4.3 How divergence is reported

When `expected_old` ≠ the server's current head, the op result is
`NonFastForward { server_head }`. The server reports its *actual* current head so
the client can fetch precisely that. The porcelain surfaces this as the familiar
"updates were rejected; the remote contains work you do not have locally — fetch
and integrate, then push again."

### 4.4 Client reconciles; there is NO server-side merge

The server is a dumb, safe ref-CAS-and-pack-sink. It never merges. On
`NonFastForward` the client:

1. **Fetches** the remote head (`refs/remotes/<remote>/<name>` now points at
   `server_head` — and the objects arrived in step-2's advertisement/closure, or
   a follow-up fetch brings them).
2. **Merges locally** using the existing `Repository::merge_timelines` /
   `check_merge` (WS1-aware: confidentiality leak scan + `PolicyHook`s run on the
   local merge exactly as today).
3. **Re-pushes** with the new `expected_old = server_head`.

This keeps all merge policy, conflict resolution, and approval logic on the
client where the `Accessor`, lattice, and hooks live — the server need not (and
in a least-trust deployment, must not) evaluate merges. It also means a
read-only or policy-restricted server is a perfectly valid sync hub.

---

## 5. The policy / label authority model (the foundations decision)

**Question (foundations §"Sync"):** when replicas disagree about policy
(lattice, rules, clearances, hooks), who is the source of truth?

**Decision: (b) — signed, content-addressed policy objects with a verification
chain, "highest-rooted-wins" by the `PolicyRoot.parent` hash chain anchored in a
per-repo trusted key set.** Rejecting (a) and (c):

- **(a) designated authority replica** — rejected as the *model*, kept as a
  *deployment convention*. Pinning truth to one replica re-introduces a single
  point of failure and a not-content-addressed source of truth, contradicting
  foundations decision that policy is transferable/verifiable "regardless of
  which authority model wins." (You can still *operate* model (b) with one
  blessed publisher; that is policy, not protocol.)
- **(c) LWW + audit** — rejected: last-writer-wins on *security* state is a
  footgun (a stale or hostile clock silently downgrades the lattice / widens a
  clearance), and "audit after the fact" cannot un-leak a secret. WS1 already
  invested in signed `Grant`s and a hash chain precisely so we need not trust
  wall-clock ordering.
- **(b) chosen** — it *is* WS1's design finished. WS1 made every policy piece a
  content-addressed `PolicyObject`, `Grant` carries a `signature`, and
  `PolicyRoot.parent` already forms an audit/lineage chain. WS5 supplies the
  missing verb: **verify the chain to a trusted root and prefer the longest
  verified chain.** No new on-disk types; we fill WS1's reserved
  `signature` field semantics and add a `TrustStore`.

### 5.1 What transfers and how

Policy objects are ordinary `Object::Policy` variants, so they ride the **same
pack mechanism** as everything else (no special transfer path). The policy ref
(`refs/policy/current` → a `PolicyRoot` id, WS1 §3.4) is just another ref in
negotiation. The closure walk (§3.3) follows the WS1 policy edges
(`PolicyRoot → lattice, rules, parent`), so fetching the policy ref pulls the
lattice, rule set, grants, **and the entire parent chain** back to a root the
receiver already has (pruned by `have`). Grants are reachable from the
`PolicyRoot` (or advertised as their own refs per-actor — Open question O3) so
they transfer with it.

A push or fetch that moves a policy ref carries a `PolicyOffer`:

```rust
pub struct PolicyOffer {
    pub policy_ref: RefName,               // e.g. refs/policy/current
    pub new_root: ObjectId,                // PolicyRoot being offered
    pub chain_tip_known: Option<ObjectId>, // deepest parent the offerer believes receiver has
}
```

### 5.2 How a receiver verifies policy

A receiver **never** adopts a policy root just because it arrived. It runs
`PolicyVerifier` against its local `TrustStore` before the policy ref is allowed
to move:

```rust
pub struct TrustStore { pub roots: Vec<TrustAnchor> }   // per-repo trusted signing keys
pub struct TrustAnchor { pub key_id: String, pub public_key: Vec<u8>, pub algo: SigAlgo }

pub enum PolicyVerdict {
    Accept { depth: u64 },                 // verified chain, length = depth (tie-break metric)
    Reject(String),                        // unsigned/forged/untrusted/unknown-hook
}
```

Verification of an offered `PolicyRoot R`:

1. **Walk the chain** `R → R.parent → … ` to a root `R0`. Every step's
   `PolicyRoot` and its `lattice`/`rules` objects must be present (fetched) and
   each object id must equal `BLAKE3(content)` — content-addressing already
   guarantees structural integrity.
2. **Signature anchoring.** Each `PolicyRoot` (and/or the `Grant`s it admits)
   must carry a signature (WS1's reserved `signature`) that verifies under a key
   in the `TrustStore`. A root `R0` is *trusted* iff it is signed by a
   `TrustAnchor`. A non-root `Rn` is trusted iff it is signed by a key the
   chain's policy itself authorises (a clearance/role for "policy-admin") OR by a
   `TrustAnchor`. (The simplest v1 rule: every `PolicyRoot` in the chain must be
   signed by some `TrustAnchor`; delegated policy-admin keys are Open question
   O2.)
3. **Hook resolvability.** Every `HookSpec.kind` named in the root must resolve
   to a compiled hook in this replica's registry; an **unknown hook kind ⇒
   `Reject`** (fail-closed, per WS1 O5). A replica cannot enforce a rule it
   cannot run, so it must refuse the policy rather than silently drop the rule.
4. On `Reject`, the policy ref update is refused (`Error(WireError::Policy)`);
   the rest of the object pack is harmless (orphans). On `Accept`, proceed to
   conflict resolution.

### 5.3 How policy conflicts resolve — highest-rooted-wins

Two replicas may each have advanced policy. Resolution is deterministic and
needs no clock:

1. Both candidate roots must verify (§5.2) **and share a common ancestor** in
   the parent chain (same `TrustAnchor`-signed root `R0`). If they do not share a
   trusted root, it is a genuine fork → refuse, surface to the maintainer
   (Open question O5).
2. The **longer verified chain wins** (greater `depth` from the shared `R0`); it
   is by construction a descendant or a strictly-further-evolved lineage. The
   shorter chain's tip must be an ancestor of the winner (fast-forward of
   policy), exactly mirroring the timeline fast-forward rule.
3. If the two verified chains have **equal depth but different tips** (a true
   policy branch — both signed, both rooted, neither an ancestor of the other),
   that is a **policy divergence**: refuse to auto-resolve, report both tips, and
   require an authorised policy-admin to issue a new `PolicyRoot` whose `parent`
   is one of them (merging the lattices/rules) — i.e. policy reconciles the same
   way data does: explicit, client-side, signed. **No silent merge of security
   state.**

This makes policy propagation monotone and tamper-evident: a replica only ever
moves to a *verified, strictly-longer* policy lineage, and any unsignable or
unrooted policy is inert.

### 5.4 Bootstrapping trust (clone)

Trust has to start somewhere. On `clone` the client receives the remote's
`PolicyRoot` chain and its advertised `TrustStore` anchors, but **adopting the
anchors is a trust-on-first-use (TOFU) decision the user must confirm** (or pin
out-of-band via a fingerprint passed to `bole clone --policy-key <fpr>`). After
clone the anchors are local; subsequent fetches verify against them. This is the
same TOFU posture as SSH host keys (§6) and is the one unavoidable human step.

---

## 6. Authn/authz between replicas

The server must answer two questions for every push: **who is this connection**
(authentication) and **what may they write** (authorization, via the *existing*
WS1 rules — WS5 adds no new permission engine).

### 6.1 Trust model — transport auth maps to a bole actor

```
transport credential  ──ActorMap──►  Actor (key-id / name)  ──Grant──►  ClearanceSet  ──►  Accessor
   (SSH key / token)                  (a WS1 actor identity)   (WS1 §3.4)        (WS1 §4.3)
```

1. **Authentication** happens at/just-above the transport: the client presents a
   credential the transport can bind to a stable principal —
   - **SSH transport:** the client's SSH public key (the server's
     `authorized_keys` equivalent is a bole `ActorMap`).
   - **HTTP transport (v1):** a bearer **token** (or mTLS client cert) carried in
     `Hello.auth` / an `Authorization` header.
   The transport surfaces the verified principal as a `PeerIdentity`:
   ```rust
   pub struct PeerIdentity { pub principal: Principal, pub method: AuthMethod }
   pub enum Principal { SshKey(KeyId), Token(TokenId), Mtls(CertSubject), Anonymous }
   ```
2. **Mapping to a bole actor.** `ActorMap` (a small content-addressed or
   on-disk-config table on the server) maps `Principal → actor-name`. An
   unmapped principal is `Anonymous`.
3. **Actor → clearance.** The server loads that actor's `ClearanceGrant` (WS1
   §3.4) from its active policy and constructs an `Accessor`
   (`lattice + rules + clearances`, WS1 §4.3). This `Accessor` is what every
   push op is checked against — **identical to the local `advance_timeline`
   path**, so remote and local writes obey one rule set.

### 6.2 Authorization is just WS1, applied server-side

There is no second authz model. Server-side push authorization = run the same
`Accessor`/`PolicyHook` checks `advance_timeline` already runs (WS1 §6):

- `can_write_timeline(name)` — actor's Write clearance dominates the timeline's
  effective label.
- per-path `can_write_path` for paths whose label changed.
- `TimelinePolicyHook` + any bound hooks for the `Advance` event.

A reader (fetch) is checked with the same `Accessor`: the server only advertises
and packs objects reachable from refs the actor `can_read` (filtering protected
timelines via `list_refs_filtered`, and — Open question O4 — optionally
label-filtering object trees the actor cannot read; v1 gates at the
ref/timeline granularity, matching today's `list_refs_filtered`).

### 6.3 Anonymous and read-only deployments

A server may map `Anonymous → a public-clearance actor` for open read-only
mirrors (fetch/clone of public timelines, no push). Push from `Anonymous` is
denied by the ordinary write rule (no Write clearance). This needs no special
case — it falls out of WS1 evaluation.

### 6.4 Signed ref updates (optional hardening)

With `SIGNED_REFS`, each `RefOp` carries a signature over
`(name, expected_old, new_head)` by the actor's key. The server verifies it
against the actor's key before the CAS. This defends against a compromised
transport/token replaying or forging a head move, and ties the *ref change*
(not just the connection) to a key — the same key material the policy chain uses.
v1 ships connection-level auth as mandatory and `SIGNED_REFS` as a capability
(recommended-on for `release/**`-class timelines via a `PolicyHook` that *demands*
a valid signature — this is exactly WS1's §5.4 worked example generalised).

---

## 7. Transport trait + the v1 concrete transport

### 7.1 The trait

The protocol (§3–§4) is written entirely against this trait; it knows nothing of
sockets:

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    /// Open a logical sync connection to the configured endpoint. Returns a
    /// duplex framed channel: control Messages both ways + a streamed body for
    /// the pack phase. Carries the auth credential so the server can build a
    /// PeerIdentity (§6).
    async fn connect(&self, intent: Intent, auth: Option<AuthToken>) -> Result<Box<dyn Conn>>;
}

#[async_trait]
pub trait Conn: Send {
    async fn send_msg(&mut self, m: &Message) -> Result<()>;
    async fn recv_msg(&mut self) -> Result<Message>;
    /// Stream pack bytes (WS4 frames) without buffering the whole pack.
    async fn send_pack(&mut self, frames: impl AsyncRead) -> Result<()>;
    async fn recv_pack(&mut self) -> Result<Box<dyn AsyncRead>>;
    /// Byte offset already durably received, for RESUME (§7.3).
    fn bytes_received(&self) -> u64;
}
```

The server side is symmetric: a `serve(conn, repo)` handler in `sync::server`
reads `Hello`, runs the same state machine in the responder role.

### 7.2 v1 concrete transport — **HTTP** (recommended), with SSH as the second

**Choice: HTTP(S) for v1.** Justification vs a git-style smart-protocol-over-SSH:

- **It is genuinely transport-agnostic-friendly.** The protocol is already a
  framed request/response with a streamed body; that maps onto HTTP cleanly:
  - `POST /bole/v1/info` → `Hello`/`Welcome` (capability + ref advertisement).
  - `POST /bole/v1/fetch` body = `HaveWant`; response body = `PolicyOffer` +
    streamed pack + `RefStatus` (chunked/streamed).
  - `POST /bole/v1/push` body = streamed pack + `RefUpdate`; response =
    `RefStatus`.
  Each endpoint is one request with a streaming body — no long-lived custom
  socket protocol to implement first.
- **Auth is mature and pluggable** — bearer tokens, mTLS, or a reverse proxy
  doing OIDC, all without bole owning a key-exchange. The `PeerIdentity` (§6)
  comes straight from the request's auth context.
- **Resumability is free-ish** — HTTP Range / a resume token (§7.3) rides native
  HTTP semantics; proxies, CDNs, and load balancers all understand it.
- **Operability** — runs behind standard infra (TLS termination, WAFs, logging),
  which a bespoke SSH-channel protocol does not get for free.

**SSH is the designed-in second transport**, not an afterthought: it implements
the same `Transport` trait by running `bole sync-serve --stdio` over an SSH exec
channel (git's model). It is preferable where users already manage SSH keys and
want principal = SSH key with zero token infrastructure. Because both sit behind
`Transport`, the session/negotiation/authority code is identical; only
`connect` + the principal extraction differ. `InProcessTransport` (a pair of
in-memory channels) is the third impl and the backbone of testing (§10).

### 7.3 Resumability

A transfer interrupted mid-pack is **resumed by re-negotiation, not byte
replay** — content-addressing makes this nearly free:

- On reconnect the client re-runs have/want. Objects from the dead transfer that
  *did* land durably (a partially-received tmp pack is discarded, but any
  fully-landed prior pack persists) are now in the client's `have` set, so the
  sender simply doesn't resend them. Worst case re-sends the in-flight pack;
  best case (with `RESUME`) the server checkpoints object boundaries and a
  resume token `(pack_digest_prefix, last_complete_object_id)` lets it skip
  already-acked frames.
- Because each frame self-verifies and `put` is idempotent (WS4), a resumed or
  doubly-sent object is harmless. There is no corrupt partial state to recover
  from — only orphans to GC.

### 7.4 Remote configuration

```rust
pub struct Remote {
    pub name: String,            // "origin"
    pub url: String,             // https://host/path  |  ssh://user@host/path
    pub transport: TransportKind,// inferred from scheme; overridable
    pub default_fetch: Vec<RefSpec>,  // e.g. refs/heads/* : refs/remotes/origin/*
    pub auth_ref: Option<String>,     // name of a stored credential (token/key)
}
pub trait RemoteStore { fn add; fn get; fn list; fn remove; }   // on-disk under <repo>/remotes/
```

`remote add/list/remove` are pure local config edits (no network), stored beside
refs. Credentials are referenced by name, not embedded, so config is shareable.

### 7.5 GC interaction with in-flight fetches (the grace window)

A fetch/push from a peer races the server's GC (WS4 §6). WS4's **grace window**
(default 2 h) is the primary guard, and WS5 adds two protocol-level guarantees:

1. **Send order respects the closure.** The server walks the closure from refs it
   currently holds; an object it decides to send is, at decision time,
   reachable. GC's mark uses the same ref roots under the `refs/.txn/lock`
   (WS4 §6.5), so a concurrent GC and a fetch see a consistent ref snapshot.
2. **Just-received objects are grace-protected.** On the receiver, landed pack
   objects are newer than `now − grace`, so a receiver-side GC between "pack
   landed" and "refs committed" cannot collect them (WS4 §6.4) — the same race
   the local write path already handles, reused verbatim.
3. **Long transfers vs short grace.** A transfer longer than the grace window
   could in principle see a sender GC retire a pack mid-stream. Because packs are
   immutable and only *unlinked* after a replacement is durable (WS4 §4.3/§5.1),
   an in-flight `mmap`/read of an old pack stays valid until the read completes
   (POSIX unlinked-but-open semantics); the sender finishes streaming from the
   pack it started on. For transports without that guarantee, the server may take
   a read-lease on the packs it is streaming (Open question O7).

---

## 8. CLI surface (high level — WS7 owns ergonomics)

The model implies these verb families; flags/UX are **WS7**:

- `bole remote add <name> <url>` · `bole remote list` · `bole remote remove <name>`
  — local config only (§7.4).
- `bole fetch [<remote>] [<refspec>…]` — pull closure, update
  `refs/remotes/<remote>/*`, never touch local timelines (§4.1).
- `bole push [<remote>] [<refspec>…]` — CAS the remote heads; report
  per-ref Ok / non-fast-forward / denied (§4.2–4.3). `--force` is **not** a
  server override; it only relaxes the *client's* local-safety checks and still
  obeys the remote's `TimelinePolicyHook`.
- `bole clone <url> [<dir>] [--policy-key <fpr>]` — bootstrap (§6.4, §9 below).
- (later) `bole policy verify` / `bole trust add <key>` — inspect/seed the
  `TrustStore` (§5).

These exist so WS7 has a target; their exact spelling is out of scope here.

---

## 9. Clone (bootstrap a new repo)

`clone` is `fetch` against an empty local repo plus three bootstrap steps:

1. **Init** an empty `Repository::disk(dir)` (default `PolicyRoot` not yet
   installed — clone replaces it).
2. **Handshake + advertise.** `Hello{ Clone }` → `Welcome` advertising all refs
   the connecting actor may read (server-side `list_refs_filtered`, §6.2).
3. **Negotiate with empty `have`.** The whole reachable closure of the
   advertised heads (including the **policy ref chain** and grants) streams as
   one pack; self-verifying receive lands it (§3.4).
4. **Adopt policy under TOFU.** Receive the remote's `TrustStore` anchors;
   verify the offered `PolicyRoot` chain to a root signed by an anchor; with user
   confirmation (or `--policy-key` pin) install the anchors locally and set
   `refs/policy/current` (§5.4). Without confirmation, clone halts with the
   fingerprint for the user to verify out-of-band.
5. **Set refs.** In one `RefTransaction`: create `refs/remotes/origin/*` from the
   advertisement, create local timelines for the default branch(es), set the
   policy ref, and record the `origin` `Remote` (§7.4).

A clone is thus just the maximal fetch + the one-time trust/remote setup; it
shares all of fetch's verification and crash-safety.

---

## 10. Backward compatibility, testing, and open questions

### 10.1 Backward compatibility

- **Single-node repos are completely unaffected.** `src/sync/*` is additive; a
  repo that never calls `fetch`/`push`/`clone` behaves exactly as today. No
  existing type changes. The 247 tests are untouched.
- **`copy_objects` / `copy_refs` are retained.** They remain the public
  whole-store copy and become the bulk path of `InProcessTransport`: in-process
  sync between two `Repository` handles can either negotiate (to exercise the
  protocol in tests) or short-circuit to `copy_objects` + a `RefTransaction` of
  `copy_refs`. `copy_to` is kept; it is now documented as "unfiltered local
  clone — bypasses negotiation and policy verification; for trusted in-process
  use only." The general path *subsumes* it (a negotiated full transfer with
  empty `have` produces the same object set) but does not delete it.
- **WS4 / WS1 surfaces are consumed, not modified.** WS5 calls
  `RefTransaction`, `PackedDiskBackend`, the closure edge set, `Accessor`,
  `PolicyHook`, and the policy objects exactly as those specs define them.
- **Versioned wire.** `proto`/`CapSet` mean a future protocol bump negotiates or
  cleanly refuses, never misparses — same discipline as WS4's format versions.

### 10.2 Testing strategy

- **Two in-process repos over `InProcessTransport`.** The backbone: create two
  `Repository::memory()` handles, populate one, run a full `fetch`/`push`/`clone`
  through the real `SyncSession`, assert the other ends with the identical
  reachable object set and the expected refs. Differential against
  `copy_objects` (negotiated transfer ≡ whole-store copy for the empty-`have`
  case).
- **have/want minimality.** Assert the streamed pack contains *exactly* the
  missing closure (no object the receiver already had); assert the pruning
  invariant (a shared subtree is sent once); property test over random snapshot
  DAGs with shared subtrees and overlay→secret edges.
- **Self-verifying receive.** Corrupt a frame byte / truncate the stream / wrong
  `end_magic` / wrong `pack_digest` ⇒ receive aborts, no ref moves, only orphans
  remain (reuses WS4's streaming-decode tests through the wire layer).
- **CAS rejection (the core concurrency test).** Two clients push the same
  timeline concurrently against one server; assert exactly one `Ok`, the other
  `NonFastForward { server_head }`, server head equals the winner, loser's
  objects are orphans; then loser fetch+merge+re-push succeeds. Assert
  non-fast-forward is rejected for `FastForwardOnly` and allowed for
  `Unrestricted`.
- **Policy verification failure.** Offer a `PolicyRoot` (a) unsigned, (b) signed
  by an untrusted key, (c) naming an unknown hook kind, (d) with a broken parent
  chain ⇒ each ⇒ `Reject`, policy ref not moved, error surfaced. Then a valid
  longer chain ⇒ `Accept` and fast-forwards the policy ref; equal-depth divergent
  tips ⇒ refuse + report both.
- **Authn/authz.** Map a principal to an actor with only `public` read ⇒ clone
  sees only public timelines; map to a Write-cleared actor ⇒ push succeeds; push
  to a label/path the actor cannot write ⇒ `Denied`; anonymous push ⇒ denied.
  `SIGNED_REFS`: a forged ref signature ⇒ rejected.
- **Resumability.** Kill a transfer mid-pack; reconnect; assert it completes,
  re-sends only what was not durably landed, and the final state matches an
  uninterrupted run.
- **GC interaction.** Run server GC concurrently with a fetch; assert grace
  window protects in-flight objects and the fetch completes; assert an unlinked
  old pack streams to completion for a reader that started before retirement.
- **HTTP transport conformance.** Run the full `InProcessTransport` test matrix
  against `HttpTransport` over a loopback server to prove transport-agnosticism.

### 10.3 Open questions (maintainer's call — genuine forks)

- **O1 — HTTP vs SSH as the *mandatory* v1 transport.** Spec recommends HTTP
  first with SSH designed-in. If the user base is SSH-key-native and token infra
  is unwanted, ship SSH first instead. Both are behind `Transport`; the question
  is only *which one we build and harden first*. *Recommendation: HTTP first.*
- **O2 — Delegated policy-admin keys.** §5.2 v1 rule: every `PolicyRoot` in the
  chain must be signed directly by a `TrustAnchor`. Do we instead let the policy
  itself grant a "policy-admin" clearance whose key may sign descendant roots
  (true delegation), and if so does revocation of that key invalidate the
  sub-chain retroactively? *Recommendation: direct-anchor signing in v1;
  delegation after the signing story is exercised.*
- **O3 — Grant distribution granularity.** Are `ClearanceGrant`s reachable
  *from* the `PolicyRoot` (transfer with policy, simplest) or advertised as
  their own per-actor refs (`refs/policy/grants/<actor>`) so a replica can fetch
  only the grants it needs? *Recommendation: reachable-from-root in v1.*
- **O4 — Object-level vs ref-level read filtering on the server.** v1 gates
  reads at timeline granularity (`list_refs_filtered`). Do we also withhold
  individual blobs/subtrees an actor cannot read within an otherwise-readable
  timeline (true label-filtered packs), accepting that this breaks the "send the
  whole closure" simplicity and complicates have/want? *Recommendation:
  ref-granularity v1; label-filtered packs is a WS-follow-up.*
- **O5 — Genuinely forked policy with no shared trusted root.** When two
  replicas present verified chains rooted in *different* `TrustAnchor`s, there is
  no automatic resolution. Refuse and surface (recommended), or support an
  explicit `bole policy adopt <other-root>` that re-anchors? *Recommendation:
  refuse + manual re-anchor.*
- **O6 — `have` encoding for huge repos.** Ship exact-id sets in v1 and add a
  `BLOOM_HAVE` capability later, or build the Bloom path now? *Recommendation:
  exact ids v1, Bloom behind a cap.*
- **O7 — Pack read-lease vs grace window for long transfers.** Is POSIX
  unlinked-but-open enough (relying on the OS), or does the server need an
  explicit lease on packs it is streaming so GC defers their unlink? Matters more
  for non-POSIX object-store backends. *Recommendation: rely on grace +
  unlinked-open for v1; add leases if a backend needs them.*
- **O8 — Push of tags and policy refs.** Tags are immutable targets (move = CAS
  like a head?) and policy refs follow the §5 chain rule, not the timeline FF
  rule. Confirm the per-ref-kind CAS semantics (timeline = FF/policy-gated, tag =
  create-only or move-with-CAS, policy = longest-verified-chain). *Recommendation:
  timeline as specified; tags create-only by default; policy via §5.3.*
- **O9 — Server statelessness vs sessions.** The HTTP mapping (§7.2) is
  request-per-phase (stateless-friendly); the negotiation conceptually wants
  session state (the agreed caps, the computed closure). Do we carry a signed,
  short-lived session token across the info/fetch/push requests, or fold the
  whole exchange into one streaming request each? *Recommendation: one streaming
  request per verb (fetch is one request, push is one request); info is a cheap
  separate GET.*
