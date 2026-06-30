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
