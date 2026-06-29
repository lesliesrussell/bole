# WS1 — Hybrid Access / Policy Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace bole's glob-only ACL system with a real bounded label lattice, scoped clearances, content-addressed policy objects, and a programmable PolicyHook layer, while preserving every observable behaviour of the current CLI and the 247 existing tests.

**Architecture:** A `LabelLattice` (`acl::lattice`) defines the partial order; `LabelRuleSet` (`acl::rules`) assigns the JOIN of matching rules as a resource's effective label; `ClearanceSet` (`acl::clearance`) holds an actor's scoped, capability-tagged grants; the rewritten `Accessor` (`acl::mod`) is the runtime check (`can_read`/`can_write` = scope match + lattice dominance, plus an optional `confined` no-write-down rule). Policy is persisted as content-addressed `Object::Policy(PolicyObject)` values, and a `PolicyRegistry` of `PolicyHook`s runs at `advance`/`merge` for rules labels cannot express. Today's glob ACLs become the degenerate two-point lattice `public ⊑ protected`, derived on demand by the `AclBackend` shims, so nothing observable changes.

**Tech Stack:** Rust (edition 2021), `serde` + `postcard` (deterministic content-addressing), `blake3` (`ObjectId`), `bitflags` (new — `Capability`), `async-trait` (already a dependency — async `PolicyHook`), `thiserror` (errors). Tests use `tokio::test` and `tempfile`.

## Global Constraints

Every task's requirements implicitly include this section.

- Preserve all 247 existing tests; they must compile unmodified and stay green.
- No `anyhow` in library code — use `thiserror`'s `crate::error::Error`/`Result` only.
- Both ACL backends (`MemoryAclBackend`, `DiskAclBackend`) are always compiled — no feature flags gate the new model.
- Tag each contiguous block of newly added code with a `// bole-fo2` comment — one comment per block, containing the bead id and nothing else.
- All policy types serialize deterministically with `postcard` so their `ObjectId` (BLAKE3 over the encoded bytes, via `ObjectId::from_content`) is stable.
- Add the `bitflags` crate dependency; `Capability` is defined with `bitflags!`.
- Conservative git policy: every task commits **locally** on a branch named `bole-fo2`. Do **not** push and do **not** run Dolt remote sync.

---

### Task 1: `bitflags` dependency + `Capability` bitflags

**Files:**
- Modify: `Cargo.toml:12-23` (add the `bitflags` dependency)
- Create: `src/acl/clearance.rs`
- Modify: `src/acl/mod.rs:1-5` (declare the `clearance` module)

**Interfaces:**
- Consumes: nothing.
- Produces: `acl::clearance::Capability` — a `bitflags!` struct over `u8` with `const READ = 0b01; const WRITE = 0b10;`, deriving `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`. Methods used downstream: `Capability::contains(self, other) -> bool`, `Capability::READ`, `Capability::WRITE`, and the `|` operator.

- [ ] **Step 1: Create the `bole-fo2` branch**

```bash
git checkout -b bole-fo2
```

- [ ] **Step 2: Add the `bitflags` dependency**

In `Cargo.toml`, under `[dependencies]` (after the `gix` line at `Cargo.toml:23`), add:

```toml
# bole-fo2
bitflags = { version = "2", features = ["serde"] }
```

- [ ] **Step 3: Write the failing test**

Create `src/acl/clearance.rs` with only the test (the type does not exist yet):

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use super::Capability;

    #[test]
    fn capability_bit_ops() {
        let rw = Capability::READ | Capability::WRITE;
        assert!(rw.contains(Capability::READ));
        assert!(rw.contains(Capability::WRITE));
        assert!(Capability::READ.contains(Capability::READ));
        assert!(!Capability::READ.contains(Capability::WRITE));
        assert!(!Capability::WRITE.contains(Capability::READ));
    }
}
```

- [ ] **Step 4: Declare the module**

In `src/acl/mod.rs`, add `clearance` to the module declarations (after `pub mod backend;` at `src/acl/mod.rs:2`):

```rust
// bole-fo2
pub mod clearance;
```

- [ ] **Step 5: Run test to verify it fails**

Run: `cargo test -p bole acl::clearance::tests::capability_bit_ops`
Expected: FAIL — `cannot find type Capability in this scope`.

- [ ] **Step 6: Write the minimal implementation**

At the top of `src/acl/clearance.rs`, above the test module, add:

```rust
// bole-fo2
use serde::{Deserialize, Serialize};

// bole-fo2
bitflags::bitflags! {
    /// Orthogonal to the lattice position: a clearance can grant read, write,
    /// or both, independent of which label it is for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Capability: u8 {
        const READ = 0b01;
        const WRITE = 0b10;
    }
}
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test -p bole acl::clearance::tests::capability_bit_ops`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/acl/clearance.rs src/acl/mod.rs
git commit -m "bole-fo2: add bitflags dep and Capability bitflags"
```

---

### Task 2: `acl::lattice` — `Label` + `LabelLattice`

**Files:**
- Create: `src/acl/lattice.rs`
- Modify: `src/acl/mod.rs` (declare the `lattice` module)

**Interfaces:**
- Consumes: `crate::error::{Error, Result}`.
- Produces:
  - `acl::lattice::Label(pub String)` — derives `Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize`; `Label::PUBLIC: &str = "public"`; `Label::PROTECTED: &str = "protected"`; `Label::public() -> Label`; `Label::protected() -> Label`.
  - `acl::lattice::LabelLattice` — derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`; constructors `LabelLattice::new(labels: impl IntoIterator<Item = Label>, cover: impl IntoIterator<Item = (Label, Label)>) -> Self` and `LabelLattice::two_point() -> Self`; methods `dominates(&self, a: &Label, b: &Label) -> bool`, `strictly_dominates(&self, a: &Label, b: &Label) -> bool`, `join(&self, a: &Label, b: &Label) -> Label`, `meet(&self, a: &Label, b: &Label) -> Label`, `bottom(&self) -> Label`, `top(&self) -> Label`, `validate(&self) -> Result<()>`.

- [ ] **Step 1: Write the failing tests**

Create `src/acl/lattice.rs` with only the test module:

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use super::{Label, LabelLattice};

    fn l(s: &str) -> Label { Label(s.into()) }

    #[test]
    fn two_point_dominance_and_join() {
        let lat = LabelLattice::two_point();
        assert!(lat.dominates(&l("protected"), &l("public")));
        assert!(lat.dominates(&l("protected"), &l("protected")));
        assert!(!lat.dominates(&l("public"), &l("protected")));
        assert!(lat.strictly_dominates(&l("protected"), &l("public")));
        assert!(!lat.strictly_dominates(&l("protected"), &l("protected")));
        assert_eq!(lat.join(&l("public"), &l("protected")), l("protected"));
        assert_eq!(lat.meet(&l("public"), &l("protected")), l("public"));
        assert_eq!(lat.bottom(), l("public"));
        assert_eq!(lat.top(), l("protected"));
        lat.validate().unwrap();
    }

    #[test]
    fn diamond_join_and_meet() {
        // bottom ⊑ a, bottom ⊑ b, a ⊑ top, b ⊑ top; a and b incomparable.
        let lat = LabelLattice::new(
            [l("bottom"), l("a"), l("b"), l("top")],
            [
                (l("bottom"), l("a")),
                (l("bottom"), l("b")),
                (l("a"), l("top")),
                (l("b"), l("top")),
            ],
        );
        lat.validate().unwrap();
        // a and b are incomparable: neither dominates the other.
        assert!(!lat.dominates(&l("a"), &l("b")));
        assert!(!lat.dominates(&l("b"), &l("a")));
        // join of two incomparable elements is the top; meet is the bottom.
        assert_eq!(lat.join(&l("a"), &l("b")), l("top"));
        assert_eq!(lat.meet(&l("a"), &l("b")), l("bottom"));
        // transitive dominance across the diamond.
        assert!(lat.dominates(&l("top"), &l("bottom")));
        assert_eq!(lat.bottom(), l("bottom"));
        assert_eq!(lat.top(), l("top"));
    }

    #[test]
    fn validate_rejects_cycle() {
        let lat = LabelLattice::new(
            [l("a"), l("b")],
            [(l("a"), l("b")), (l("b"), l("a"))],
        );
        assert!(lat.validate().is_err());
    }

    #[test]
    fn validate_rejects_missing_top() {
        // Two maximal elements (a, b) and one bottom -> no unique top.
        let lat = LabelLattice::new(
            [l("bottom"), l("a"), l("b")],
            [(l("bottom"), l("a")), (l("bottom"), l("b"))],
        );
        assert!(lat.validate().is_err());
    }
}
```

- [ ] **Step 2: Declare the module**

In `src/acl/mod.rs`, add (next to the other `pub mod` lines):

```rust
// bole-fo2
pub mod lattice;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p bole acl::lattice`
Expected: FAIL — `cannot find type Label`/`LabelLattice`.

- [ ] **Step 4: Write the implementation**

At the top of `src/acl/lattice.rs`, above the test module, add:

```rust
// bole-fo2
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

// bole-fo2
/// An opaque confidentiality marker. The string is only an identity; all
/// ordering comes from the LabelLattice. `Label::PUBLIC` is the conventional
/// bottom (least restrictive) element.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Label(pub String);

impl Label {
    /// The conventional bottom (least restrictive) label.
    pub const PUBLIC: &str = "public";
    /// The conventional non-bottom label of the degenerate two-point lattice.
    pub const PROTECTED: &str = "protected";
    /// Constructs the bottom label.
    pub fn public() -> Label { Label(Self::PUBLIC.to_string()) }
    /// Constructs the two-point lattice's protected label.
    pub fn protected() -> Label { Label(Self::PROTECTED.to_string()) }
}

// bole-fo2
/// A bounded lattice: every pair has a unique join/meet and the order has a
/// unique bottom (`public`) and a unique top. Stored as the label set plus the
/// covering edges `(lower, higher)` meaning `lower ⊑ higher`. A content-addressed
/// policy object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelLattice {
    labels: BTreeSet<Label>,
    /// (lower, higher): `lower ⊑ higher`, i.e. higher dominates lower.
    cover: BTreeSet<(Label, Label)>,
}

impl LabelLattice {
    /// Builds a lattice from a label set and its covering edges.
    pub fn new(
        labels: impl IntoIterator<Item = Label>,
        cover: impl IntoIterator<Item = (Label, Label)>,
    ) -> Self {
        Self {
            labels: labels.into_iter().collect(),
            cover: cover.into_iter().collect(),
        }
    }

    /// The degenerate two-point lattice `{public ⊑ protected}` — the entire
    /// expressive content of today's PathAcl/TimelineAcl system.
    pub fn two_point() -> Self {
        Self::new(
            [Label::public(), Label::protected()],
            [(Label::public(), Label::protected())],
        )
    }

    /// `a ⊒ b` — a dominates (is at least as restrictive as) b. Reflexive.
    /// Computed by upward reachability over the cover edges from b to a.
    pub fn dominates(&self, a: &Label, b: &Label) -> bool {
        if a == b {
            return true;
        }
        let mut stack = vec![b.clone()];
        let mut seen: BTreeSet<Label> = BTreeSet::new();
        while let Some(cur) = stack.pop() {
            for (lo, hi) in &self.cover {
                if lo == &cur {
                    if hi == a {
                        return true;
                    }
                    if seen.insert(hi.clone()) {
                        stack.push(hi.clone());
                    }
                }
            }
        }
        false
    }

    /// `a ⊐ b` — strict domination: `dominates(a, b) && a != b`.
    pub fn strictly_dominates(&self, a: &Label, b: &Label) -> bool {
        a != b && self.dominates(a, b)
    }

    /// Least upper bound (most-protective common ceiling). Total: always exists.
    pub fn join(&self, a: &Label, b: &Label) -> Label {
        let ubs: Vec<Label> = self
            .labels
            .iter()
            .filter(|l| self.dominates(l, a) && self.dominates(l, b))
            .cloned()
            .collect();
        ubs.iter()
            .find(|cand| ubs.iter().all(|other| self.dominates(other, cand)))
            .cloned()
            .unwrap_or_else(|| self.top())
    }

    /// Greatest lower bound. Total: always exists.
    pub fn meet(&self, a: &Label, b: &Label) -> Label {
        let lbs: Vec<Label> = self
            .labels
            .iter()
            .filter(|l| self.dominates(a, l) && self.dominates(b, l))
            .cloned()
            .collect();
        lbs.iter()
            .find(|cand| lbs.iter().all(|other| self.dominates(cand, other)))
            .cloned()
            .unwrap_or_else(|| self.bottom())
    }

    /// The unique minimum (dominated by every label).
    pub fn bottom(&self) -> Label {
        self.labels
            .iter()
            .find(|l| self.labels.iter().all(|x| self.dominates(x, l)))
            .cloned()
            .unwrap_or_else(Label::public)
    }

    /// The unique maximum (dominates every label).
    pub fn top(&self) -> Label {
        self.labels
            .iter()
            .find(|l| self.labels.iter().all(|x| self.dominates(l, x)))
            .cloned()
            .unwrap_or_else(Label::public)
    }

    /// Rejects configs that are not bounded lattices: empty label sets, cover
    /// edges referencing unknown labels, cycles, missing/duplicate bottom or top,
    /// or any pair lacking a unique join or meet.
    pub fn validate(&self) -> Result<()> {
        if self.labels.is_empty() {
            return Err(Error::Storage("lattice has no labels".into()));
        }
        for (lo, hi) in &self.cover {
            if !self.labels.contains(lo) || !self.labels.contains(hi) {
                return Err(Error::Storage(
                    "cover edge references an unknown label".into(),
                ));
            }
        }
        for a in &self.labels {
            for b in &self.labels {
                if a != b && self.dominates(a, b) && self.dominates(b, a) {
                    return Err(Error::Storage(format!(
                        "cycle: {:?} and {:?} mutually dominate",
                        a, b
                    )));
                }
            }
        }
        let bottoms = self
            .labels
            .iter()
            .filter(|l| self.labels.iter().all(|x| self.dominates(x, l)))
            .count();
        if bottoms != 1 {
            return Err(Error::Storage(format!(
                "expected exactly one bottom, found {}",
                bottoms
            )));
        }
        let tops = self
            .labels
            .iter()
            .filter(|l| self.labels.iter().all(|x| self.dominates(l, x)))
            .count();
        if tops != 1 {
            return Err(Error::Storage(format!(
                "expected exactly one top, found {}",
                tops
            )));
        }
        for a in &self.labels {
            for b in &self.labels {
                let ubs: Vec<&Label> = self
                    .labels
                    .iter()
                    .filter(|l| self.dominates(l, a) && self.dominates(l, b))
                    .collect();
                let lubs = ubs
                    .iter()
                    .filter(|cand| ubs.iter().all(|o| self.dominates(o, cand)))
                    .count();
                if lubs != 1 {
                    return Err(Error::Storage(format!(
                        "pair {:?},{:?} lacks a unique join",
                        a, b
                    )));
                }
                let lbs: Vec<&Label> = self
                    .labels
                    .iter()
                    .filter(|l| self.dominates(a, l) && self.dominates(b, l))
                    .collect();
                let glbs = lbs
                    .iter()
                    .filter(|cand| lbs.iter().all(|o| self.dominates(cand, o)))
                    .count();
                if glbs != 1 {
                    return Err(Error::Storage(format!(
                        "pair {:?},{:?} lacks a unique meet",
                        a, b
                    )));
                }
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bole acl::lattice`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add src/acl/lattice.rs src/acl/mod.rs
git commit -m "bole-fo2: add Label and bounded LabelLattice"
```

---

### Task 3: `acl::rules` — `LabelRule` + `LabelRuleSet`

**Files:**
- Create: `src/acl/rules.rs`
- Modify: `src/acl/mod.rs` (declare the `rules` module)

**Interfaces:**
- Consumes: `acl::lattice::{Label, LabelLattice}`, `acl::glob::glob_matches`.
- Produces:
  - `acl::rules::LabelRule` — enum with variants `Path { glob: String, label: Label }`, `Timeline { pattern: String, label: Label }`, `Secret { name: String, label: Label }`; derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`.
  - `acl::rules::LabelRuleSet { pub rules: Vec<LabelRule> }` — derives `Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize`; methods `label_for_path(&self, lattice: &LabelLattice, path: &str) -> Label`, `label_for_timeline(&self, lattice: &LabelLattice, name: &str) -> Label`, `label_for_secret(&self, lattice: &LabelLattice, name: &str) -> Label` — each returns the JOIN of every matching rule's label, defaulting to `lattice.bottom()`.

- [ ] **Step 1: Write the failing tests**

Create `src/acl/rules.rs` with only the test module:

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use super::{LabelRule, LabelRuleSet};
    use crate::acl::lattice::{Label, LabelLattice};

    fn l(s: &str) -> Label { Label(s.into()) }

    #[test]
    fn default_is_bottom() {
        let lat = LabelLattice::two_point();
        let rs = LabelRuleSet::default();
        assert_eq!(rs.label_for_path(&lat, "src/main.rs"), lat.bottom());
        assert_eq!(rs.label_for_timeline(&lat, "main"), lat.bottom());
        assert_eq!(rs.label_for_secret(&lat, "DB_URL"), lat.bottom());
    }

    #[test]
    fn single_match_two_point() {
        let lat = LabelLattice::two_point();
        let rs = LabelRuleSet {
            rules: vec![LabelRule::Path { glob: "secrets/**".into(), label: l("protected") }],
        };
        assert_eq!(rs.label_for_path(&lat, "secrets/prod.key"), l("protected"));
        assert_eq!(rs.label_for_path(&lat, "src/main.rs"), l("public"));
    }

    #[test]
    fn multiple_match_takes_join() {
        // public ⊑ internal ⊑ secret chain.
        let lat = LabelLattice::new(
            [l("public"), l("internal"), l("secret")],
            [(l("public"), l("internal")), (l("internal"), l("secret"))],
        );
        let rs = LabelRuleSet {
            rules: vec![
                LabelRule::Path { glob: "secrets/**".into(), label: l("internal") },
                LabelRule::Path { glob: "secrets/prod/**".into(), label: l("secret") },
            ],
        };
        // both rules match -> JOIN(internal, secret) = secret.
        assert_eq!(rs.label_for_path(&lat, "secrets/prod/key"), l("secret"));
        // only the first matches -> internal.
        assert_eq!(rs.label_for_path(&lat, "secrets/dev/key"), l("internal"));
    }

    #[test]
    fn timeline_and_secret_rules() {
        let lat = LabelLattice::two_point();
        let rs = LabelRuleSet {
            rules: vec![
                LabelRule::Timeline { pattern: "leslie/private/**".into(), label: l("protected") },
                LabelRule::Secret { name: "DB_URL".into(), label: l("protected") },
            ],
        };
        assert_eq!(rs.label_for_timeline(&lat, "leslie/private/exp"), l("protected"));
        assert_eq!(rs.label_for_timeline(&lat, "main"), l("public"));
        assert_eq!(rs.label_for_secret(&lat, "DB_URL"), l("protected"));
        assert_eq!(rs.label_for_secret(&lat, "LOG_LEVEL"), l("public"));
    }
}
```

- [ ] **Step 2: Declare the module**

In `src/acl/mod.rs`, add:

```rust
// bole-fo2
pub mod rules;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p bole acl::rules`
Expected: FAIL — `cannot find type LabelRule`/`LabelRuleSet`.

- [ ] **Step 4: Write the implementation**

At the top of `src/acl/rules.rs`, above the test module, add:

```rust
// bole-fo2
use crate::acl::glob::glob_matches;
use crate::acl::lattice::{Label, LabelLattice};
use serde::{Deserialize, Serialize};

// bole-fo2
/// Assigns labels to resources by glob/name match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelRule {
    /// Any path matching `glob` carries `label`.
    Path { glob: String, label: Label },
    /// Any timeline whose name matches `pattern` carries `label`.
    Timeline { pattern: String, label: Label },
    /// Any secret matching `name` (env-var name or secret id) carries `label`.
    Secret { name: String, label: Label },
}

// bole-fo2
/// An ordered set of label rules. The effective label of a resource is the JOIN
/// of every matching rule of its kind, defaulting to the lattice bottom.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRuleSet {
    pub rules: Vec<LabelRule>,
}

impl LabelRuleSet {
    /// Effective label of a path = JOIN of every matching Path rule, default bottom.
    pub fn label_for_path(&self, lattice: &LabelLattice, path: &str) -> Label {
        let mut acc = lattice.bottom();
        for rule in &self.rules {
            if let LabelRule::Path { glob, label } = rule {
                if glob_matches(glob, path) {
                    acc = lattice.join(&acc, label);
                }
            }
        }
        acc
    }

    /// Effective label of a timeline = JOIN of every matching Timeline rule.
    pub fn label_for_timeline(&self, lattice: &LabelLattice, name: &str) -> Label {
        let mut acc = lattice.bottom();
        for rule in &self.rules {
            if let LabelRule::Timeline { pattern, label } = rule {
                if glob_matches(pattern, name) {
                    acc = lattice.join(&acc, label);
                }
            }
        }
        acc
    }

    /// Effective label of a secret = JOIN of every matching Secret rule.
    pub fn label_for_secret(&self, lattice: &LabelLattice, name: &str) -> Label {
        let mut acc = lattice.bottom();
        for rule in &self.rules {
            if let LabelRule::Secret { name: rname, label } = rule {
                if glob_matches(rname, name) {
                    acc = lattice.join(&acc, label);
                }
            }
        }
        acc
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bole acl::rules`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add src/acl/rules.rs src/acl/mod.rs
git commit -m "bole-fo2: add LabelRule and LabelRuleSet (JOIN composition)"
```

---

### Task 4: Finish `acl::clearance` — `ClearanceScope`, `Clearance`, `ClearanceSet`

**Files:**
- Modify: `src/acl/clearance.rs` (add the three types above the existing test module)

**Interfaces:**
- Consumes: `acl::lattice::Label`, `acl::clearance::Capability` (Task 1).
- Produces:
  - `acl::clearance::ClearanceScope` — enum `Path(String)`, `Timeline(String)`, `Secret(String)`; derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`.
  - `acl::clearance::Clearance { pub ceiling: Label, pub cap: Capability, pub scope: Option<ClearanceScope> }` — derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`.
  - `acl::clearance::ClearanceSet { pub clearances: Vec<Clearance>, pub confined: bool }` — derives `Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize`.

- [ ] **Step 1: Write the failing test**

In `src/acl/clearance.rs`, add a test to the existing `tests` module (alongside `capability_bit_ops`):

```rust
    // bole-fo2
    #[test]
    fn clearance_set_constructs_and_defaults() {
        use super::{Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::Label;

        let cs = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: Label::protected(),
                cap: Capability::READ | Capability::WRITE,
                scope: Some(ClearanceScope::Path("src/**".into())),
            }],
            confined: true,
        };
        assert_eq!(cs.clearances.len(), 1);
        assert!(cs.confined);
        assert_eq!(cs.clearances[0].ceiling, Label::protected());

        let empty = ClearanceSet::default();
        assert!(empty.clearances.is_empty());
        assert!(!empty.confined);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bole acl::clearance::tests::clearance_set_constructs_and_defaults`
Expected: FAIL — `cannot find type ClearanceSet`.

- [ ] **Step 3: Write the implementation**

In `src/acl/clearance.rs`, after the `bitflags!` block and before the test module, add:

```rust
// bole-fo2
use crate::acl::lattice::Label;

// bole-fo2
/// Optional resource scope on a clearance. Restricts which resources the
/// clearance applies to; it never widens the label bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClearanceScope {
    /// Path glob, matched with `acl::glob::glob_matches`.
    Path(String),
    /// Timeline pattern.
    Timeline(String),
    /// Secret name / id.
    Secret(String),
}

// bole-fo2
/// A single grant: "cleared up to `ceiling`, for these capabilities, optionally
/// only within `scope`." Downward-closed in the lattice. `None` scope applies to
/// every resource at/under `ceiling`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clearance {
    pub ceiling: Label,
    pub cap: Capability,
    pub scope: Option<ClearanceScope>,
}

// bole-fo2
/// What an actor holds: the union of the down-sets of its clearances, split by
/// capability and constrained by per-clearance scope. `confined` opts the whole
/// actor into the no-write-down rule (for untrusted agents). Default `false`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceSet {
    pub clearances: Vec<Clearance>,
    pub confined: bool,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p bole acl::clearance`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/acl/clearance.rs
git commit -m "bole-fo2: add ClearanceScope, Clearance, ClearanceSet"
```

---

### Task 5: `acl::mod` Accessor rewrite (scoped + confined evaluation, compat shims)

**Files:**
- Modify: `src/acl/mod.rs:67-147` (replace the `Accessor` struct and its impl; keep `Permission`, `PathRole`, `TimelineRole`, `PathAcl`, `TimelineAcl`, `AclStore` unchanged) and `src/acl/mod.rs:1-11` (imports)

**Interfaces:**
- Consumes: `acl::lattice::{Label, LabelLattice}`, `acl::rules::LabelRuleSet`, `acl::rules::LabelRule`, `acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet}`, `acl::glob::glob_matches`, `Permission`.
- Produces:
  - `acl::ResourceRef<'a>` — enum `Path(&'a str)`, `Timeline(&'a str)`, `Secret(&'a str)`; derives `Debug, Clone, Copy`.
  - `acl::Accessor` — fields `lattice: Arc<LabelLattice>`, `rules: Arc<LabelRuleSet>`, `clearances: ClearanceSet` (all private). Public methods: `new() -> Self`, `with_path_role(self, PathRole) -> Self`, `with_timeline_role(self, TimelineRole) -> Self`, `privileged() -> Self`, `from_parts(Arc<LabelLattice>, Arc<LabelRuleSet>, ClearanceSet) -> Self`, `can_read(&self, &Label, ResourceRef) -> bool`, `can_write(&self, &Label, ResourceRef) -> bool`, `can_read_path(&self, &str) -> bool`, `can_write_path(&self, &str) -> bool`, `can_read_timeline(&self, &str) -> bool`, `can_write_timeline(&self, &str) -> bool`, `can_read_secret(&self, &str) -> bool`. Derives `Debug, Clone`.
  - `impl From<Permission> for Capability`.

- [ ] **Step 1: Write the failing tests**

In `src/acl/mod.rs`, add these tests to the existing `tests` module (keep all current tests intact):

```rust
    // bole-fo2
    #[test]
    fn scoped_read_write_dominance_three_level() {
        use super::{Accessor, ResourceRef};
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use std::sync::Arc;

        let l = |s: &str| Label(s.into());
        let lat = Arc::new(LabelLattice::new(
            [l("public"), l("internal"), l("secret")],
            [(l("public"), l("internal")), (l("internal"), l("secret"))],
        ));
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: l("secret"),
                cap: Capability::READ | Capability::WRITE,
                scope: Some(ClearanceScope::Path("src/**".into())),
            }],
            confined: false,
        };
        let a = Accessor::from_parts(lat, Arc::new(LabelRuleSet::default()), clr);

        // In scope: read/write up to secret.
        assert!(a.can_read(&l("internal"), ResourceRef::Path("src/x")));
        assert!(a.can_write(&l("internal"), ResourceRef::Path("src/x")));
        assert!(a.can_read(&l("secret"), ResourceRef::Path("src/x")));
        // Out of scope: nothing.
        assert!(!a.can_read(&l("secret"), ResourceRef::Path("docs/x")));
        assert!(!a.can_write(&l("internal"), ResourceRef::Path("docs/x")));
        // Wrong resource kind never matches a path scope.
        assert!(!a.can_read(&l("public"), ResourceRef::Timeline("src/x")));
    }

    // bole-fo2
    #[test]
    fn confined_denies_write_down_allows_equal_and_incomparable() {
        use super::{Accessor, ResourceRef};
        use crate::acl::clearance::{Capability, Clearance, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use std::sync::Arc;

        let l = |s: &str| Label(s.into());
        // Diamond: bottom ⊑ {a, b} ⊑ top; a and b incomparable.
        let lat = Arc::new(LabelLattice::new(
            [l("bottom"), l("a"), l("b"), l("top")],
            [
                (l("bottom"), l("a")),
                (l("bottom"), l("b")),
                (l("a"), l("top")),
                (l("b"), l("top")),
            ],
        ));
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: l("a"),
                cap: Capability::READ | Capability::WRITE,
                scope: None,
            }],
            confined: true,
        };
        let agent = Accessor::from_parts(lat.clone(), Arc::new(LabelRuleSet::default()), clr);

        // Write-down (a strictly dominates bottom) -> denied for confined.
        assert!(!agent.can_write(&l("bottom"), ResourceRef::Path("x")));
        // Write-equal (a == a) -> allowed.
        assert!(agent.can_write(&l("a"), ResourceRef::Path("x")));
        // Write-incomparable (a vs b) -> base rule: a does not dominate b, so the
        // base dominance check already denies it.
        assert!(!agent.can_write(&l("b"), ResourceRef::Path("x")));
        // Reads are unaffected by confinement: can read everything a dominates.
        assert!(agent.can_read(&l("bottom"), ResourceRef::Path("x")));
        assert!(agent.can_read(&l("a"), ResourceRef::Path("x")));
    }

    // bole-fo2
    #[test]
    fn confined_allows_incomparable_when_clearance_covers_it() {
        use super::{Accessor, ResourceRef};
        use crate::acl::clearance::{Capability, Clearance, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use std::sync::Arc;

        let l = |s: &str| Label(s.into());
        let lat = Arc::new(LabelLattice::new(
            [l("bottom"), l("a"), l("b"), l("top")],
            [
                (l("bottom"), l("a")),
                (l("bottom"), l("b")),
                (l("a"), l("top")),
                (l("b"), l("top")),
            ],
        ));
        // Two write clearances at incomparable ceilings a and b.
        let clr = ClearanceSet {
            clearances: vec![
                Clearance { ceiling: l("a"), cap: Capability::WRITE, scope: None },
                Clearance { ceiling: l("b"), cap: Capability::WRITE, scope: None },
            ],
            confined: true,
        };
        let agent = Accessor::from_parts(lat, Arc::new(LabelRuleSet::default()), clr);
        // Target b: the b-clearance is equal (not strictly dominating), so the
        // confined no-write-down rule permits it (not every clearance writes down).
        assert!(agent.can_write(&l("b"), ResourceRef::Path("x")));
        // Target bottom: BOTH clearances strictly dominate it -> denied.
        assert!(!agent.can_write(&l("bottom"), ResourceRef::Path("x")));
    }
```

(The existing `tests` module already imports `use super::{Accessor, PathRole, Permission, TimelineRole};`; the backward-compat tests in that module — `empty_accessor_cannot_read_anything`, `matching_role_grants_read`, `write_role_does_not_grant_read`, `timeline_role_matching`, `read_role_does_not_grant_write`, `write_role_grants_write`, `roles_union_across_multiple_grants`, `privileged_accessor_can_read_everything` — are the backward-compat gate and must keep passing unchanged.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole acl::tests`
Expected: FAIL to compile — `from_parts`, `ResourceRef`, `can_read(&Label, _)` do not exist yet.

- [ ] **Step 3: Rewrite the imports**

Replace `src/acl/mod.rs:7-11`:

```rust
use crate::error::Result;
use backend::AclBackend;
use glob::glob_matches;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
```

with:

```rust
use crate::error::Result;
use backend::AclBackend;
use glob::glob_matches;
use serde::{Deserialize, Serialize};
// bole-fo2
use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
use crate::acl::lattice::{Label, LabelLattice};
use crate::acl::rules::{LabelRule, LabelRuleSet};
use std::sync::Arc;
```

(The `HashSet` import is dropped because the rewritten `Accessor` no longer stores role sets.)

- [ ] **Step 4: Replace the `Accessor` struct and impl**

Replace the whole block `src/acl/mod.rs:67-147` (the `Accessor` doc comment, struct, and `impl Accessor`) with:

```rust
// bole-fo2
/// Converts the legacy permission enum into the orthogonal capability bit.
impl From<Permission> for Capability {
    fn from(p: Permission) -> Self {
        match p {
            Permission::Read => Capability::READ,
            Permission::Write => Capability::WRITE,
        }
    }
}

// bole-fo2
/// Identifies the concrete resource being checked, for scope matching.
#[derive(Debug, Clone, Copy)]
pub enum ResourceRef<'a> {
    Path(&'a str),
    Timeline(&'a str),
    Secret(&'a str),
}

// bole-fo2
/// A scoped clearance *applies* to a resource iff its scope is absent or matches
/// the resource's kind and selector.
fn scope_applies(scope: &Option<ClearanceScope>, r: ResourceRef) -> bool {
    match (scope, r) {
        (None, _) => true,
        (Some(ClearanceScope::Path(g)), ResourceRef::Path(p)) => glob_matches(g, p),
        (Some(ClearanceScope::Timeline(g)), ResourceRef::Timeline(t)) => glob_matches(g, t),
        (Some(ClearanceScope::Secret(s)), ResourceRef::Secret(n)) => glob_matches(s, n),
        _ => false,
    }
}

// bole-fo2
/// The runtime credential and evaluator. Binds a label lattice, a rule set, and
/// an actor's clearance set, and answers `can_read`/`can_write` for a resource's
/// effective label and identity.
///
/// The legacy `new`/`with_path_role`/`with_timeline_role`/`privileged`
/// constructors lower natively into scoped clearances over the two-point lattice,
/// preserving the historical glob-ACL behaviour exactly.
#[derive(Debug, Clone)]
pub struct Accessor {
    lattice: Arc<LabelLattice>,
    rules: Arc<LabelRuleSet>,
    clearances: ClearanceSet,
}

impl Default for Accessor {
    fn default() -> Self {
        Self {
            lattice: Arc::new(LabelLattice::two_point()),
            rules: Arc::new(LabelRuleSet::default()),
            clearances: ClearanceSet::default(),
        }
    }
}

impl Accessor {
    /// Creates an empty `Accessor` over the default two-point lattice: no
    /// clearances, so it reads/writes nothing until granted a role.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an `Accessor` directly from a lattice, rule set, and clearance set
    /// (the native, label-aware path used by tests and the repository).
    pub fn from_parts(
        lattice: Arc<LabelLattice>,
        rules: Arc<LabelRuleSet>,
        clearances: ClearanceSet,
    ) -> Self {
        Self { lattice, rules, clearances }
    }

    /// Lowers a `PathRole` into a `protected`-rule + a path-scoped clearance.
    pub fn with_path_role(mut self, role: PathRole) -> Self {
        let rules = Arc::make_mut(&mut self.rules);
        rules.rules.push(LabelRule::Path {
            glob: role.glob.clone(),
            label: Label::protected(),
        });
        self.clearances.clearances.push(Clearance {
            ceiling: Label::protected(),
            cap: role.permission.into(),
            scope: Some(ClearanceScope::Path(role.glob)),
        });
        self
    }

    /// Lowers a `TimelineRole` into a `protected`-rule + a timeline-scoped clearance.
    pub fn with_timeline_role(mut self, role: TimelineRole) -> Self {
        let rules = Arc::make_mut(&mut self.rules);
        rules.rules.push(LabelRule::Timeline {
            pattern: role.pattern.clone(),
            label: Label::protected(),
        });
        self.clearances.clearances.push(Clearance {
            ceiling: Label::protected(),
            cap: role.permission.into(),
            scope: Some(ClearanceScope::Timeline(role.pattern)),
        });
        self
    }

    /// Read-everything, no write — the privileged() of today. A Read clearance
    /// with ceiling = lattice top and no scope.
    pub fn privileged() -> Self {
        let lattice = Arc::new(LabelLattice::two_point());
        let top = lattice.top();
        Self {
            lattice,
            rules: Arc::new(LabelRuleSet::default()),
            clearances: ClearanceSet {
                clearances: vec![Clearance { ceiling: top, cap: Capability::READ, scope: None }],
                confined: false,
            },
        }
    }

    /// Read iff some Read-capable, in-scope clearance's ceiling dominates `label`.
    pub fn can_read(&self, label: &Label, r: ResourceRef) -> bool {
        self.clearances.clearances.iter().any(|c| {
            c.cap.contains(Capability::READ)
                && scope_applies(&c.scope, r)
                && self.lattice.dominates(&c.ceiling, label)
        })
    }

    /// Write iff some Write-capable, in-scope clearance's ceiling dominates
    /// `label`; for confined actors, additionally forbid writing strictly down.
    pub fn can_write(&self, label: &Label, r: ResourceRef) -> bool {
        let dominated = self.clearances.clearances.iter().any(|c| {
            c.cap.contains(Capability::WRITE)
                && scope_applies(&c.scope, r)
                && self.lattice.dominates(&c.ceiling, label)
        });
        if !dominated {
            return false;
        }
        if self.clearances.confined {
            let writes_down = self.clearances.clearances.iter().all(|c| {
                !c.cap.contains(Capability::WRITE)
                    || self.lattice.strictly_dominates(&c.ceiling, label)
            });
            if writes_down {
                return false;
            }
        }
        true
    }

    /// True if this accessor may read `path` under its own rule set.
    pub fn can_read_path(&self, path: &str) -> bool {
        self.can_read(&self.rules.label_for_path(&self.lattice, path), ResourceRef::Path(path))
    }

    /// True if this accessor may write `path` under its own rule set.
    pub fn can_write_path(&self, path: &str) -> bool {
        self.can_write(&self.rules.label_for_path(&self.lattice, path), ResourceRef::Path(path))
    }

    /// True if this accessor may read timeline `name` under its own rule set.
    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.can_read(
            &self.rules.label_for_timeline(&self.lattice, name),
            ResourceRef::Timeline(name),
        )
    }

    /// True if this accessor may write timeline `name` under its own rule set.
    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.can_write(
            &self.rules.label_for_timeline(&self.lattice, name),
            ResourceRef::Timeline(name),
        )
    }

    /// New, for WS3's resolve_overlay: gate a secret by its Secret-rule label.
    pub fn can_read_secret(&self, name: &str) -> bool {
        self.can_read(
            &self.rules.label_for_secret(&self.lattice, name),
            ResourceRef::Secret(name),
        )
    }
}
```

- [ ] **Step 5: Run the full acl + repo suites to verify old and new pass**

Run: `cargo test -p bole acl`
Expected: PASS — all backward-compat `acl::tests` plus the three new tests.

Run: `cargo test -p bole repo`
Expected: PASS — the repo still compiles against the rewritten Accessor (`get_snapshot_filtered`/`advance_timeline` use the preserved `can_*_path`/`can_*_timeline` names).

- [ ] **Step 6: Commit**

```bash
git add src/acl/mod.rs
git commit -m "bole-fo2: rewrite Accessor over scoped clearances + lattice"
```

---

### Task 6: `Object::Policy` + `acl::policy_object`

**Files:**
- Create: `src/acl/policy_object.rs`
- Modify: `src/object/mod.rs:42-59` (add the `Policy` variant) and `src/object/mod.rs:32` (import `PolicyObject`)
- Modify: `src/acl/mod.rs` (declare the `policy_object` module)

**Interfaces:**
- Consumes: `acl::lattice::LabelLattice`, `acl::rules::LabelRuleSet`, `acl::clearance::ClearanceSet`, `object::ObjectId`, `store::ObjectStore`.
- Produces:
  - `acl::policy_object::PolicyObject` — enum `Lattice(LabelLattice)`, `RuleSet(LabelRuleSet)`, `Grant(ClearanceGrant)`, `Root(PolicyRoot)`; derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`.
  - `acl::policy_object::PolicyRoot { pub lattice: ObjectId, pub rules: ObjectId, pub parent: Option<ObjectId>, pub hooks: Vec<HookSpec> }`.
  - `acl::policy_object::ClearanceGrant { pub actor: String, pub clearances: ClearanceSet, pub signature: Option<Vec<u8>> }`.
  - `acl::policy_object::HookSpec { pub kind: String, pub pattern: String, pub params: BTreeMap<String, u64> }`.
  - `object::Object::Policy(PolicyObject)` variant.

- [ ] **Step 1: Write the failing test**

Create `src/acl/policy_object.rs` with only the test module:

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use super::{ClearanceGrant, HookSpec, PolicyObject, PolicyRoot};
    use crate::acl::clearance::{Capability, Clearance, ClearanceSet};
    use crate::acl::lattice::{Label, LabelLattice};
    use crate::acl::rules::{LabelRule, LabelRuleSet};
    use crate::object::{Object, ObjectId};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    async fn round_trip(store: &ObjectStore, obj: PolicyObject) -> (ObjectId, ObjectId) {
        let wrapped = Object::Policy(obj);
        let id1 = store.put(&wrapped).await.unwrap();
        let got = store.get(&id1).await.unwrap().unwrap();
        assert_eq!(got, wrapped);
        let id2 = store.put(&wrapped).await.unwrap();
        (id1, id2)
    }

    #[tokio::test]
    async fn policy_objects_round_trip_with_stable_ids() {
        let store = ObjectStore::new(MemoryBackend::new());

        let lattice = PolicyObject::Lattice(LabelLattice::two_point());
        let ruleset = PolicyObject::RuleSet(LabelRuleSet {
            rules: vec![LabelRule::Path { glob: "secrets/**".into(), label: Label::protected() }],
        });
        let grant = PolicyObject::Grant(ClearanceGrant {
            actor: "leslie".into(),
            clearances: ClearanceSet {
                clearances: vec![Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::READ,
                    scope: None,
                }],
                confined: false,
            },
            signature: None,
        });
        let root = PolicyObject::Root(PolicyRoot {
            lattice: ObjectId::new([1u8; 32]),
            rules: ObjectId::new([2u8; 32]),
            parent: None,
            hooks: vec![HookSpec {
                kind: "approval".into(),
                pattern: "release/**".into(),
                params: BTreeMap::from([("needed".to_string(), 2u64)]),
            }],
        });

        for obj in [lattice, ruleset, grant, root] {
            let (a, b) = round_trip(&store, obj).await;
            assert_eq!(a, b, "identical content must yield identical ObjectId");
        }
    }
}
```

- [ ] **Step 2: Declare the module**

In `src/acl/mod.rs`, add:

```rust
// bole-fo2
pub mod policy_object;
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p bole acl::policy_object`
Expected: FAIL — `cannot find type PolicyObject`, and `Object::Policy` does not exist.

- [ ] **Step 4: Write the policy object types**

At the top of `src/acl/policy_object.rs`, above the test module, add:

```rust
// bole-fo2
use crate::acl::clearance::ClearanceSet;
use crate::acl::lattice::LabelLattice;
use crate::acl::rules::LabelRuleSet;
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// bole-fo2
/// A declarative binding of a `PolicyHook` to a resource pattern, resolved by the
/// registry to a hook instance. Kept as data so policy stays content-addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub kind: String,
    pub pattern: String,
    pub params: BTreeMap<String, u64>,
}

// bole-fo2
/// The root tying a policy generation together; what a policy ref points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRoot {
    pub lattice: ObjectId,
    pub rules: ObjectId,
    pub parent: Option<ObjectId>,
    pub hooks: Vec<HookSpec>,
}

// bole-fo2
/// An issued, optionally-signed clearance credential for one actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceGrant {
    pub actor: String,
    pub clearances: ClearanceSet,
    pub signature: Option<Vec<u8>>,
}

// bole-fo2
/// Content-addressed policy payload. Each kind is independently addressed so a
/// replica can have/want lattice and ruleset separately during pack negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyObject {
    Lattice(LabelLattice),
    RuleSet(LabelRuleSet),
    Grant(ClearanceGrant),
    Root(PolicyRoot),
}
```

- [ ] **Step 5: Add the `Object::Policy` variant**

In `src/object/mod.rs`, add to the imports near `src/object/mod.rs:32` (after `use serde::{Deserialize, Serialize};`):

```rust
// bole-fo2
use crate::acl::policy_object::PolicyObject;
```

Then, in the `Object` enum (after the `EnvOverlay(EnvOverlay)` variant at `src/object/mod.rs:57`), add:

```rust
    // bole-fo2
    /// A content-addressed access-policy payload (lattice, rules, grant, or root).
    Policy(PolicyObject),
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p bole acl::policy_object`
Expected: PASS.

- [ ] **Step 7: Run the codec/object suites to confirm no regression**

Run: `cargo test -p bole object codec store`
Expected: PASS (the new variant rides the single `postcard` codec path).

- [ ] **Step 8: Commit**

```bash
git add src/acl/policy_object.rs src/acl/mod.rs src/object/mod.rs
git commit -m "bole-fo2: add PolicyObject and Object::Policy variant"
```

---

### Task 7: `AclBackend` extension + `AclStore` projections

**Files:**
- Modify: `src/acl/backend.rs` (add default-implemented lattice/ruleset/grant methods)
- Modify: `src/acl/mod.rs` (add `AclStore::lattice`/`label_ruleset`/`grant` wrappers)
- Modify: `src/acl/backend.rs` test section (new tests) — or add tests inline in `backend.rs`

**Interfaces:**
- Consumes: `acl::lattice::{Label, LabelLattice}`, `acl::rules::{LabelRule, LabelRuleSet}`, `acl::policy_object::ClearanceGrant`, and the existing `list_path_acls`/`list_timeline_acls`.
- Produces (new default methods on `trait AclBackend`):
  - `fn get_lattice(&self) -> Result<LabelLattice>` (default: `two_point()`).
  - `fn set_lattice(&self, _l: &LabelLattice) -> Result<()>` (default: `Ok(())`).
  - `fn get_label_ruleset(&self) -> Result<LabelRuleSet>` (default: derived two-point rules from path/timeline ACLs).
  - `fn set_label_ruleset(&self, _r: &LabelRuleSet) -> Result<()>` (default: `Ok(())`).
  - `fn get_grant(&self, _actor: &str) -> Result<Option<ClearanceGrant>>` (default: `Ok(None)`).
  - `fn set_grant(&self, _g: &ClearanceGrant) -> Result<()>` (default: `Ok(())`).
- Produces (new `AclStore` methods): `fn lattice(&self) -> Result<LabelLattice>`, `fn label_ruleset(&self) -> Result<LabelRuleSet>`, `fn grant(&self, actor: &str) -> Result<Option<ClearanceGrant>>`.

- [ ] **Step 1: Write the failing tests**

In `src/acl/backend.rs`, add a test module at the bottom:

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use crate::acl::backend::AclBackend;
    use crate::acl::lattice::Label;
    use crate::acl::memory::MemoryAclBackend;
    use crate::acl::rules::LabelRule;
    use crate::acl::PathAcl;

    #[test]
    fn set_path_acl_lowers_to_two_point_rule() {
        let b = MemoryAclBackend::new();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        let rs = b.get_label_ruleset().unwrap();
        assert!(rs.rules.iter().any(|r| matches!(
            r,
            LabelRule::Path { glob, label }
                if glob == "secrets/**" && *label == Label::protected()
        )));
    }

    #[test]
    fn default_lattice_is_two_point() {
        let b = MemoryAclBackend::new();
        let lat = b.get_lattice().unwrap();
        assert_eq!(lat.bottom(), Label::public());
        assert_eq!(lat.top(), Label::protected());
    }
}
```

Also, in `src/acl/disk.rs`, add a test confirming pre-existing on-disk ACL files still project into the ruleset (append to the existing `tests` module):

```rust
    // bole-fo2
    #[test]
    fn old_disk_acls_project_into_two_point_ruleset() {
        use crate::acl::backend::AclBackend;
        use crate::acl::lattice::Label;
        use crate::acl::rules::LabelRule;
        let dir = TempDir::new().unwrap();
        {
            let b = DiskAclBackend::open(dir.path()).unwrap();
            b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
            b.set_timeline_acl(&TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();
        }
        let b2 = DiskAclBackend::open(dir.path()).unwrap();
        let rs = b2.get_label_ruleset().unwrap();
        assert!(rs.rules.iter().any(|r| matches!(
            r, LabelRule::Path { glob, label } if glob == "secrets/**" && *label == Label::protected()
        )));
        assert!(rs.rules.iter().any(|r| matches!(
            r, LabelRule::Timeline { pattern, label }
                if pattern == "leslie/private/**" && *label == Label::protected()
        )));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bole acl::backend acl::disk`
Expected: FAIL — `get_label_ruleset`/`get_lattice` not found.

- [ ] **Step 3: Extend the `AclBackend` trait**

Replace the contents of `src/acl/backend.rs:1-15` (imports + trait) with:

```rust
// bole-mhs
use crate::error::Result;
use crate::acl::{PathAcl, TimelineAcl};
// bole-fo2
use crate::acl::lattice::{Label, LabelLattice};
use crate::acl::policy_object::ClearanceGrant;
use crate::acl::rules::{LabelRule, LabelRuleSet};

pub trait AclBackend: Send + Sync {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>>;
    fn set_path_acl(&self, acl: &PathAcl) -> Result<()>;
    fn delete_path_acl(&self, glob: &str) -> Result<()>;
    fn list_path_acls(&self) -> Result<Vec<PathAcl>>;

    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>>;
    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()>;
    fn delete_timeline_acl(&self, pattern: &str) -> Result<()>;
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>>;

    // bole-fo2
    /// The active label lattice. Defaults to the degenerate two-point lattice.
    fn get_lattice(&self) -> Result<LabelLattice> {
        Ok(LabelLattice::two_point())
    }
    // bole-fo2
    /// Persist a new lattice. Default no-op (two-point backends derive it).
    fn set_lattice(&self, _lattice: &LabelLattice) -> Result<()> {
        Ok(())
    }
    // bole-fo2
    /// The active rule set, derived by default from the existing path/timeline
    /// ACLs as two-point `protected` rules.
    fn get_label_ruleset(&self) -> Result<LabelRuleSet> {
        let mut rules = Vec::new();
        for a in self.list_path_acls()? {
            rules.push(LabelRule::Path { glob: a.glob, label: Label::protected() });
        }
        for a in self.list_timeline_acls()? {
            rules.push(LabelRule::Timeline { pattern: a.pattern, label: Label::protected() });
        }
        Ok(LabelRuleSet { rules })
    }
    // bole-fo2
    /// Persist a new rule set. Default no-op (two-point backends derive it).
    fn set_label_ruleset(&self, _rules: &LabelRuleSet) -> Result<()> {
        Ok(())
    }
    // bole-fo2
    /// Look up an actor's issued clearance grant. Default: none stored yet.
    fn get_grant(&self, _actor: &str) -> Result<Option<ClearanceGrant>> {
        Ok(None)
    }
    // bole-fo2
    /// Persist an actor's clearance grant. Default no-op.
    fn set_grant(&self, _grant: &ClearanceGrant) -> Result<()> {
        Ok(())
    }
}
```

- [ ] **Step 4: Add `AclStore` wrapper methods**

In `src/acl/mod.rs`, inside `impl AclStore` (after `timeline_is_protected` at `src/acl/mod.rs:198`), add:

```rust
    // bole-fo2
    /// Returns the active label lattice (default two-point).
    pub fn lattice(&self) -> Result<LabelLattice> {
        self.backend.get_lattice()
    }
    // bole-fo2
    /// Returns the active label rule set (derived from path/timeline ACLs).
    pub fn label_ruleset(&self) -> Result<LabelRuleSet> {
        self.backend.get_label_ruleset()
    }
    // bole-fo2
    /// Returns an actor's stored clearance grant, if any.
    pub fn grant(&self, actor: &str) -> Result<Option<crate::acl::policy_object::ClearanceGrant>> {
        self.backend.get_grant(actor)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bole acl::backend acl::disk acl::memory`
Expected: PASS — new tests plus all existing memory/disk ACL tests (old on-disk files still load).

- [ ] **Step 6: Commit**

```bash
git add src/acl/backend.rs src/acl/mod.rs src/acl/disk.rs
git commit -m "bole-fo2: extend AclBackend with lattice/ruleset/grant shims"
```

---

### Task 8: `acl::hook` — PolicyHook trait, registry, TimelinePolicyHook

**Files:**
- Create: `src/acl/hook.rs`
- Modify: `src/acl/mod.rs` (declare the `hook` module)

**Interfaces:**
- Consumes: `Accessor`, `object::{Object, ObjectId}`, `store::ObjectStore`, `refs::{RefName, RefStore, TimelinePolicy}`, `error::Result`, `async_trait`.
- Produces:
  - `acl::hook::PolicyDecision` — enum `Allow`, `Deny(String)`, `RequiresApproval { reason: String, needed: u32 }`; derives `Debug, Clone, PartialEq, Eq`.
  - `acl::hook::PolicyEvent<'a>` — enum `Advance { timeline: &'a RefName, old_head: ObjectId, new_head: ObjectId }`, `Merge { source: &'a RefName, target: &'a RefName, old_head: ObjectId, result_head: ObjectId }`.
  - `acl::hook::PolicyContext<'a>` — fields `event: PolicyEvent<'a>`, `accessor: &'a Accessor`, `objects: &'a ObjectStore`, `refs: &'a RefStore`, `now: u64`.
  - `acl::hook::PolicyHook` — `#[async_trait]` trait: `fn name(&self) -> &str`, `async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision`.
  - `acl::hook::PolicyRegistry` — `new() -> Self` (preloads `TimelinePolicyHook`), `push(&mut self, Box<dyn PolicyHook>)`, `async fn evaluate(&self, ctx: &PolicyContext<'_>) -> PolicyDecision` (most restrictive).
  - `acl::hook::TimelinePolicyHook` (unit struct, implements `PolicyHook`).

> **Spec refinement (noted):** the WS1 spec sketches `PolicyHook::check` as synchronous, but reproducing the timeline fast-forward rule requires async snapshot-ancestry walks (`ObjectStore::get` is `async`). We make `check` async via `async_trait` (already a crate dependency). This is the single deviation from the spec's abbreviated signature.

- [ ] **Step 1: Write the failing tests**

Create `src/acl/hook.rs` with only the test module:

```rust
// bole-fo2
#[cfg(test)]
mod tests {
    use super::{PolicyContext, PolicyDecision, PolicyEvent, PolicyHook, PolicyRegistry, TimelinePolicyHook};
    use crate::acl::Accessor;
    use crate::object::Snapshot;
    use crate::refs::{RefName, RefStore, TimelinePolicy};
    use crate::refs::memory::MemoryRefBackend;
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    struct DenyAll;
    #[async_trait::async_trait]
    impl PolicyHook for DenyAll {
        fn name(&self) -> &str { "deny-all" }
        async fn check(&self, _ctx: &PolicyContext<'_>) -> PolicyDecision {
            PolicyDecision::Deny("nope".into())
        }
    }

    #[tokio::test]
    async fn most_restrictive_composition() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let accessor = Accessor::privileged();
        let ctx = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: base, new_head: base },
            accessor: &accessor,
            objects: &objects,
            refs: &refs,
            now: 0,
        };

        // Registry with only the built-in TimelinePolicyHook on an Unrestricted
        // timeline -> Allow.
        let reg = PolicyRegistry::new();
        assert_eq!(reg.evaluate(&ctx).await, PolicyDecision::Allow);

        // Add a DenyAll hook -> most restrictive wins -> Deny.
        let mut reg2 = PolicyRegistry::new();
        reg2.push(Box::new(DenyAll));
        assert!(matches!(reg2.evaluate(&ctx).await, PolicyDecision::Deny(_)));
    }

    #[tokio::test]
    async fn timeline_policy_hook_fast_forward() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let child = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into(),
        }).await.unwrap();
        let sibling = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 2, message: "s".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        refs.create_timeline(name.clone(), base, TimelinePolicy::FastForwardOnly, 0, "persistent".into(), None).unwrap();

        let accessor = Accessor::privileged();
        let hook = TimelinePolicyHook;

        // base -> child is a fast-forward -> Allow.
        let ctx_ff = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: base, new_head: child },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert_eq!(hook.check(&ctx_ff).await, PolicyDecision::Allow);

        // child -> sibling is NOT a fast-forward (sibling is not a descendant of child) -> Deny.
        // (timeline head is base here, but the hook tests old_head -> new_head ancestry.)
        let ctx_non = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: child, new_head: sibling },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert!(matches!(hook.check(&ctx_non).await, PolicyDecision::Deny(_)));
    }
}
```

- [ ] **Step 2: Declare the module**

In `src/acl/mod.rs`, add:

```rust
// bole-fo2
pub mod hook;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p bole acl::hook`
Expected: FAIL — `cannot find type PolicyHook`/`PolicyRegistry`/`TimelinePolicyHook`.

- [ ] **Step 4: Write the implementation**

At the top of `src/acl/hook.rs`, above the test module, add:

```rust
// bole-fo2
use crate::acl::Accessor;
use crate::error::Result;
use crate::object::{Object, ObjectId};
use crate::refs::{RefName, RefStore, TimelinePolicy};
use crate::store::ObjectStore;
use std::collections::BTreeSet;

// bole-fo2
/// The outcome of a hook check. Hooks may only further restrict access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequiresApproval { reason: String, needed: u32 },
}

// bole-fo2
/// The decision point being evaluated.
pub enum PolicyEvent<'a> {
    Advance {
        timeline: &'a RefName,
        old_head: ObjectId,
        new_head: ObjectId,
    },
    Merge {
        source: &'a RefName,
        target: &'a RefName,
        old_head: ObjectId,
        result_head: ObjectId,
    },
}

// bole-fo2
/// The read-only context handed to a hook.
pub struct PolicyContext<'a> {
    pub event: PolicyEvent<'a>,
    pub accessor: &'a Accessor,
    pub objects: &'a ObjectStore,
    pub refs: &'a RefStore,
    pub now: u64,
}

// bole-fo2
/// A predicate evaluated at a write decision point, for rules the label lattice
/// cannot express. Hooks run after the label check passes and may only deny.
#[async_trait::async_trait]
pub trait PolicyHook: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision;
}

// bole-fo2
/// Returns the more restrictive of two decisions (Deny > RequiresApproval > Allow).
fn more_restrictive(a: PolicyDecision, b: PolicyDecision) -> PolicyDecision {
    fn rank(d: &PolicyDecision) -> u8 {
        match d {
            PolicyDecision::Allow => 0,
            PolicyDecision::RequiresApproval { .. } => 1,
            PolicyDecision::Deny(_) => 2,
        }
    }
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

// bole-fo2
/// True if `ancestor` is `descendant` or an ancestor of it in the snapshot DAG.
async fn is_ancestor(
    objects: &ObjectStore,
    ancestor: ObjectId,
    descendant: ObjectId,
) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }
    let mut stack = vec![descendant];
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    while let Some(cur) = stack.pop() {
        if let Some(Object::Snapshot(s)) = objects.get(&cur).await? {
            for p in s.parents {
                if p == ancestor {
                    return Ok(true);
                }
                if seen.insert(p) {
                    stack.push(p);
                }
            }
        }
    }
    Ok(false)
}

// bole-fo2
/// Holds the bound hooks; the effective decision is the most restrictive across
/// all of them. Always preloads the built-in `TimelinePolicyHook`.
pub struct PolicyRegistry {
    hooks: Vec<Box<dyn PolicyHook>>,
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self { hooks: vec![Box::new(TimelinePolicyHook)] }
    }

    pub fn push(&mut self, hook: Box<dyn PolicyHook>) {
        self.hooks.push(hook);
    }

    pub async fn evaluate(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        let mut decision = PolicyDecision::Allow;
        for hook in &self.hooks {
            decision = more_restrictive(decision, hook.check(ctx).await);
        }
        decision
    }
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// bole-fo2
/// The built-in hook reproducing today's `TimelinePolicy` enforcement: on an
/// `Advance`, `FastForwardOnly`/`Append` require the new head to descend from the
/// old head; `Unrestricted` always allows. Non-advance events are allowed.
pub struct TimelinePolicyHook;

#[async_trait::async_trait]
impl PolicyHook for TimelinePolicyHook {
    fn name(&self) -> &str {
        "timeline-policy"
    }

    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        if let PolicyEvent::Advance { timeline, old_head, new_head } = &ctx.event {
            let tl = match ctx.refs.get_timeline(timeline) {
                Ok(Some(tl)) => tl,
                Ok(None) => {
                    return PolicyDecision::Deny(format!(
                        "timeline '{}' not found",
                        timeline.as_str()
                    ))
                }
                Err(e) => {
                    return PolicyDecision::Deny(format!("timeline lookup failed: {e}"))
                }
            };
            match tl.policy {
                TimelinePolicy::Unrestricted => PolicyDecision::Allow,
                TimelinePolicy::FastForwardOnly | TimelinePolicy::Append => {
                    match is_ancestor(ctx.objects, *old_head, *new_head).await {
                        Ok(true) => PolicyDecision::Allow,
                        Ok(false) => PolicyDecision::Deny(format!(
                            "timeline '{}' has policy {:?}; new head {} is not a descendant of current head {}",
                            timeline.as_str(),
                            tl.policy,
                            new_head,
                            old_head
                        )),
                        Err(e) => {
                            PolicyDecision::Deny(format!("ancestry check failed: {e}"))
                        }
                    }
                }
            }
        } else {
            PolicyDecision::Allow
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bole acl::hook`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add src/acl/hook.rs src/acl/mod.rs
git commit -m "bole-fo2: add PolicyHook, PolicyRegistry, TimelinePolicyHook"
```

---

### Task 9: `repo::mod` integration — label-native filtering, merge scan, and registry-driven advance

**Files:**
- Modify: `src/repo/mod.rs:17` (imports), `src/repo/mod.rs:136-159` (`get_snapshot_filtered`), `src/repo/mod.rs:161-172` (`list_refs_filtered`), `src/repo/mod.rs:184-223` (`check_merge`), `src/repo/mod.rs:282-352` (`advance_timeline`), `src/repo/mod.rs:426-461` (`walk_tree_filtered`)

**Interfaces:**
- Consumes: `acl::lattice::LabelLattice`, `acl::rules::LabelRuleSet`, `acl::{Accessor, ResourceRef}`, `acl::hook::{PolicyContext, PolicyDecision, PolicyEvent, PolicyRegistry}`, `AclStore::{lattice, label_ruleset}` (Task 7).
- Produces:
  - Reworked `walk_tree_filtered(objects, lattice, rules, tree_id, prefix, accessor, out)` — a path is visible when its effective label is the lattice bottom, or the accessor `can_read` it.
  - `Repository::policy_registry(&self) -> PolicyRegistry` (returns `PolicyRegistry::new()` in this task; extended in Task 10).
  - Unchanged public signatures for `get_snapshot_filtered`, `list_refs_filtered`, `check_merge`, `advance_timeline`.

- [ ] **Step 1: Run the existing repo suite to capture the green baseline**

Run: `cargo test -p bole repo && cargo test -p bole --test acl`
Expected: PASS (this is the regression gate; every test here must remain green after the rewrite).

- [ ] **Step 2: Update imports**

In `src/repo/mod.rs`, replace the acl import at `src/repo/mod.rs:17`:

```rust
use crate::acl::{Accessor, AclStore, PathAcl, PathRole, Permission};
```

with:

```rust
use crate::acl::{Accessor, AclStore, PathAcl, PathRole, Permission};
// bole-fo2
use crate::acl::ResourceRef;
use crate::acl::hook::{PolicyContext, PolicyDecision, PolicyEvent, PolicyRegistry};
use crate::acl::lattice::LabelLattice;
use crate::acl::rules::LabelRuleSet;
```

- [ ] **Step 3: Add the `policy_registry` helper**

In `src/repo/mod.rs`, inside `impl Repository`, add (immediately after the `disk` constructor at `src/repo/mod.rs:119`):

```rust
    // bole-fo2
    /// Builds the active policy registry. Always includes the built-in
    /// `TimelinePolicyHook`; declarative hooks are added in a later task.
    fn policy_registry(&self) -> PolicyRegistry {
        PolicyRegistry::new()
    }
```

- [ ] **Step 4: Rewrite `walk_tree_filtered`**

Replace the whole `walk_tree_filtered` function (`src/repo/mod.rs:426-461`) with:

```rust
// bole-fo2
// Visible iff the path's effective label is the lattice bottom (public — visible
// to all) or the accessor's clearances dominate it in scope. This collapses the
// old `path_is_protected` gate into the dominance check.
async fn walk_tree_filtered(
    objects: &ObjectStore,
    lattice: &LabelLattice,
    rules: &LabelRuleSet,
    tree_id: ObjectId,
    prefix: &str,
    accessor: &Accessor,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    let tree = match objects.get(&tree_id).await? {
        Some(Object::Tree(t)) => t,
        _ => return Ok(()),
    };
    for (name, entry) in &tree.entries {
        let full_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        match entry.kind {
            EntryKind::Blob => {
                let label = rules.label_for_path(lattice, &full_path);
                if label == lattice.bottom()
                    || accessor.can_read(&label, ResourceRef::Path(&full_path))
                {
                    out.insert(full_path, entry.id);
                }
            }
            EntryKind::Tree => {
                Box::pin(walk_tree_filtered(
                    objects, lattice, rules, entry.id, &full_path, accessor, out,
                ))
                .await?;
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Update `get_snapshot_filtered`**

In `get_snapshot_filtered`, replace the `walk_tree_filtered` call (`src/repo/mod.rs:151`):

```rust
        walk_tree_filtered(&self.objects, &self.acls, snap.root, "", accessor, &mut visible_paths).await?;
```

with:

```rust
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        walk_tree_filtered(&self.objects, &lattice, &rules, snap.root, "", accessor, &mut visible_paths).await?;
```

- [ ] **Step 6: Rewrite `list_refs_filtered`**

Replace the body of `list_refs_filtered` (`src/repo/mod.rs:161-172`) with:

```rust
    pub fn list_refs_filtered(&self, prefix: &str, accessor: &Accessor) -> Result<Vec<RefName>> {
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        let all = self.refs.list(prefix)?;
        let mut out = Vec::new();
        for name in all {
            let label = rules.label_for_timeline(&lattice, name.as_str());
            if label == lattice.bottom()
                || accessor.can_read(&label, ResourceRef::Timeline(name.as_str()))
            {
                out.push(name);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 7: Rewrite the `check_merge` leak scan**

In `check_merge`, replace the block from the `walk_tree_filtered` call through the leak loop (`src/repo/mod.rs:203-215`):

```rust
        walk_tree_filtered(&self.objects, &self.acls, source_tree, "", &Accessor::privileged(), &mut visible).await?;
        // bole-l55
        // Find all paths in source that are protected but dest doesn't enforce them
        let dest_is_protected = self.acls.timeline_is_protected(dest.as_str())?;
        let mut leaking: Vec<PathAcl> = Vec::new();
        let path_acls = self.acls.list_path_acls()?;
        for acl in &path_acls {
            let any_match = visible.keys().any(|p| crate::acl::glob::glob_matches(&acl.glob, p));
            if any_match && !dest_is_protected && !leaking.iter().any(|l| l.glob == acl.glob) {
                leaking.push(acl.clone());
            }
        }
```

with:

```rust
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        walk_tree_filtered(&self.objects, &lattice, &rules, source_tree, "", &Accessor::privileged(), &mut visible).await?;
        // bole-fo2
        // A path leaks if its effective label is NOT dominated by the dest
        // timeline's effective label — i.e. content would flow to a strictly
        // less-protected place. Reported as the matching PathAcl(s), as before.
        let dest_label = rules.label_for_timeline(&lattice, dest.as_str());
        let mut leaking: Vec<PathAcl> = Vec::new();
        let path_acls = self.acls.list_path_acls()?;
        for acl in &path_acls {
            let any_leak = visible.keys().any(|p| {
                crate::acl::glob::glob_matches(&acl.glob, p)
                    && {
                        let plabel = rules.label_for_path(&lattice, p);
                        !lattice.dominates(&dest_label, &plabel)
                    }
            });
            if any_leak && !leaking.iter().any(|l| l.glob == acl.glob) {
                leaking.push(acl.clone());
            }
        }
```

- [ ] **Step 8: Rewrite `advance_timeline`**

Replace the body of `advance_timeline` (`src/repo/mod.rs:282-352`) with:

```rust
    pub async fn advance_timeline(
        &self,
        name: &RefName,
        snapshot_id: ObjectId,
        accessor: &Accessor,
    ) -> Result<()> {
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        // Timeline write check via the dominance rule.
        let tl_label = rules.label_for_timeline(&lattice, name.as_str());
        if !accessor.can_write(&tl_label, ResourceRef::Timeline(name.as_str())) {
            return Err(Error::AccessDenied(format!(
                "write denied on timeline: {}",
                name.as_str()
            )));
        }
        let snap = match self.objects.get(&snapshot_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Err(Error::Storage(format!("snapshot not found: {}", snapshot_id))),
        };
        // bole-fo2
        // Per-path write check (also enforces the confined no-write-down rule).
        let mut paths = BTreeMap::new();
        walk_tree_filtered(
            &self.objects,
            &lattice,
            &rules,
            snap.root,
            "",
            &Accessor::privileged(),
            &mut paths,
        )
        .await?;
        for path in paths.keys() {
            let label = rules.label_for_path(&lattice, path);
            if !accessor.can_write(&label, ResourceRef::Path(path)) {
                return Err(Error::AccessDenied(format!("write denied on path: {}", path)));
            }
        }
        // bole-fo2
        // The inline TimelinePolicy match is replaced by the policy registry.
        let timeline = self.refs.get_timeline(name)?.ok_or_else(|| {
            Error::Storage(format!("timeline not found: {}", name.as_str()))
        })?;
        let registry = self.policy_registry();
        let ctx = PolicyContext {
            event: PolicyEvent::Advance {
                timeline: name,
                old_head: timeline.head,
                new_head: snapshot_id,
            },
            accessor,
            objects: &self.objects,
            refs: &self.refs,
            now: 0,
        };
        match registry.evaluate(&ctx).await {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny(reason) => return Err(Error::PolicyViolation(reason)),
            PolicyDecision::RequiresApproval { reason, .. } => {
                return Err(Error::PolicyViolation(reason))
            }
        }
        self.refs.advance_head(name, snapshot_id)?;
        Ok(())
    }
```

- [ ] **Step 9: Add the confined-agent regression test**

In the `src/repo/mod.rs` `tests` module, add:

```rust
    // bole-fo2
    #[tokio::test]
    async fn confined_agent_cannot_advance_declassifying_snapshot() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::Label;
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::{Accessor, PathAcl};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let repo = Repository::memory();
        // The repo protects secrets/**; everything else is public (bottom).
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        // Snapshot writes a PUBLIC path (declassifying target for a confined,
        // protected-cleared agent).
        let blob = repo.objects.put_blob(Bytes::from("leak")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("public/notes.md".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "base".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let next = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "next".into(),
        }).await.unwrap();

        // A confined agent cleared to WRITE at `protected`, scoped to everything.
        let lattice = Arc::new(crate::acl::lattice::LabelLattice::two_point());
        let clr = ClearanceSet {
            clearances: vec![
                Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::WRITE,
                    scope: Some(ClearanceScope::Path("**".into())),
                },
                Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::WRITE,
                    scope: Some(ClearanceScope::Timeline("**".into())),
                },
            ],
            confined: true,
        };
        let agent = Accessor::from_parts(lattice, Arc::new(LabelRuleSet::default()), clr);

        // Writing the public path strictly below `protected` is a declassifying
        // write -> denied before the head moves.
        let err = repo.advance_timeline(&name, next, &agent).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)), "got {err:?}");
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, base);
    }
```

- [ ] **Step 10: Run the whole bole suite to verify green**

Run: `cargo test -p bole`
Expected: PASS — all prior repo/acl tests plus the new confined-agent test. (In particular `filtered_snapshot_hides_protected_path`, `t3_path_filtering`, `t3_merge_check`, `list_refs_filtered_hides_protected_timeline`, `advance_timeline_requires_write_cap_on_paths`, and the `fast_forward_only_*`/`append_*`/`unrestricted_*` policy tests must remain green.)

- [ ] **Step 11: Commit**

```bash
git add src/repo/mod.rs
git commit -m "bole-fo2: route repo ops through lattice labels + policy registry"
```

---

### Task 10: `ApprovalHook` + HookSpec resolution + repo Merge hooks

**Files:**
- Modify: `src/acl/hook.rs` (add `ApprovalHook`, `resolve_hook`, and approval helpers)
- Modify: `src/repo/mod.rs` (add a `hooks` field, `register_hook`, make `policy_registry` resolve specs, and run Merge hooks in `check_merge`)

**Interfaces:**
- Consumes: `acl::policy_object::HookSpec`, `acl::glob::glob_matches`, `refs::RefStore`, the `PolicyHook` trait.
- Produces:
  - `acl::hook::ApprovalHook { pattern: String, needed: u32 }` implementing `PolicyHook` (checks `Merge` events against the target pattern; counts approval refs).
  - `acl::hook::resolve_hook(spec: &HookSpec) -> Result<Box<dyn PolicyHook>>` — fail-closed: unknown `kind` returns `Err(Error::PolicyViolation(..))`.
  - `acl::hook::approval_ref_prefix(target: &str) -> String` and `acl::hook::count_approvals(refs: &RefStore, target: &str) -> Result<u32>`.
  - `Repository::register_hook(&mut self, spec: HookSpec)` and a `policy_registry(&self) -> Result<PolicyRegistry>` that resolves `self.hooks`.

> **Spec note (O4 — deferred attestation format):** approvals are recorded as a placeholder: refs under `refs/approval/<target>/<approver>`. `count_approvals` tallies them. The signed-attestation format is WS5's job (O4); this minimal tally is sufficient for the worked example and is replaceable without touching the hook interface.

- [ ] **Step 1: Write the failing test**

In `src/repo/mod.rs` `tests` module, add:

```rust
    // bole-fo2
    #[tokio::test]
    async fn merge_into_release_requires_two_approvals() {
        use crate::acl::hook::approval_ref_prefix;
        use crate::acl::policy_object::HookSpec;
        use crate::acl::{Accessor, TimelineRole, Permission};
        use crate::object::{EntryKind, ObjectId, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use crate::MergeCheck;
        use std::collections::{BTreeMap, BTreeSet};

        let mut repo = Repository::memory();
        repo.register_hook(HookSpec {
            kind: "approval".into(),
            pattern: "release/**".into(),
            params: BTreeMap::from([("needed".to_string(), 2u64)]),
        });

        // A clean source (no protected paths) merging into release/1.0.
        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/lib.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into(),
        }).await.unwrap();
        let source = RefName::new("feature/x").unwrap();
        let dest = RefName::new("release/1.0").unwrap();
        repo.refs.create_timeline(source.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(dest.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let writer = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "release/**".into(), permission: Permission::Write });

        // No approvals yet -> RequiresApproval.
        let r1 = repo.check_merge(&source, &dest, &writer).await.unwrap();
        assert!(matches!(r1, MergeCheck::RequiresApproval(_)), "got {r1:?}");

        // Record two approval refs, then -> Allowed.
        let prefix = approval_ref_prefix(dest.as_str());
        for approver in ["alice", "bob"] {
            let rn = RefName::new(format!("{prefix}{approver}")).unwrap();
            repo.refs.create_tag(rn, ObjectId::new([7u8; 32]), None, 0).unwrap();
        }
        let _ = BTreeSet::<u8>::new();
        let r2 = repo.check_merge(&source, &dest, &writer).await.unwrap();
        assert_eq!(r2, MergeCheck::Allowed);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bole repo::tests::merge_into_release_requires_two_approvals`
Expected: FAIL — `register_hook`, `approval_ref_prefix` not found.

- [ ] **Step 3: Add the approval helpers and hook in `acl::hook`**

In `src/acl/hook.rs`, update the imports block to add `Error`, `glob_matches`, `HookSpec`, and `RefName` usage (replace `use crate::error::Result;` with):

```rust
// bole-fo2
use crate::error::{Error, Result};
// bole-fo2
use crate::acl::glob::glob_matches;
use crate::acl::policy_object::HookSpec;
```

Then, after the `TimelinePolicyHook` impl (and before the test module), add:

```rust
// bole-fo2
/// The ref-namespace prefix under which approval attestations for `target` live.
/// Placeholder storage (O4): real signed attestations are WS5's job.
pub fn approval_ref_prefix(target: &str) -> String {
    format!("refs/approval/{}/", target)
}

// bole-fo2
/// Counts recorded approval refs for `target`.
pub fn count_approvals(refs: &RefStore, target: &str) -> Result<u32> {
    let prefix = approval_ref_prefix(target);
    let n = refs.list(&prefix)?.len();
    Ok(n as u32)
}

// bole-fo2
/// "Merges into `<pattern>` need `needed` approvals." Checks `Merge` events whose
/// target matches `pattern`; blocks until enough approval refs exist.
pub struct ApprovalHook {
    pub pattern: String,
    pub needed: u32,
}

#[async_trait::async_trait]
impl PolicyHook for ApprovalHook {
    fn name(&self) -> &str {
        "approval"
    }

    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        if let PolicyEvent::Merge { target, .. } = &ctx.event {
            if glob_matches(&self.pattern, target.as_str()) {
                let approvals = match count_approvals(ctx.refs, target.as_str()) {
                    Ok(n) => n,
                    Err(e) => return PolicyDecision::Deny(format!("approval lookup failed: {e}")),
                };
                if approvals < self.needed {
                    return PolicyDecision::RequiresApproval {
                        reason: format!(
                            "{} needs {} approvals, has {}",
                            target.as_str(),
                            self.needed,
                            approvals
                        ),
                        needed: self.needed - approvals,
                    };
                }
            }
        }
        PolicyDecision::Allow
    }
}

// bole-fo2
/// Resolves a declarative `HookSpec` into a hook instance. Fail-closed: an
/// unknown `kind` is rejected rather than silently skipped (decision O5).
pub fn resolve_hook(spec: &HookSpec) -> Result<Box<dyn PolicyHook>> {
    match spec.kind.as_str() {
        "approval" => {
            let needed = *spec.params.get("needed").unwrap_or(&1) as u32;
            Ok(Box::new(ApprovalHook {
                pattern: spec.pattern.clone(),
                needed,
            }))
        }
        "timeline-policy" => Ok(Box::new(TimelinePolicyHook)),
        other => Err(Error::PolicyViolation(format!(
            "unknown policy hook kind '{}' (fail-closed)",
            other
        ))),
    }
}
```

- [ ] **Step 4: Add a registry-composition unit test for `ApprovalHook`**

In `src/acl/hook.rs` `tests` module, add:

```rust
    // bole-fo2
    #[tokio::test]
    async fn approval_hook_blocks_then_allows() {
        use super::{approval_ref_prefix, ApprovalHook};
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let source = RefName::new("feature/x").unwrap();
        let target = RefName::new("release/1.0").unwrap();
        let accessor = Accessor::privileged();
        let head = objects.put_tree(BTreeMap::new()).await.unwrap();

        let hook = ApprovalHook { pattern: "release/**".into(), needed: 1 };
        let ctx = PolicyContext {
            event: PolicyEvent::Merge { source: &source, target: &target, old_head: head, result_head: head },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert!(matches!(hook.check(&ctx).await, PolicyDecision::RequiresApproval { .. }));

        let rn = RefName::new(format!("{}alice", approval_ref_prefix(target.as_str()))).unwrap();
        refs.create_tag(rn, head, None, 0).unwrap();
        assert_eq!(hook.check(&ctx).await, PolicyDecision::Allow);
    }
```

- [ ] **Step 5: Wire hooks into the repository**

In `src/repo/mod.rs`, add the `hooks` field to the `Repository` struct (after the `acls` field at `src/repo/mod.rs:92`):

```rust
    // bole-fo2
    /// Declarative policy hook bindings resolved into the registry per call.
    hooks: Vec<crate::acl::policy_object::HookSpec>,
```

Add `hooks: Vec::new()` to the struct literal in `memory()` (after `acls: AclStore::new(MemoryAclBackend::new()),` at `src/repo/mod.rs:106`) and in `disk()` (after `acls: AclStore::new(DiskAclBackend::open(root)?),` at `src/repo/mod.rs:118`):

```rust
            // bole-fo2
            hooks: Vec::new(),
```

Add the registration method and replace `policy_registry` (the helper added in Task 9). Replace:

```rust
    // bole-fo2
    /// Builds the active policy registry. Always includes the built-in
    /// `TimelinePolicyHook`; declarative hooks are added in a later task.
    fn policy_registry(&self) -> PolicyRegistry {
        PolicyRegistry::new()
    }
```

with:

```rust
    // bole-fo2
    /// Registers a declarative policy hook binding.
    pub fn register_hook(&mut self, spec: crate::acl::policy_object::HookSpec) {
        self.hooks.push(spec);
    }

    // bole-fo2
    /// Builds the active policy registry: the built-in `TimelinePolicyHook` plus
    /// every resolved declarative hook (fail-closed on unknown kinds).
    fn policy_registry(&self) -> Result<PolicyRegistry> {
        let mut reg = PolicyRegistry::new();
        for spec in &self.hooks {
            reg.push(crate::acl::hook::resolve_hook(spec)?);
        }
        Ok(reg)
    }
```

Update the `advance_timeline` call site (Task 9 added `let registry = self.policy_registry();`) to:

```rust
        let registry = self.policy_registry()?;
```

- [ ] **Step 6: Run Merge hooks in `check_merge`**

In `check_merge`, replace the decision tail (`src/repo/mod.rs`, the `if leaking.is_empty() { … } else if … { RequiresApproval } else { Rejected }` block) with:

```rust
        // bole-fo2
        // After the leak scan, run registered Merge hooks (most restrictive wins).
        let registry = self.policy_registry()?;
        let ctx = PolicyContext {
            event: PolicyEvent::Merge {
                source,
                target: dest,
                old_head: source_head,
                result_head: source_head,
            },
            accessor,
            objects: &self.objects,
            refs: &self.refs,
            now: 0,
        };
        let hook_decision = registry.evaluate(&ctx).await;

        if let PolicyDecision::Deny(_) = hook_decision {
            return Ok(MergeCheck::Rejected(leaking));
        }
        if leaking.is_empty() {
            match hook_decision {
                PolicyDecision::RequiresApproval { .. } => Ok(MergeCheck::RequiresApproval(leaking)),
                _ => Ok(MergeCheck::Allowed),
            }
        } else if accessor.can_write_timeline(dest.as_str()) {
            Ok(MergeCheck::RequiresApproval(leaking))
        } else {
            Ok(MergeCheck::Rejected(leaking))
        }
    }
```

(`source_head` is bound earlier in `check_merge`; this reuses it as the `Merge` event's head, since `check_merge` is a pre-merge check that has no computed result head — see the Task notes.)

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p bole acl::hook repo::tests::merge_into_release_requires_two_approvals`
Expected: PASS.

Run: `cargo test -p bole`
Expected: PASS — `t3_merge_check` (no hooks registered there) still returns `RequiresApproval`/`Rejected`/`Allowed` exactly as before, because an empty `hooks` list yields a registry whose only hook (`TimelinePolicyHook`) returns `Allow` for `Merge` events.

- [ ] **Step 8: Commit**

```bash
git add src/acl/hook.rs src/repo/mod.rs
git commit -m "bole-fo2: add ApprovalHook, fail-closed hook resolution, merge hooks"
```

---

### Task 11: Public exports, error note, and full-workspace verification

**Files:**
- Modify: `src/lib.rs:46-49` (re-export the new public policy types)
- Modify: `src/error.rs:39-41` (extend the `PolicyViolation` doc to note hook reuse)

**Interfaces:**
- Consumes: every public type produced by Tasks 1–10.
- Produces: top-level re-exports so downstream crates (`bole-cli`, WS3/WS5) reach the new surface without deep paths.

- [ ] **Step 1: Extend the `acl` re-exports**

In `src/lib.rs`, replace the `acl` re-export block (`src/lib.rs:46-49`):

```rust
pub mod acl;
pub use acl::{
    Accessor, AclStore, PathAcl, PathRole, Permission, TimelineAcl, TimelineRole,
};
```

with:

```rust
pub mod acl;
pub use acl::{
    Accessor, AclStore, PathAcl, PathRole, Permission, TimelineAcl, TimelineRole,
};
// bole-fo2
pub use acl::ResourceRef;
pub use acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
pub use acl::lattice::{Label, LabelLattice};
pub use acl::rules::{LabelRule, LabelRuleSet};
pub use acl::policy_object::{ClearanceGrant, HookSpec, PolicyObject, PolicyRoot};
pub use acl::hook::{
    ApprovalHook, PolicyContext, PolicyDecision, PolicyEvent, PolicyHook, PolicyRegistry,
    TimelinePolicyHook,
};
```

- [ ] **Step 2: Add the `Error::PolicyViolation` reuse note**

In `src/error.rs`, replace the `PolicyViolation` doc comment (`src/error.rs:39-41`):

```rust
    // bole-3w9
    /// A timeline advance was rejected because it violates the timeline's [`TimelinePolicy`](crate::refs::TimelinePolicy).
    #[error("policy violation: {0}")] PolicyViolation(String),
```

with:

```rust
    // bole-3w9
    // bole-fo2
    /// A policy decision rejected the operation. Raised by the timeline's
    /// [`TimelinePolicy`](crate::refs::TimelinePolicy) and, under WS1, by any
    /// `PolicyHook` returning `Deny` or `RequiresApproval` from
    /// `advance_timeline`, and by fail-closed resolution of an unknown hook kind.
    #[error("policy violation: {0}")] PolicyViolation(String),
```

- [ ] **Step 3: Write the export smoke test**

Create `tests/ws1_exports.rs`:

```rust
// bole-fo2
use bole::{
    Capability, Clearance, ClearanceScope, ClearanceSet, HookSpec, Label, LabelLattice, LabelRule,
    LabelRuleSet, PolicyObject, PolicyRoot, ResourceRef,
};
use std::collections::BTreeMap;

#[test]
fn ws1_public_types_are_reachable() {
    let lat = LabelLattice::two_point();
    assert_eq!(lat.bottom(), Label::public());
    let _rs = LabelRuleSet {
        rules: vec![LabelRule::Path { glob: "secrets/**".into(), label: Label::protected() }],
    };
    let _cs = ClearanceSet {
        clearances: vec![Clearance {
            ceiling: Label::protected(),
            cap: Capability::READ | Capability::WRITE,
            scope: Some(ClearanceScope::Path("src/**".into())),
        }],
        confined: true,
    };
    let _r = ResourceRef::Path("src/x");
    let _po = PolicyObject::Root(PolicyRoot {
        lattice: bole::object::ObjectId::new([0u8; 32]),
        rules: bole::object::ObjectId::new([1u8; 32]),
        parent: None,
        hooks: vec![HookSpec { kind: "approval".into(), pattern: "release/**".into(), params: BTreeMap::new() }],
    });
}
```

- [ ] **Step 4: Run the export test**

Run: `cargo test -p bole --test ws1_exports`
Expected: PASS.

- [ ] **Step 5: Run the WHOLE workspace suite**

Run: `cargo test --workspace`
Expected: PASS — at least the original 247 tests plus all WS1 tests added across Tasks 1–11. Confirm the summary shows zero failures across the `bole` lib, its integration tests, and `bole-cli`.

- [ ] **Step 6: Confirm no warnings from the new code**

Run: `cargo build --workspace 2>&1 | grep -i "warning" || echo "no warnings"`
Expected: `no warnings` (or only pre-existing, unrelated warnings).

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/error.rs tests/ws1_exports.rs
git commit -m "bole-fo2: export WS1 policy types and document PolicyViolation reuse"
```

---

## Self-Review

**1. Spec coverage** — every spec section maps to a task:

| Spec section | Task |
|--------------|------|
| §2 module layout (`lattice`, `rules`, `clearance`, `mod`, `policy_object`, `hook`, `backend`) | 1–10 |
| §3.1 `Label`, `LabelLattice` (dominates/strictly_dominates/join/meet/bottom/top/validate, two-point) | 2 |
| §3.2 `LabelRule` (Path/Timeline/Secret), `LabelRuleSet`, JOIN composition | 3 |
| §3.3 `Capability`, `ClearanceScope`, `Clearance`, `ClearanceSet { confined }` | 1, 4 |
| §3.4 `Object::Policy`, `PolicyObject`, `PolicyRoot`, `ClearanceGrant`, `HookSpec` | 6 |
| §3.5 `AclBackend` extension + `AclStore` projection shims | 7 |
| §4.1/§4.2 read rule, write rule (base + confined), `ResourceRef`, `scope_applies` | 5 |
| §4.3 Accessor `can_*_path`/`can_*_timeline`/`can_read_secret`/`privileged` | 5 |
| §5.1–§5.3 `PolicyHook`, `PolicyEvent`, `PolicyContext`, `PolicyDecision`, registry, `TimelinePolicyHook` | 8 |
| §5.4 `ApprovalHook`, `HookSpec` resolution, worked release/** example | 10 |
| §6 `get_snapshot_filtered`, `list_refs_filtered`, `check_merge`, `advance_timeline` re-expression | 9, 10 |
| §6 `compute_workspace_view` / `can_read_secret` seam | 5 (additive method); behaviour preserved (no secret rules ⇒ public) — verified by existing `compute_workspace_view_resolves_env` staying green in Task 9 |
| §7 backward compat (two-point lowering, native roles, on-disk migration) | 5, 7 |
| §9 testing strategy (lattice algebra, two-point equivalence, read/write/confined/scoped, effective label, hooks, policy objects, migration) | 2–10 |
| §10 resolved decisions O1/O3/O6/#4 | 2 (O3), 4–5 (O1, O6), 3/5 (#4) |
| §11 open O2/O4/O5 | O2: PolicyViolation reuse (10, 11); O4: placeholder approval refs (10); O5: fail-closed `resolve_hook` (10) |

No gap required an extra task beyond the 11 in the decomposition. The §6 `compute_workspace_view` secret-gating seam is the one place WS1 only adds an additive method (`can_read_secret`) and leaves behaviour unchanged until WS3 supplies secret rules; this is folded into Task 5 (method) with the no-op-by-default guarantee verified by the existing workspace tests in Task 9, rather than spun into its own task.

**2. Placeholder scan** — no `TBD`/`TODO`/"similar to Task N"/"add error handling" remain; every code step shows complete Rust (full type defs and method bodies). The only deferred items are explicitly the spec's own open questions (O4 attestation format), implemented as a documented placeholder, not a plan placeholder.

**3. Type consistency** — cross-task names verified: `Capability::{READ,WRITE}`, `Label::{public,protected,PUBLIC,PROTECTED}`, `LabelLattice::{two_point,dominates,strictly_dominates,bottom,top}`, `LabelRuleSet::{label_for_path,label_for_timeline,label_for_secret}`, `ClearanceSet::{clearances,confined}`, `Accessor::{from_parts,can_read,can_write,can_*_path,can_*_timeline,can_read_secret}`, `ResourceRef::{Path,Timeline,Secret}`, `PolicyObject::{Lattice,RuleSet,Grant,Root}`, `HookSpec::{kind,pattern,params}`, `PolicyDecision::{Allow,Deny,RequiresApproval}`, `PolicyEvent::{Advance,Merge}`, `PolicyRegistry::{new,push,evaluate}`, `resolve_hook`, `approval_ref_prefix`/`count_approvals`, `Repository::{register_hook,policy_registry}` — all used with identical signatures where consumed. `walk_tree_filtered`'s new `(objects, lattice, rules, …)` arity is updated at every call site (`get_snapshot_filtered`, `check_merge`, `advance_timeline`) in Task 9.
