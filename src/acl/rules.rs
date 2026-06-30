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
