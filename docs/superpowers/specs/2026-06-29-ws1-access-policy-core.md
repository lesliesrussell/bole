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

/// A **bounded lattice** (LOCKED, decision O3 / 2026-06-29) — not a general
/// poset. Every pair of labels has a unique join (least upper bound) and a
/// unique meet (greatest lower bound), and the order has a unique bottom
/// (`public`) and a unique top. A content-addressed policy object (§3.4).
///
/// Stored as the set of labels plus the *covering* edges of the order
/// (a ⋖ b means "a is immediately dominated by b"; b is strictly more
/// restrictive). dominates/join/meet are computed by reachability over the
/// transitive closure, cached on load. Because the structure is a true lattice,
/// join/meet are **total** — they always return a label — so every downstream
/// rule (effective-label composition §3.2, clearance evaluation §4) can rely on
/// them existing without an `Option`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelLattice {
    labels: BTreeSet<Label>,
    /// (lower, higher): `lower ⊑ higher`, i.e. higher dominates lower.
    cover: BTreeSet<(Label, Label)>,
}

impl LabelLattice {
    /// `a ⊒ b` — a dominates (is at least as restrictive as) b. Reflexive.
    pub fn dominates(&self, a: &Label, b: &Label) -> bool;
    /// `a ⊐ b` — strict domination: `dominates(a, b) && a != b`. Used by the
    /// confined (no-write-down) write rule (§4.2).
    pub fn strictly_dominates(&self, a: &Label, b: &Label) -> bool;
    /// Least upper bound (most-protective common ceiling). Total: always exists.
    pub fn join(&self, a: &Label, b: &Label) -> Label;
    /// Greatest lower bound. Total: always exists.
    pub fn meet(&self, a: &Label, b: &Label) -> Label;
    pub fn bottom(&self) -> Label;  // the unique minimum (public)
    pub fn top(&self) -> Label;     // the unique maximum (most restrictive)
    /// Rejects configs that are not bounded lattices: cycles, missing/duplicate
    /// bottom or top, or any pair lacking a unique join or meet.
    pub fn validate(&self) -> Result<()>;
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
    /// Any secret matching `name` (env-var name or secret id) carries `label`.
    /// (LOCKED, decision #4 / 2026-06-29; ratifies WS3's request.) Makes secrets
    /// first-class in the label model: WS3's `resolve_overlay` labels an
    /// EnvOverlay's `EnvValue::Secret(id)` entries via this rule and gates them
    /// through the same read clearance check as paths, instead of a separate
    /// secret-ACL path.
    Secret { name: String, label: Label },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRuleSet { pub rules: Vec<LabelRule> }

impl LabelRuleSet {
    /// Effective label of a path = JOIN of every matching Path rule's label,
    /// defaulting to `lattice.bottom()` (public) when nothing matches.
    pub fn label_for_path(&self, lattice: &LabelLattice, path: &str) -> Label;
    pub fn label_for_timeline(&self, lattice: &LabelLattice, name: &str) -> Label;
    /// Effective label of a secret = JOIN of every matching Secret rule's label.
    pub fn label_for_secret(&self, lattice: &LabelLattice, name: &str) -> Label;
}
```

**Effective label = JOIN of all matching rules of the resource's kind** (LOCKED,
decision O3 / 2026-06-29). When several rules match (`secrets/**` → `secret`,
`secrets/prod/**` → `top-secret`) the resource takes the least upper bound of the
matched labels — the most restrictive applicable label — defaulting to
`bottom()` (public) when nothing matches. Because the structure is a true bounded
lattice (§3.1) this join always exists and is unique, so composition is total and
deterministic regardless of rule order. In the two-point lattice this collapses
to exactly today's "matches any protected glob? → protected, else public".

### 3.3 Clearance

```rust
bitflags! {
    /// Orthogonal to the lattice position: a clearance can grant read, write,
    /// or both, independent of which label it is for.
    #[derive(Serialize, Deserialize)]
    pub struct Capability: u8 { const READ = 0b01; const WRITE = 0b10; }
}

/// A single grant: "cleared up to `ceiling`, for these capabilities,
/// optionally only within `scope`."
/// Downward-closed in the lattice: holding `ceiling = L` clears the actor for
/// every label `L` dominates (the down-set of L).
///
/// `scope` (LOCKED, decision O6 / 2026-06-29) makes glob/timeline scoping a
/// **native** clearance property, not just compat sugar: a grant can be both
/// label-bounded AND resource-scoped, e.g. "Write at label `secret`, but only
/// under `src/**`". `None` scope = applies to every resource at/under `ceiling`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clearance {
    pub ceiling: Label,
    pub cap: Capability,
    pub scope: Option<ClearanceScope>,
}

/// Optional resource scope on a clearance. A scope restricts which resources the
/// clearance applies to; it never widens the label bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClearanceScope {
    Path(String),       // glob, matched with acl::glob::glob_matches
    Timeline(String),   // pattern
    Secret(String),     // secret name / id
}

/// What an actor holds. The union of the down-sets of its clearances, split by
/// capability and constrained by per-clearance scope. Represented as an
/// antichain of ceilings rather than the enumerated set, so it is O(grants) not
/// O(labels).
///
/// `confined` (LOCKED, decision O1 / 2026-06-29) opts the whole actor into the
/// no-write-down `*`-property defined in §4.2 — intended for untrusted agents.
/// Default `false`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceSet {
    pub clearances: Vec<Clearance>,
    pub confined: bool,
}
```

A `PathRole { glob, Read }` of today lowers natively into a single scoped
`Clearance { ceiling: protected, cap: READ, scope: Some(Path(glob)) }` — no
special-case compat layer needed (§7.2 is simplified accordingly). The label
bound lives in `ceiling` ("how sensitive"), the verb in `cap` ("read vs write"),
and the resource selector in `scope` ("which resources"). This split is the crux
of the de-confusion.

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

A clearance is now scoped (§3.3), so evaluation matches against both the
resource's **effective label** and its **identity** (`ResourceRef`):

```rust
/// Identifies the concrete resource being checked, for scope matching.
pub enum ResourceRef<'a> { Path(&'a str), Timeline(&'a str), Secret(&'a str) }

/// A scoped clearance *applies* to a resource iff its scope is absent or matches
/// the resource's kind and selector.
fn scope_applies(scope: &Option<ClearanceScope>, r: ResourceRef) -> bool {
    match (scope, r) {
        (None, _) => true,
        (Some(ClearanceScope::Path(g)),     ResourceRef::Path(p))     => glob_matches(g, p),
        (Some(ClearanceScope::Timeline(g)), ResourceRef::Timeline(t)) => glob_matches(g, t),
        (Some(ClearanceScope::Secret(s)),   ResourceRef::Secret(n))   => glob_matches(s, n),
        _ => false, // a path-scoped clearance never applies to a timeline, etc.
    }
}
```

### 4.1 Read rule (locked by foundations: confidentiality dominance)

> An actor may **read** a resource iff it holds a **Read-capable** clearance that
> **applies in scope** and whose ceiling **dominates** the resource's effective
> label.

```rust
fn can_read(&self, label: &Label, r: ResourceRef) -> bool {
    self.clearances.iter().any(|c|
        c.cap.contains(Capability::READ) &&
        scope_applies(&c.scope, r) &&
        self.lattice.dominates(&c.ceiling, label))
}
```

This is "no read up": you cannot read a resource more restrictive than your
clearance. In the two-point lattice, `dominates(protected, protected)` requires a
clearance ceiling of `protected`; an actor with only `public` (the empty/default
`Accessor`) cannot read protected paths — identical to today. `confined` does not
affect reads.

### 4.2 Write rule — **chosen: clearance-dominates (capped, scoped), with optional confinement and flow enforced separately**

> **Base rule (all actors).** An actor may **write** a resource iff it holds a
> **Write-capable** clearance that **applies in scope** and whose ceiling
> **dominates** the resource's effective label.
>
> **Confinement rule (LOCKED, decision O1 / 2026-06-29; applies only when the
> actor's `ClearanceSet.confined == true`).** A confined actor may additionally
> **not** write any resource whose effective label it *strictly* dominates. It
> may only write at-or-incomparable labels (equal to, or unordered against, every
> label its clearances reach), never strictly downward — the Bell–LaPadula
> `*`-property restricted to this actor.

```rust
fn can_write(&self, label: &Label, r: ResourceRef) -> bool {
    // Base: some write-capable, in-scope clearance dominates the label.
    let dominated = self.clearances.iter().any(|c|
        c.cap.contains(Capability::WRITE) &&
        scope_applies(&c.scope, r) &&
        self.lattice.dominates(&c.ceiling, label));
    if !dominated { return false; }

    // Confinement (*-property): forbid declassifying writes — writing to a
    // resource strictly below this actor's reach. `strictly_dominates(a, b)` is
    // `dominates(a, b) && a != b`. A confined actor may only write where some
    // write-capable clearance is at-or-incomparable to the target label.
    if self.clearances.confined {
        let writes_down = self.clearances.iter().all(|c|
            !c.cap.contains(Capability::WRITE)
            || self.lattice.strictly_dominates(&c.ceiling, label));
        if writes_down { return false; }
    }
    true
}
```

Precisely: a confined actor is denied a write whenever **every** write-capable
clearance it holds *strictly* dominates the target label (i.e. the target sits
strictly below all of the actor's write reach — the definition of writing down).
It is allowed when at least one write-capable clearance has a ceiling **equal to**
or **incomparable with** the target label. Reads are unaffected; default
(`confined == false`) actors use the base rule only. This is the intended
containment for **untrusted agents**: an agent cleared to *read* `secret` cannot
launder that content into a `public` path, because writing `public` (strictly
below `secret`) is denied. It **composes with, and does not replace,**
`check_merge`'s confidentiality leak scan (§6): per-write confinement stops an
agent authoring a declassifying snapshot in the first place, while the merge scan
remains the cross-timeline flow backstop for all actors including unconfined ones.

**The three candidates and why the base rule is clearance-dominates:**

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

**Security note (the tradeoff and how confinement closes it).** The *base*
clearance-dominates rule does not by itself prevent a *malicious* high-cleared
actor from declassifying (read `secret`, write the bytes into a `public` path):
bole's default threat model is mistakes and least-privilege, not a confined
adversary, and forcing strict no-write-down on *everyone* is unusable for
everyday multi-level work (a `secret`-cleared developer must still edit the public
README). The locked answer is the per-actor `confined` opt-in defined above:
untrusted actors (agents) get strict no-write-down, normal users keep the base
rule. The cross-resource leak case (merge) stays covered by `check_merge` for all
actors regardless of confinement.

### 4.3 How the Accessor evaluates against a resource's labels

```rust
pub struct Accessor {
    lattice: Arc<LabelLattice>,
    rules:   Arc<LabelRuleSet>,
    clearances: ClearanceSet,
}

impl Accessor {
    pub fn can_read_path(&self, path: &str) -> bool {
        self.can_read(&self.rules.label_for_path(&self.lattice, path),
                      ResourceRef::Path(path))
    }
    pub fn can_write_path(&self, path: &str) -> bool {
        self.can_write(&self.rules.label_for_path(&self.lattice, path),
                       ResourceRef::Path(path))
    }
    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.can_read(&self.rules.label_for_timeline(&self.lattice, name),
                      ResourceRef::Timeline(name))
    }
    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.can_write(&self.rules.label_for_timeline(&self.lattice, name),
                       ResourceRef::Timeline(name))
    }
    /// New, for WS3's resolve_overlay: gate a secret by its Secret-rule label.
    pub fn can_read_secret(&self, name: &str) -> bool {
        self.can_read(&self.rules.label_for_secret(&self.lattice, name),
                      ResourceRef::Secret(name))
    }
    /// Read-everything, no write — the privileged() of today.
    pub fn privileged(lattice: Arc<LabelLattice>, rules: Arc<LabelRuleSet>) -> Self;
}
```

The four legacy `can_*_path`/`can_*_timeline` method names and semantics are
preserved so `repo/mod.rs` call sites are untouched except for how the `Accessor`
is constructed (§6, §7); `can_read_secret` is additive for WS3. All of them honor
both the clearance scope and the `confined` flag automatically because they
delegate to the scoped `can_read`/`can_write` core.

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
  Because the per-path write check now flows through the scoped/confined
  `can_write` core (§4.2), a `confined` agent cannot advance a timeline with a
  snapshot that contains a declassifying write — the down-write is rejected here,
  before the head moves. `PolicyDecision::Deny` → `Error::PolicyViolation` (same
  error type as today); `RequiresApproval` → `Error::PolicyViolation` with the
  approval reason (or a new `Error::ApprovalRequired` — O2).

- **`compute_workspace_view`** → unchanged shape, but env-overlay resolution now
  gates each `EnvValue::Secret(id)` through `accessor.can_read_secret(name)` using
  `LabelRule::Secret` (decision #4). This is the seam WS3's `resolve_overlay`
  builds on; default (no secret rules) leaves every secret at `public`, so current
  behaviour is preserved.

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
- `Accessor::with_path_role(PathRole { glob, perm })` lowers **natively** (no
  special case, decision O6) into a scoped clearance:
  `Clearance { ceiling: protected, cap: perm.into(), scope: Some(Path(glob)) }`
  (plus, for `AclStore`-driven protection, the rule `glob → protected`). The
  role's glob becomes the clearance `scope`; the new scoped evaluator (§4.3)
  reproduces exactly the current semantics — a `protected` resource is readable
  iff some Read clearance whose scope-glob matches it dominates `protected`.
  `TimelineRole` lowers identically with `scope: Some(Timeline(pattern))`.
- `Accessor::privileged()` ≡ a Read clearance with ceiling = lattice top,
  `scope: None` — read everything, no write, as today.

Because clearance scoping is now a first-class field (§3.3), `PathRole` /
`TimelineRole` are a thin backward-compat *surface* that lowers losslessly into
scoped clearances; there is no longer any special-case shim in the evaluator.

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
  (reflexive, antisymmetric, transitive); `join`/`meet` are total and agree with
  `dominates`; `validate` **rejects non-bounded-lattice configs** — cycles,
  missing/duplicate top or bottom, and any pair lacking a unique join/meet
  (locks O3). Random small lattices including diamonds.
- **Two-point equivalence (the compat guarantee):** a generated oracle that runs
  the *old* `Accessor`/`AclStore` logic and the *new* engine over the same
  glob/path/role inputs and asserts identical `can_read_path/…` results. This is
  the gate that protects the 247 tests; keep all current `acl` and `repo` tests
  unmodified and green. Includes asserting `PathRole`/`TimelineRole` lower to
  scoped clearances with identical observable behaviour (O6).
- **Read rule:** no-read-up across ≥3-level chains and a diamond lattice.
- **Write rule (base):** clearance-dominates accepts multi-level writers; confirm
  an unconfined `secret`-cleared actor can write `public`; confirm an uncleared
  actor cannot write `protected`.
- **Write rule (confined, O1):** a `confined` actor cleared to read/write
  `secret` is **denied** a write to `public` (strict no-write-down); is allowed a
  write at `secret` (equal) and at an incomparable label; reads are unaffected by
  `confined`. Includes the agent-laundering scenario: confined agent reads
  `secret`, attempts to write the bytes to a `public` path → denied at the write,
  before any merge.
- **Scoped clearances (O6):** a `Clearance` scoped to `src/**` permits writes
  under `src/**` but not elsewhere at the same label; a path-scoped clearance
  never satisfies a timeline check.
- **Effective label:** JOIN of all matching rules with overlapping path rules;
  default-public; secret rules (decision #4) label a secret by name and gate
  `can_read_secret`.
- **PolicyHook:** `TimelinePolicyHook` reproduces every current
  `advance_timeline` policy test verbatim; `ApprovalHook` blocks then allows a
  `release/**` merge as approvals accrue; most-restrictive composition of two
  hooks.
- **Policy objects:** round-trip `put`/`get` for each `PolicyObject` kind; stable
  `ObjectId`; `copy_objects` carries policy across repos (pre-WS5 smoke test).
- **Migration:** open a repo written by the old code, assert synthesised
  two-point `PolicyRoot` reproduces prior protections.

---

## 10. Resolved decisions (maintainer, 2026-06-29)

These were open forks; the maintainer has locked them and they are now folded
into the body above. One-line rationale each.

- **O3 — TRUE bounded lattice (not a general poset).** §3.1, §3.2. *Rationale:
  guaranteeing a unique join/meet for every pair makes effective-label
  composition total and order-independent — a resource matching multiple rules
  takes the JOIN of the matched labels — and lets clearance/evaluation rely on
  meet/join existing without `Option`.*
- **O1 — SHIP the `confined` / no-write-down opt-in now.** §3.3 (`ClearanceSet.confined`),
  §4.2 (write predicate). *Rationale: declassification by a high-cleared actor is
  a real risk for untrusted agents; the opt-in gives strict `*`-property
  containment for those actors without taxing normal multi-level users, and it
  composes with `check_merge`'s leak scan rather than replacing it.*
- **O6 — NATIVE glob/timeline-scoped clearances.** §3.3 (`Clearance.scope`,
  `ClearanceScope`), §4 (scope-aware evaluation), §7.2 (roles lower into scoped
  clearances). *Rationale: a grant should be able to be both label-bounded and
  resource-scoped natively; this also removes the only special-case compat shim,
  since `PathRole`/`TimelineRole` now lower losslessly.*
- **#4 — `LabelRule::Secret` kind.** §3.2 (`LabelRule::Secret`,
  `label_for_secret`), §4.3 (`can_read_secret`). *Rationale: makes secrets
  first-class in the label model so WS3's `resolve_overlay` gates secret values
  through the same clearance check as paths/timelines instead of a separate
  secret-ACL path.*

## 11. Open questions (still need the maintainer's call — deferred)

- **O2 — How `RequiresApproval` surfaces from `advance_timeline`.** `merge` has
  `MergeCheck::RequiresApproval`, but `advance_timeline` returns `Result<()>`.
  Add `Error::ApprovalRequired { reason, needed }`, or change
  `advance_timeline` to return an enum like `check_merge` does? The latter is a
  signature change (touches §7.1's "stays"). *Recommendation: add the error
  variant; keep the signature.*

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
