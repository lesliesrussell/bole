<!-- bole-0wr -->
# bole Threat Model

> **Status: experimental and NOT independently audited.** bole's cryptography and
> access-control enforcement have had an internal adversarial review (see
> [Audit history](#audit-history)) but **no external security audit**. Do not use
> bole to protect secrets or enforce access against a motivated adversary yet, and
> do not expose a bole network listener on an untrusted network (see
> [Deployment guidance](#deployment-guidance)).

This document states what bole is designed to defend, against whom, and — just as
importantly — what it does **not** defend today. A stated adversary is what makes
"secure" a testable claim; without it the word is meaningless.

## What bole is

bole puts access control *inside* a content-addressed version-control repository:
named actors carry labeled grants over a bounded label lattice; secrets are
envelope-encrypted objects in the same store; timeline advancement and merges are
gated by content-addressed, signed policy. See [README](../README.md) and
[docs/API.md](API.md).

## Assets

1. **Confidentiality of labeled content** — a path/timeline/secret an actor is not
   cleared for must not be readable by that actor (including via filtered views,
   projections, and sync).
2. **Integrity of history** — timelines advance and merge only as policy allows;
   no actor writes or deletes content it lacks write clearance for.
3. **Authenticity of governance** — policy roots and approvals are what a trusted
   key actually signed; they cannot be forged or replayed into a context the
   signer did not intend.
4. **Availability** — a single request must not exhaust CPU, memory, or disk.
5. **Confidentiality of key material** — master keys / data keys are not written
   to the repository and are handled through a narrow wrap/unwrap boundary.

## Adversaries in scope

- **A low-privilege local actor** driving the library/CLI: an actor (human or
  agent) with *some* grants trying to exceed them — read what it cannot read,
  write/advance/merge against policy, or delete protected content.
- **A malicious or compromised sync peer** speaking the wire protocol: trying to
  read objects it is not cleared for, push refs it is not authorized to move,
  forge/replay signatures, or crash/exhaust a node with hostile input.

The primary design intent remains **mistakes and least-privilege among
semi-trusted participants** (people and agents who are part of the system but must
be confined), hardened where practical against the malicious cases above.

## Out of scope (today)

- **External cryptographic audit.** Not done. The primitives are standard
  (ChaCha20-Poly1305, Ed25519 via `ed25519-dalek`, BLAKE3, `rand` CSPRNG), but the
  *constructions and composition* have not had third-party review.
- **A confined in-process adversary reading process memory / core dumps.** Key
  material lives in process memory and in `$BOLE_KEY` / key files
  (`/proc/<pid>/environ`); bole does not defend against an attacker who can read
  the host process's memory. (Zeroization of in-memory keys is not implemented and
  was assessed as marginal under this model.)
- **Network confidentiality/integrity of the transport itself.** The TCP/HTTP
  transports ship **without TLS**. Run them only over a trusted network or behind
  a TLS-terminating reverse proxy / SSH tunnel.
- **Transport-level peer authentication.** The transports do not themselves
  authenticate the peer; authorization is by the `Accessor` the server binds to a
  connection. Establishing *which* actor a connection is is the deployer's job
  (e.g. a proxy that maps mutual-TLS identity to an actor).
- **Side channels** (timing, cache), and **denial of service by resource
  volume** beyond the per-request caps documented below (e.g. a flood of valid
  connections).

## Known limitations (tracked)

These are real gaps a deployer must account for; each is tracked as a bead.

- **Signature repo-binding.** Ed25519 payloads are domain-separated per scheme
  (`bole-m2p`), which prevents cross-*scheme* reuse. They are **not** yet bound to
  a repository/namespace id, so the same key trusted in two repos could allow
  same-scheme cross-*repo* replay of an admin-authored artifact. Use distinct keys
  per repo until repo-binding lands. (No repo-identity primitive exists yet.)
- **Signed approvals gate local operations only (`bole-6i7`).** The strong,
  head-bound `SignedApprovalHook` is now the *only* configurable approval hook
  (kind `"signed-approval"`): it verifies distinct Ed25519 attestations of the
  exact head against a content-addressed approver registry loaded from the repo
  (`Repository::set_approvers` / `add_attestation`). The forgeable ref-counting
  `ApprovalHook` is **removed**. It gates local `advance_timeline` / `check_merge`
  (both mutate via advance — `bole-rdh`). **Limitation:** because it counts
  attestations in the repo's mutable `refs/attestations/` namespace, it is
  *non-deterministic* across replicas, so a **replicated push** into an
  approval-gated timeline is refused fail-closed (`bole-7c1`) rather than
  enforced remotely. A deterministic variant (binding approvals into the head's
  object closure — an "approval commit") is future work; until then, perform
  approval-gated advances locally, and treat approvals as defense-in-depth on top
  of write capability, not a remote-enforced control.
- **Forward-only secret revocation.** `MultiRecipientSecret::revoke` drops a
  recipient's key wrap but does not rotate the data key; a reader who already
  extracted the DK can still decrypt existing ciphertext. Pair a revocation with a
  value rotation (`secret rotate` / fresh `encrypt_for`) to defeat that.
- **Sync read-ACL is ref-granular.** Sync gates reads at the ref/timeline
  granularity: an actor gets the closure of refs it can read (`bole-yl2`). It does
  not offer sub-tree read filtering over the wire.

## Enforced protections (post-audit)

The internal audit's confirmed findings are fixed and regression-tested:

- **Read-ACL on served objects** — `serve_fetch` constrains client `want` to
  authorized advertised refs; a peer cannot fetch an arbitrary object id
  (`bole-yl2`).
- **No-write-down soundness** — the confined-actor write-down guard is scope-aware
  and cannot be disabled by an unrelated grant (`bole-kt8`).
- **Deletion enforcement** — advancing a timeline write-checks removed paths, not
  just the new tree (`bole-48r`).
- **Policy on mutation paths** — approval/policy hooks gate the actual advance
  (and replicated advance), not only a `Merge` event (`bole-rdh`).
- **Bounded decode / buffering** — pack decompression, frame, header, and body
  sizes are capped; the glob matcher is memoized against ReDoS (`bole-oby`,
  `bole-1hu`).
- **Authorize-before-store** — a no-write-capability push lands no objects
  (`bole-zez`); a pushed head must be a real snapshot and push-created timelines
  default to fast-forward-only (`bole-e9a`).
- **Signature domain separation** — all Ed25519 schemes are domain-tagged
  (`bole-m2p`).

- **Auditability of access decisions** — a repository can be given an
  `AuditSink` (`Repository::with_audit_sink`); a timeline advance then emits a
  structured `AuditEvent` — recording the actor, the ref, the old/new head, and
  the decision (allowed / denied / approval-required) — for every attempt an
  access or policy check decides, including ACL write-denials, not only
  policy-registry verdicts. The bole CLI
  installs a JSON-lines file sink when `$BOLE_AUDIT_LOG` is set, so
  agent-initiated timeline transitions and the access/policy verdicts that
  gated them are attributable after the fact (`bole-eean`). Emission is
  side-effect-only and never changes an operation's outcome.
- **Scoped collab refs never travel any serve path** — the collab endpoint
  advertises only `refs/collab/public/**` (+ cached remotes) structurally
  (`bole-g7i`), and the general sync advert now structurally excludes
  `refs/collab/scoped/**` for every accessor — without that gate, unlabeled
  scoped refs default to the lattice bottom and would advertise to anonymous
  peers. Push ops naming `refs/collab/*` (like `refs/policy/*`) are rejected
  as reserved, so a peer cannot squat a timeline that wedges the collab
  adverts (`bole-e78l`, WS8b M2).
- **Fail-closed on unknown hook kinds** — a repository pins its active policy
  root at `refs/policy/root` (a content-addressed tag whose closure transfers
  via sync); every advance/merge and both replicated-push paths (wire and
  in-process) resolve the root's declarative hooks, and a node that does not
  recognize a hook kind refuses the operation rather than skipping the hook.
  Push ops naming `refs/policy/*` are rejected as reserved (`bole-au0t`,
  WS1-O5). **Limitations:** adoption of a fetched root is deliberately NOT
  automatic — a node enforces a root only after it is pinned locally
  (`set_policy_root`), and a verified adopt-from-remote step (via
  `verify_chain`) is future work; and a pinned root containing the
  non-deterministic `signed-approval` hook refuses ALL replicated pushes
  (the `bole-7c1` guard), so approval-gated timelines and push replication
  remain mutually exclusive.

### Per-request resource caps

| Limit | Value | Where |
|-------|-------|-------|
| Max uncompressed object | 128 MiB | `store::pack::MAX_OBJECT_LEN` |
| Max objects per pack | 8,000,000 | `store::pack::MAX_PACK_OBJECTS` |
| Max total uncompressed pack | 1 GiB | `store::pack::MAX_PACK_TOTAL_LEN` |
| Max stream frame | 256 MiB | `sync::wire::MAX_FRAME_LEN` |
| Max HTTP header block | 64 KiB | `sync::http` |
| Max HTTP body | 256 MiB | `sync::http` |
| Max glob pattern/path | 8 KiB | `acl::glob::MAX_GLOB_LEN` |
| Max tree nesting depth | 256 | `repo::MAX_TREE_DEPTH` |

## Deployment guidance

- **Do not expose a bole network listener (`serve_tcp_once` / `serve_http_once`)
  on an untrusted network.** They lack TLS and peer authentication. Front them
  with a TLS-terminating, authenticating proxy, or restrict to a trusted network.
- **Use distinct signing keys per repository** until signature repo-binding lands.
- **Do not depend on approval hooks for security** until signed approvals are
  wired (`bole-6i7`).
- **Keep key material out of the repo** — supply master keys via `$BOLE_KEY` or
  `--key-file`; rotate with `secret rekey`.
- **Rotate secret values after revoking a reader**, not just the key wrap.

## Audit history

- **2026-07 (pass 1)** — Internal multi-agent adversarial review across nine
  surfaces (envelope, key provider, signatures, attestations, access-control,
  filtering, policy hooks, sync, deps/RNG/panics). 15 findings confirmed and
  remediated (`bole-oby`, `bole-yl2`, `bole-rdh`, `bole-kt8`, `bole-1hu`,
  `bole-48r`, `bole-e9a`, `bole-zez`, `bole-m2p`).
- **2026-07 (pass 2)** — Second-pass review of the surfaces pass 1 did not cover:
  structural/cyclic decode DoS, serde decode limits, ref-CAS concurrency/TOCTOU,
  filesystem path traversal, and a real `cargo audit` dependency scan. 7 findings
  confirmed and remediated (`bole-daf` RefName path-traversal, `bole-wy4`
  tree-walk stack-overflow, `bole-bti` ref-CAS serialization, `bole-qj4`
  advance-timeline CAS, `bole-sq4` push closure verification, `bole-1hd` worktree
  id validation, `bole-0x3` journal filename uniqueness), plus `bole-jio`
  (dependency advisories: bumped gix, cleared `cargo audit`).
- **No external / third-party cryptographic audit has been performed.** Both
  passes were internal adversarial reviews with per-finding verification.
