# bole Roadmap — Cross-Cutting Foundations

This document fixes the vocabulary and load-bearing decisions shared by the
seven workstream specs (`2026-06-29-ws1` … `ws7`). Each spec MUST conform to the
names and principles here; where a spec needs to refine a rule, it does so
explicitly and notes the deviation. This file is the single source of truth for
the cross-cutting model.

## Locked decisions

1. **Access model = hybrid** (WS1). A real label lattice is the foundation;
   today's glob ACLs are a degenerate two-point case; a programmable
   `PolicyHook` covers rules labels can't express. Not pure capabilities, not a
   bare IFC engine.
2. **Distribution is in scope** (WS5). Therefore the WS4 pack format is designed
   from the start as the on-wire transfer payload — one format on disk and on
   the network.
3. **Backward compatibility is required.** Existing repos, the current CLI
   surface, and the 247 passing tests must keep working. New models are
   introduced as supersets with documented migrations, not breaking rewrites.

### WS1 access-model decisions (locked 2026-06-29)

These resolve the WS1 gating questions; the WS1 spec carries the detail.

- **True lattice, not a general poset.** Labels form a bounded lattice with a
  unique join and meet for every pair. A resource matching multiple label rules
  takes the **join** of the matched labels.
- **Ship `confined` / no-write-down now.** A `confined` actor may not write to
  any resource whose label it strictly dominates (no declassification downward),
  on top of the default clearance-dominates write rule. For untrusted agents;
  composes with `check_merge`'s leak scan.
- **Native glob-scoped clearances.** A clearance may carry an optional
  path-glob / timeline-pattern scope. The existing `PathRole`/`TimelineRole`
  lower into scoped clearances for backward compatibility.
- **`LabelRule::Secret` ratified.** Secrets are first-class in the label model
  (label-by-name/id), alongside path-glob and timeline-pattern rules; WS3's
  `resolve_overlay` relies on it.

Still open (deferred by the maintainer): how `RequiresApproval` surfaces from
`advance_timeline` (WS1-O2), attestation/signature format (WS1-O4, overlaps
WS5), and fail-closed behavior on unknown hook kinds across replicas (WS1-O5).

## Vocabulary — access & policy (WS1, referenced by WS3/WS5/WS6/WS7)

- **Label** — an opaque marker drawn from a partial order. The order is declared
  by a **`LabelLattice`** that defines `dominates(a, b) -> bool` and derives
  `join`/`meet`. The lattice is itself a content-addressed policy object so it
  can be transferred and verified (WS5).
- **Label rule** — assigns labels to resources: `glob → label` for paths,
  `pattern → label` for timelines. The current `PathAcl`/`TimelineAcl`
  ("protected") become the two-point lattice `public ⊑ protected` with rules
  assigning `protected`.
- **Clearance** — what an **actor** holds: the set of labels it is cleared for,
  downward-closed under the lattice. Each clearance carries an orthogonal
  capability bit (`Read` / `Write`). Current `PathRole`/`TimelineRole` grants
  become clearances for the label the matching rule assigns.
- **Accessor** — the runtime *evaluation*: it binds an actor's
  clearances/grants and answers `can_read`/`can_write` against a resource's
  labels. One pipeline, three stages: **rules label *what* needs clearance →
  actor clearances say *who* has it → the Accessor is the runtime check.** This
  is the sentence that resolves the "three permission concepts" confusion.
- **PolicyHook** — a trait invoked at decision points (`advance`, `merge`) for
  rules not expressible via labels (e.g. "merges into `release/**` need two
  approvals"). The existing timeline policy (`ff`/`append`/`unrestricted`)
  becomes one built-in hook.

WS1 fixes the exact read/write evaluation rule (confidentiality dominance for
reads; the write rule is WS1's to specify) and the on-disk representation. All
other specs treat the above as the stable surface.

## Vocabulary — storage & transfer (WS4, referenced by WS5)

- **Pack** — a single file: header + zstd-compressed stream of encoded objects,
  plus a sorted `ObjectId → (offset, len)` index (the `.idx`). Reads consult
  loose objects first, then packs via index.
- **Repack / GC** — `repack` consolidates loose objects into a pack;
  mark-and-sweep GC from refs drops unreachable objects. Together these are the
  consistency story: a crashed write leaves collectible orphans, never
  corruption.
- **Transfer = pack delta** — sync negotiates `have`/`want` sets by `ObjectId`;
  the sender emits a pack of exactly the missing objects. The on-disk pack and
  the on-wire payload are the *same* format. WS4 must not design a pack format
  that can't be streamed.
- **Atomic refs** — `Repository::transaction()` commits a batch of ref + policy
  updates all-or-nothing (single journal/rename). Objects need no transaction
  (immutable, content-addressed).

## Vocabulary — workspaces (WS2, referenced by WS3/WS6)

- **`Workspace` trait** — `read`/`write`/`remove`/`paths`/`diff`/`commit` over a
  `path → bytes` view. Two impls: `DiskWorkspace` (filesystem-backed, what the
  CLI drives) and the existing in-memory `EphemeralWorkspace`. The work tree is
  not a second model — it is one model with a filesystem backing.

## Sync — the hard authority question (WS5 owns the decision)

Distribution forces a decision the local model never had: **who is the source of
truth for policy (labels, rules, clearances) when replicas disagree?** WS5 must
specify one of: a designated authority replica; signed, content-addressed policy
objects with a verification chain; or last-writer-wins with an audit log.
Constraint from this doc: policy is represented as content-addressed objects so
it *can* be transferred and verified regardless of which authority model wins.

## Spec conventions

- Each spec lives at `docs/superpowers/specs/2026-06-29-ws<N>-<slug>.md`.
- Each is a **design spec** (not an implementation plan): goal, architecture,
  components with clear boundaries, data model, public API surface, key
  decisions with rationale, backward-compat/migration, testing strategy, and an
  explicit **Open questions** section for forks that need the maintainer's call.
- Each names its bead id and its dependencies, and references this foundations
  doc rather than re-deriving shared vocabulary.

## Bead map

| WS | Bead | Depends on |
|----|------|-----------|
| 1 Access/policy core | `bole-fo2` | — |
| 2 Workspace unification | `bole-1kz` | — |
| 3 Secrets/env completion | `bole-9mz` | WS1 |
| 4 Storage + packs + GC | `bole-81z` | — |
| 5 Distributed sync | `bole-cy6` | WS1, WS4 |
| 6 Git import | `bole-mtq` | WS1 |
| 7 Worktree hardening + repositioning | `bole-3hj` | WS1–3 |

Critical path: **WS1 → WS4 → WS5**. WS2 and WS6 run in parallel; WS7 is the capstone.
