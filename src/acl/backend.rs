// bole-mhs
use crate::error::Result;
use crate::acl::{PathAcl, TimelineAcl};
// bole-fo2
use crate::acl::lattice::{Label, LabelLattice};
use crate::acl::policy_object::ClearanceGrant;
use crate::acl::rules::{LabelRule, LabelRuleSet};
// bole-9mz
use crate::acl::SecretAcl;

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
        // bole-9mz
        for a in self.list_secret_acls()? {
            rules.push(LabelRule::Secret { name: a.name, label: Label::protected() });
        }
        Ok(LabelRuleSet { rules })
    }

    // bole-9mz
    /// Secret-protection rules. Default empty (no secret is protected). Backends
    /// that persist secret ACLs override these.
    fn list_secret_acls(&self) -> Result<Vec<SecretAcl>> {
        Ok(Vec::new())
    }
    // bole-9mz
    /// Adds or replaces a secret-protection rule. Default no-op.
    fn set_secret_acl(&self, _acl: &SecretAcl) -> Result<()> {
        Ok(())
    }
    // bole-9mz
    /// Removes a secret-protection rule by name. Default no-op.
    fn delete_secret_acl(&self, _name: &str) -> Result<()> {
        Ok(())
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
