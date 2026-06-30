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
}
