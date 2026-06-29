# WS1 — Hybrid Access / Policy Core

- **Bead:** `bole-fo2`
- **Depends on:** none (root of the critical path `WS1 → WS4 → WS5`)
- **Status:** design spec (not an implementation plan)
- **Conforms to:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md).
  All shared vocabulary (Label, LabelLattice, label rule, Clearance, Accessor,
  PolicyHook, content-addressed policy object, atomic refs) is defined there and
  is not re-derived here. This spec fixes the parts the foundations doc delegates
  to WS1: the exact read/write evaluation rules and the on-disk/on-wire
  representation.

---

## 1. Goal

Replace today's glob-only ACL system — which is labelled a "lattice" in prose but
is really an unordered set of glob protections — with the hybrid model locked in
the foundations doc:

1. A **real label lattice** as the foundation.
2. The existing glob ACLs re-expressed as a **degenerate two-point lattice**
   `public ⊑ protected`, so nothing the current CLI or the 247 tests do changes
   in observable behaviour.
3. A programmable **`PolicyHook`** for rules labels cannot express (approvals,
   time windows, signature requirements), of which the current `TimelinePolicy`
   (`ff` / `append` / `unrestricted`) becomes one built-in instance.
4. One documented **pipeline** that dissolves the current three-way confusion
   between "actor grants", "ACLs", and "Accessor":

   > **rules label _what_ needs clearance → clearances say _who_ holds it → the
   > Accessor is the runtime check; PolicyHooks decide what labels can't say.**

Policy is represented as **content-addressed objects** so WS5 can transfer and
verify it regardless of which authority model WS5 picks.

Non-goals: WS1 does not pick the sync authority model (WS5), does not design the
pack format (WS4), and does not specify the CLI verbs (WS7); it only notes the
CLI surface they imply (§8).

---

## 2. Architecture

Three layers, evaluated in this order at every guarded operation:

```
                 ┌─────────────────────────────────────────────┐
  resource ───►  │ 1. LABELLING   rules: glob→label, pat→label  │  (what is sensitive)
  (path / tl)    │    effective_label(r) = join of all matches  │
                 └───────────────────────┬─────────────────────┘
                                         │ Label
                 ┌───────────────────────▼─────────────────────┐
  actor ──────►  │ 2. CLEARANCE   ClearanceSet over the lattice │  (who is cleared)
  credential     │    + Read/Write capability bits              │
                 └───────────────────────┬─────────────────────┘
                                         │ allow / deny (info-flow)
                 ┌───────────────────────▼─────────────────────┐
  operation ──►  │ 3. ACCESSOR    can_read / can_write          │  (runtime label check)
  (read/write)   │    binds lattice + rules + clearances        │
                 └───────────────────────┬─────────────────────┘
                                         │ passed the label check
                 ┌───────────────────────▼─────────────────────┐
  event ──────►  │ 4. POLICYHOOK  advance / merge decision pts  │  (rules labels can't say)
  (advance/merge)│    TimelinePolicyHook + custom hooks         │
                 └─────────────────────────────────────────────┘
```

- The **lattice** and the **rule set** are repo-global, content-addressed policy
  objects named by a policy ref. They answer "what label does this resource
  carry?"
- A **clearance set** is an actor credential (the new `Accessor`'s payload). It
  answers "is this actor cleared for that label, for read / for write?"
- The **Accessor** is the runtime binder: `lattice + rules + clearances`. It is
  the single object repository operations consult, exactly as today.
- **PolicyHooks** run *after* the label check passes, only at `advance` and
  `merge`, for predicates the lattice cannot encode.

Components and module layout:

| Module | Responsibility | Status |
|--------|----------------|--------|
| `acl::lattice` | `Label`, `LabelLattice`, `dominates`/`join`/`meet` | new |
| `acl::rules` | `LabelRule`, `LabelRuleSet`, `effective_label` | new |
| `acl::clearance` | `Clearance`, `ClearanceSet`, `Capability` | new |
| `acl::mod` (`Accessor`) | runtime read/write evaluation | rewritten, compat shims kept |
| `acl::policy_object` | content-addressed `PolicyObject` + `PolicyRoot` | new |
| `acl::hook` | `PolicyHook` trait, `TimelinePolicyHook`, registry | new |
| `acl::backend` | persistence trait (extended) | extended |
| `acl::glob` | unchanged glob matcher (reused by rules) | unchanged |

---

## 3. Data model

All types `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` unless
noted, and serialize deterministically with `postcard` so their `ObjectId`
(BLAKE3 over the encoded bytes, per `ObjectId::from_content`) is stable — the
same property `copy_objects` already relies on.

### 3.1 Label and lattice

```rust
/// An opaque confidentiality marker. The string is only an identity; all
/// ordering comes from the LabelLattice. `Label::PUBLIC` is the conventional
/// bottom (least restrictive) element.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Label(pub String);

impl Label {
    pub const PUBLIC: &str = "public";       // bottom by convention
}

/// The partial order over labels. A content-addressed policy object (§3.4).
///
/// Stored as the set of labels plus the *covering* edges of the order
/// (a ⋖ b means "a is immediately dominated by b"; b is strictly more
/// restrictive). dominates/join/meet are computed by reachability over the
/// transitive closure, cached on load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelLattice {
    labels: BTreeSet<Label>,
    /// (lower, higher): `lower ⊑ higher`, i.e. higher dominates lower.
    cover: BTreeSet<(Label, Label)>,
}

impl LabelLattice {
    /// `a ⊒ b` — a dominates (is at least as restrictive as) b. Reflexive.
    pub fn dominates(&self, a: &Label, b: &Label) -> bool;
    /// Least upper bound (most-protective common ceiling). `None` if the order
    /// is not a true lattice for this pair (see Open question O3).
    pub fn join(&self, a: &Label, b: &Label) -> Option<Label>;
    /// Greatest lower bound.
    pub fn meet(&self, a: &Label, b: &Label) -> Option<Label>;
    pub fn bottom(&self) -> Label;  // the unique minimum (public)
    pub fn validate(&self) -> Result<()>; // acyclic, has bottom, joins exist
}
```

**Degenerate two-point case** (the current world):
`labels = {public, protected}`, `cover = {(public, protected)}`. `protected`
dominates `public`; `join(public, protected) = protected`. This *is* the entire
expressive content of today's `PathAcl`/`TimelineAcl` ("protected") system.

### 3.2 Label rules

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelRule {
    /// Any path matching `glob` carries `label`.
    Path { glob: String, label: Label },
    /// Any timeline whose name matches `pattern` carries `label`.
    Timeline { pattern: String, label: Label },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRuleSet { pub rules: Vec<LabelRule> }

impl LabelRuleSet {
    /// Effective label of a path = join of every matching Path rule's label,
    /// defaulting to `lattice.bottom()` (public) when nothing matches.
    /// "Most restrictive matching rule wins", and it composes for free in a
    /// real lattice instead of being a flat boolean.
    pub fn label_for_path(&self, lattice: &LabelLattice, path: &str) -> Label;
    pub fn label_for_timeline(&self, lattice: &LabelLattice, name: &str) -> Label;
}
```

Rationale for **join of matches** (vs first-match or boolean): with overlapping
rules (`secrets/**` → `secret`, `secrets/prod/**` → `top-secret`) the resource
correctly takes the most restrictive applicable label. In the two-point lattice
this collapses to exactly today's "matches any protected glob? → protected, else
public".

### 3.3 Clearance

```rust
bitflags! {
    /// Orthogonal to the lattice position: a clearance can grant read, write,
    /// or both, independent of which label it is for.
    #[derive(Serialize, Deserialize)]
    pub struct Capability: u8 { const READ = 0b01; const WRITE = 0b10; }
}

/// A single grant: "cleared up to `ceiling`, for these capabilities."
/// Downward-closed: holding `ceiling = L` clears the actor for every label `L`
/// dominates (the down-set of L).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clearance { pub ceiling: Label, pub cap: Capability }

/// What an actor holds. The union of the down-sets of its clearances, split by
/// capability. Represented as an antichain of ceilings rather than the
/// enumerated set, so it is O(grants) not O(labels).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceSet { pub clearances: Vec<Clearance> }
```

A `PathRole { glob, Read }` of today maps to: the rule `glob → protected` plus a
`Clearance { ceiling: protected, cap: READ }`. The glob lives in the rule (it is
"what is sensitive"); the capability lives in the clearance (it is "who may
touch it"). This split is the crux of the de-confusion.

### 3.4 Content-addressed policy objects

Policy must sync (WS5), so it is stored as ordinary content-addressed objects in
the existing `ObjectStore`. Add **one** variant to the `Object` enum
(`object::mod`), keeping the codec/`put`/`get` path untouched:

```rust
pub enum Object {
    Blob(Blob), Tree(Tree), Snapshot(Snapshot),
    Secret(Secret), EnvOverlay(EnvOverlay),
    Policy(PolicyObject),                     // new
}

/// Each kind is independently content-addressed so a replica can `have`/`want`
/// lattice and ruleset separately during pack negotiation (WS5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyObject {
    Lattice(LabelLattice),
    RuleSet(LabelRuleSet),
    /// An issued, optionally-signed clearance credential for one actor.
    Grant(ClearanceGrant),
    /// The root that ties a policy generation together; what a policy ref points at.
    Root(PolicyRoot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRoot {
    pub lattice: ObjectId,
    pub rules: ObjectId,
    pub parent: Option<ObjectId>,   // previous PolicyRoot → an audit chain
    pub hooks: Vec<HookSpec>,       // declarative hook bindings (§5.4)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceGrant {
    pub actor: String,              // actor identity (key id / name)
    pub clearances: ClearanceSet,
    pub signature: Option<Vec<u8>>, // WS5 fills the verification story; field reserved now
}
```

The **active policy** is named by a ref `refs/policy/current` → a `PolicyRoot`
`ObjectId` (refs already exist and are what `Repository::transaction()` in the
foundations doc will commit atomically alongside ref updates). Because every
piece is content-addressed:

- A replica can verify policy independent of transport (signed grants, hash
  chain via `PolicyRoot.parent`).
- `copy_objects`/pack-delta move policy with no special case.
- "Which policy was in force at snapshot X" is answerable by recording the
  `PolicyRoot` id, enabling reproducible historical access decisions.

### 3.5 AclStore / backend role under the new model

`AclStore` is **retained** as the local, fast, mutable index and the
compatibility surface. It is reframed as a cache/projection over the policy
objects + policy ref, not a parallel source of truth:

- Source of truth = the `PolicyRoot` reachable from `refs/policy/current`.
- `AclBackend` is extended with `get_label_ruleset` / `set_label_ruleset` /
  `get_lattice` / `set_lattice` / `get_grant(actor)` and the existing
  `*_path_acl` / `*_timeline_acl` methods are kept as **default-implemented
  shims** that translate to/from two-point rules (§7). `MemoryAclBackend` and
  `DiskAclBackend` gain the new fields; old on-disk ACL files still load.

---

## 4. Evaluation rules

### 4.1 Read rule (locked by foundations: confidentiality dominance)

> An actor may **read** a resource iff it holds a **Read-capable** clearance
> whose ceiling **dominates** the resource's effective label.

```rust
fn can_read(&self, resource_label: &Label) -> bool {
    self.clearances.iter().any(|c|
        c.cap.contains(Capability::READ) &&
        self.lattice.dominates(&c.ceiling, resource_label))
}
```

This is "no read up": you cannot read a resource more restrictive than your
clearance. In the two-point lattice, `dominates(protected, protected)` requires a
clearance ceiling of `protected`; an actor with only `public` (the empty/default
`Accessor`) cannot read protected paths — identical to today.

### 4.2 Write rule — **chosen: clearance-dominates (capped), with flow enforced separately**

> An actor may **write** a resource iff it holds a **Write-capable** clearance
> whose ceiling **dominates** the resource's effective label.

```rust
fn can_write(&self, resource_label: &Label) -> bool {
    self.clearances.iter().any(|c|
        c.cap.contains(Capability::WRITE) &&
        self.lattice.dominates(&c.ceiling, resource_label))
}
```

**The three candidates and why this one:**

- **Biba / integrity "no write up"** (a low subject can't write a high object,
  on an *integrity* lattice). Rejected as the core rule: bole has exactly one
  lattice and the foundations doc orients it toward *confidentiality* (reads use
  confidentiality dominance). Layering a Biba write rule on a confidentiality
  lattice produces the classic Biba/BLP deadlock where only same-level access is
  possible, and conflates two axes a maintainer would have to reason about
  jointly. The *useful* part of Biba — "untrusted contributors must not modify
  release-critical files" — is better modelled as a **PolicyHook** on
  `release/**` (§5.3), which is precisely why hooks exist.

- **Bell–LaPadula `*`-property "no write down"** (a subject may only write
  objects whose label dominates the subject's *current* level). Rejected as the
  core rule: BLP assumes a subject runs at a single current level. bole actors
  hold a *clearance set* and legitimately edit files at many levels in one
  session — a developer cleared for `secret` who also edits the public README is
  "writing down" constantly. Strict no-write-down would make ordinary
  multi-level work impossible and would break the current tests outright. Its
  goal — stop a high-cleared actor from copying secrets into a low resource
  (declassification/exfiltration) — is real but is a property of **information
  flow between resources**, not of a single write. bole already enforces flow
  with the merge-leak check (`check_merge`: protected content must not flow into
  an unprotected timeline). We keep flow control there, not in the per-write
  rule.

- **Write-equal** (label must match exactly). Rejected: too rigid — an admin
  cleared for the top label could not write a public file without a separate
  exact-match grant, and it has no analogue in the current model.

- **Clearance-dominates (chosen).** Symmetric with the read rule (same
  `dominates` check, keyed on the `WRITE` bit instead of `READ`). It is the
  exact generalisation of today's behaviour: "to write `secrets/prod.key` you
  need a Write grant covering it" becomes "your Write ceiling dominates the
  resource's label." It is backward-compatible with all 247 tests, it is the
  rule a VCS maintainer already has in their head, and it cleanly separates the
  two real questions: *may this actor touch this resource at all* (the write
  rule) vs *may protected content move to a less-protected place* (the flow /
  merge check + hooks).

**Security note (honest tradeoff, surfaced as O1).** Clearance-dominates does
not by itself prevent a *malicious* high-cleared actor from declassifying (read
`secret`, write the bytes into a `public` path). That is intentional: bole's
threat model is mistakes and least-privilege, not a confined adversary, and the
write rule that would prevent it (strict no-write-down) is unusable for everyday
multi-level work. For untrusted automation we offer an **optional** per-clearance
`confined` flag (future, see O1) that *additionally* enforces no-write-down for
that actor only — opt-in containment without taxing normal users. The
cross-resource leak case (merge) stays covered by `check_merge`.

### 4.3 How the Accessor evaluates against a resource's labels

```rust
pub struct Accessor {
    lattice: Arc<LabelLattice>,
    rules:   Arc<LabelRuleSet>,
    clearances: ClearanceSet,
}

impl Accessor {
    pub fn can_read_path(&self, path: &str) -> bool {
        self.can_read(&self.rules.label_for_path(&self.lattice, path))
    }
    pub fn can_write_path(&self, path: &str) -> bool {
        self.can_write(&self.rules.label_for_path(&self.lattice, path))
    }
    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.can_read(&self.rules.label_for_timeline(&self.lattice, name))
    }
    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.can_write(&self.rules.label_for_timeline(&self.lattice, name))
    }
    /// Read-everything, no write — the privileged() of today.
    pub fn privileged(lattice: Arc<LabelLattice>, rules: Arc<LabelRuleSet>) -> Self;
}
```

The four `can_*` method names and semantics are preserved so `repo/mod.rs` call
sites are untouched except for how the `Accessor` is constructed (§6, §7).

---

## 5. PolicyHook

### 5.1 Trait

```rust
/// A predicate evaluated at a write decision point, for rules the label lattice
/// cannot express. Hooks run AFTER the label read/write check passes; they can
/// only further restrict, never widen, access (monotone deny).
pub trait PolicyHook: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision;
}

pub enum PolicyEvent<'a> {
    Advance { timeline: &'a RefName, old_head: ObjectId, new_head: ObjectId },
    Merge   { source: &'a RefName, target: &'a RefName,
              old_head: ObjectId, result_head: ObjectId },
}

pub struct PolicyContext<'a> {
    pub event: PolicyEvent<'a>,
    pub accessor: &'a Accessor,
    pub objects: &'a ObjectStore,   // read-only ancestry / content queries
    pub refs: &'a RefStore,
    pub now: u64,
}

pub enum PolicyDecision {
    Allow,
    Deny(String),                                  // → Error::PolicyViolation
    RequiresApproval { reason: String, needed: u32 }, // surfaced like MergeCheck
}
```

Composition: a `PolicyRegistry` holds the bound hooks; the effective decision is
the **most restrictive** across all matching hooks (`Deny` > `RequiresApproval` >
`Allow`). Hooks cannot grant access the label check denied.

### 5.2 Where it is invoked

- **`advance_timeline`**: after the timeline + path write checks pass, before
  `refs.advance_head`. Replaces the inline `match timeline.policy { … }` block.
- **`merge_timelines` / `check_merge`**: after the write check on `target` and
  the confidentiality leak scan, before producing the result. A
  `RequiresApproval` decision composes with the existing
  `MergeCheck::RequiresApproval`.

### 5.3 The existing `TimelinePolicy` becomes a built-in hook

`TimelinePolicy::{FastForwardOnly, Append, Unrestricted}` (stored on the
`Timeline`) is reframed as `TimelinePolicyHook`, a built-in always-registered
hook:

```rust
pub struct TimelinePolicyHook;
impl PolicyHook for TimelinePolicyHook {
    fn name(&self) -> &str { "timeline-policy" }
    fn check(&self, ctx: &PolicyContext) -> PolicyDecision {
        if let PolicyEvent::Advance { timeline, old_head, new_head } = ctx.event {
            let tl = ctx.refs.get_timeline(timeline)?;
            match tl.policy {
                TimelinePolicy::Unrestricted => PolicyDecision::Allow,
                TimelinePolicy::FastForwardOnly | TimelinePolicy::Append => {
                    // identical fast-forward test as today: old_head must be an
                    // ancestor of new_head (lca(old,new) == old).
                    if is_fast_forward { Allow } else { Deny(...) }
                }
            }
        } else { PolicyDecision::Allow }
    }
}
```

The `TimelinePolicy` enum and `Timeline.policy` field **stay** (no migration of
ref storage); only the *enforcement site* moves from a hard-coded `match` in
`advance_timeline` into this hook. This keeps `refs` untouched and the policy
tests green.

### 5.4 Worked custom example — "merges into `release/**` need two approvals"

Declarative binding in `PolicyRoot.hooks` (`HookSpec { kind, pattern, params }`),
resolved to a hook instance by a registry, so policy stays content-addressed and
syncable rather than living only in compiled code:

```rust
pub struct ApprovalHook { pattern: String, needed: u32 }

impl PolicyHook for ApprovalHook {
    fn name(&self) -> &str { "approval" }
    fn check(&self, ctx: &PolicyContext) -> PolicyDecision {
        if let PolicyEvent::Merge { target, result_head, .. } = ctx.event {
            if glob_matches(&self.pattern, target.as_str()) {
                let approvals = count_approval_attestations(ctx.objects, result_head);
                if approvals < self.needed {
                    return PolicyDecision::RequiresApproval {
                        reason: format!("{} needs {} approvals, has {}",
                                        target.as_str(), self.needed, approvals),
                        needed: self.needed - approvals,
                    };
                }
            }
        }
        PolicyDecision::Allow
    }
}
// bound via: HookSpec { kind: "approval", pattern: "release/**", params: {needed: 2} }
```

Approvals themselves are attestation objects (a small content-addressed
`PolicyObject` extension or a signed note pointing at `result_head`); the precise
attestation format is deferred to WS5's signing story (O4).

---

## 6. Re-expressing existing operations

No call-site signatures in `repo/mod.rs` change. `Accessor` keeps its four
`can_*` methods; the repo now also owns a `PolicyRegistry` (built from the active
`PolicyRoot`). Internally:

- **`get_snapshot_filtered`** → unchanged shape. `walk_tree_filtered` still calls
  `accessor.can_read_path(full_path)`, but "is protected?" is no longer a
  separate `AclStore::path_is_protected` gate: every path now has an effective
  label (default `public`), and `can_read_path` returns `true` for `public`
  automatically (any clearance trivially dominates bottom, and the default
  Accessor is treated as holding read over `public`). The explicit
  `path_is_protected` branch collapses into the dominance check. Behaviour is
  identical: public paths visible to all, protected paths only to the cleared.

- **`check_merge`** → keeps `MergeCheck { Allowed, RequiresApproval, Rejected }`.
  The leak scan generalises from "path is protected & dest timeline not
  protected" to "a path's effective label is **not dominated by** the dest
  timeline's effective label" — i.e. content would flow to a strictly
  less-protected place. Two-point lattice reproduces today's result exactly.
  After the leak scan, registered `PolicyHook`s run for the `Merge` event;
  a hook `RequiresApproval` merges into `MergeCheck::RequiresApproval`, a
  hook `Deny` into `MergeCheck::Rejected`.

- **`advance_timeline`** → write-cap check on the timeline label, then per-path
  write-cap check (both via the new dominance rule), then the inline
  `TimelinePolicy` `match` is **replaced** by `registry.evaluate(Advance{…})`.
  `PolicyDecision::Deny` → `Error::PolicyViolation` (same error type as today);
  `RequiresApproval` → `Error::PolicyViolation` with the approval reason (or a
  new `Error::ApprovalRequired` — O2).

---

## 7. Backward compatibility & migration

**Requirement (foundations §3): the current CLI, repos, and 247 tests keep
working.** Strategy: the old API becomes a thin facade over the two-point
lattice; nothing is deleted in this WS.

### 7.1 What stays (unchanged public API)

- `Permission { Read, Write }` — retained; maps to `Capability` bits.
- `PathRole`, `TimelineRole`, `PathAcl`, `TimelineAcl` — retained as types.
- `AclStore::{set,remove,list}_path_acl`, `…_timeline_acl`,
  `path_is_protected`, `timeline_is_protected` — retained, now implemented over
  two-point rules.
- `Accessor::{new, with_path_role, with_timeline_role, privileged,
  can_read_path, can_write_path, can_read_timeline, can_write_timeline}` —
  retained signatures.
- `Repository::{get_snapshot_filtered, list_refs_filtered, check_merge,
  merge_timelines, advance_timeline, compute_workspace_view}` — retained.
- `TimelinePolicy` enum and `Timeline.policy` — retained.

### 7.2 Reinterpretation (the two-point lattice `{public ⊑ protected}`)

- Every repo gets a default `PolicyRoot` whose lattice is `{public, protected}`
  with `cover {(public, protected)}`, and an empty rule set.
- `AclStore::set_path_acl { glob }` ≡ insert `LabelRule::Path { glob, protected }`.
  `path_is_protected(p)` ≡ `label_for_path(p) == protected`.
- `Accessor::with_path_role(PathRole { glob, Read })` ≡ ensure the rule
  `glob → protected` exists *and* add `Clearance { ceiling: protected,
  cap: READ }`. (In the builder, the glob in the role is what selects which
  resources the clearance covers; the two-point model has only one non-trivial
  label, so any protected resource the role's glob matches is covered. The
  builder records the (glob, permission) pair and the new evaluator treats a
  `protected` resource as readable iff some Read role's glob matches it — exactly
  the current semantics, now phrased as: clearance for `protected` gated by the
  role glob.)
- `Accessor::privileged()` ≡ a Read clearance with ceiling = lattice top over
  all rules — read everything, no write, as today.

> Note: in the full model a clearance is by ceiling label, not by glob. The
> two-point compatibility layer preserves the *glob-scoped* role behaviour by
> keeping the role's glob as the selector; the general lattice path uses
> label-scoped clearances. Both coexist: a role is sugar for "a clearance whose
> reachable resources are those its glob matches at the `protected` label." This
> is the one place the compat shim is genuinely a special case, and it is
> documented as such.

### 7.3 Migration mechanics

- **On-disk:** `DiskAclBackend` continues to read existing path/timeline ACL
  files. On first open under the new code, a lazy migration synthesises a
  `PolicyRoot` (two-point lattice + rules derived from existing ACL files) and
  writes `refs/policy/current`. No destructive rewrite; the old files remain the
  serialization for the two-point rule set. New lattices/rulesets are written as
  `PolicyObject`s.
- **In-memory / tests:** `Repository::memory()` installs the default two-point
  `PolicyRoot` automatically, so existing tests that never mention labels keep
  passing.
- **No CLI verbs change in WS1.** New label/clearance verbs are additive (WS7).

### 7.4 What changes (additive, non-breaking)

- New types/modules in §2.
- New `Object::Policy` variant (codec is forward-compatible: an old reader
  encountering it errors clearly; within one release this is fine since policy
  objects are new).
- New `Repository::policy()` accessor and `Accessor::from_grant(...)`
  constructors for the label-native path.

---

## 8. CLI surface implications (high level — detail is WS7)

WS1 only notes that the model implies these verb families; **WS7 owns the
actual CLI**:

- `bole label` — define lattice points & edges; `bole label rule add
  <glob|pattern> <label>`; list/inspect effective label of a path/timeline.
- `bole clearance` — issue/list/revoke `ClearanceGrant`s for an actor; show an
  actor's effective read/write reach.
- `bole policy` — show the active `PolicyRoot`, its hooks, and the audit chain;
  `bole policy hook add <kind> <pattern> [params]`.
- Existing `acl`/role commands stay as aliases over the two-point lattice.

These are listed so WS7 has a target; their flags and UX are explicitly out of
scope here.

---

## 9. Testing strategy

- **Lattice algebra (property tests):** `dominates` is a partial order
  (reflexive, antisymmetric, transitive); `join`/`meet` agree with `dominates`;
  `validate` rejects cycles and missing bottom. Random small lattices.
- **Two-point equivalence (the compat guarantee):** a generated oracle that runs
  the *old* `Accessor`/`AclStore` logic and the *new* engine over the same
  glob/path/role inputs and asserts identical `can_read_path/…` results. This is
  the gate that protects the 247 tests; keep all current `acl` and `repo` tests
  unmodified and green.
- **Read rule:** no-read-up across ≥3-level chains and a diamond lattice.
- **Write rule:** clearance-dominates accepts multi-level writers; confirm a
  `secret`-cleared actor can write `public`; confirm an uncleared actor cannot
  write `protected`. Negative test documenting that declassification is *not*
  blocked by the write rule (locks the chosen tradeoff so a future change is a
  conscious one).
- **Effective label:** join-of-matches with overlapping rules; default-public.
- **PolicyHook:** `TimelinePolicyHook` reproduces every current
  `advance_timeline` policy test verbatim; `ApprovalHook` blocks then allows a
  `release/**` merge as approvals accrue; most-restrictive composition of two
  hooks.
- **Policy objects:** round-trip `put`/`get` for each `PolicyObject` kind; stable
  `ObjectId`; `copy_objects` carries policy across repos (pre-WS5 smoke test).
- **Migration:** open a repo written by the old code, assert synthesised
  two-point `PolicyRoot` reproduces prior protections.

---

## 10. Open questions (need the maintainer's call — not silently decided)

- **O1 — Declassification containment.** The chosen write rule
  (clearance-dominates) deliberately does *not* stop a high-cleared actor from
  copying protected content into a public path; cross-resource leaks are caught
  only at merge. Do we ship the optional per-clearance **`confined` (strict
  no-write-down)** flag in WS1 for untrusted agents, defer it to WS3
  (secrets/env, where agent confinement lives), or not build it? *Recommendation:
  reserve the field now, implement in WS3.*

- **O2 — How `RequiresApproval` surfaces from `advance_timeline`.** `merge` has
  `MergeCheck::RequiresApproval`, but `advance_timeline` returns `Result<()>`.
  Add `Error::ApprovalRequired { reason, needed }`, or change
  `advance_timeline` to return an enum like `check_merge` does? The latter is a
  signature change (touches §7.1's "stays"). *Recommendation: add the error
  variant; keep the signature.*

- **O3 — Must the order be a true lattice, or a general poset?** A real lattice
  guarantees `join`/`meet` exist (well-defined "most restrictive of matching
  rules"). A bare poset is more flexible but makes effective-label undefined when
  two matching rules have no upper bound. *Recommendation: require a true lattice
  (`validate` enforces unique bottom + existing joins); reject configs that
  aren't.* Confirm.

- **O4 — Approval / signature attestation format.** The two-approval hook needs a
  way to record and verify approvals. This overlaps WS5's signing/authority
  decision. Does WS1 define a minimal `Attestation` `PolicyObject` now, or fully
  defer to WS5? *Recommendation: define the placeholder type, leave verification
  to WS5.*

- **O5 — Where do hooks come from across replicas?** `PolicyRoot.hooks` is a
  declarative `HookSpec` list resolved by a registry of *compiled* hook kinds. A
  replica that lacks a hook kind named in a synced `PolicyRoot` must
  fail-closed (deny) — confirm that's the desired posture vs. fail-open or
  warn-and-skip. Pure-data hooks (a small expression language) would remove the
  "unknown kind" problem but are a much larger surface. *Recommendation:
  fail-closed on unknown hook kinds; revisit a policy DSL post-WS5.*

- **O6 — Clearance scoping in the general model.** The two-point compat layer
  keeps role *globs* as selectors (§7.2). In the full lattice, is a clearance
  purely label-scoped (`ceiling` only), or do we also support glob-scoped
  clearances natively (e.g. "Write, but only under `src/**`, at label `secret`")?
  The former is cleaner; the latter is what some current roles express.
  *Recommendation: label-scoped clearances are canonical; glob-scoped roles
  remain a compat-only sugar.* Confirm whether native glob-scoped clearances are
  wanted long-term.
