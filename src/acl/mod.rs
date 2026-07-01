// bole-mhs
pub mod backend;
// bole-fo2
pub mod clearance;
pub mod disk;
// bole-fo2
pub mod lattice;
// bole-fo2
pub mod rules;
// bole-fo2
pub mod policy_object;
// bole-fo2
pub mod hook;
// bole-0tp
pub mod authority;
// bole-fz1
pub mod attestation;
pub mod glob;
pub mod memory;

use crate::error::Result;
use backend::AclBackend;
use glob::glob_matches;
use serde::{Deserialize, Serialize};
// bole-fo2
use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
use crate::acl::lattice::{Label, LabelLattice};
use crate::acl::rules::{LabelRule, LabelRuleSet};
use std::sync::Arc;

// bole-p8u
/// Whether a role grants read or write access.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    // bole-p8u
    /// Permits read-only access.
    Read,
    // bole-p8u
    /// Permits mutation (implies write but not necessarily read).
    Write,
}

// bole-p8u
/// A grant of a specific permission over all paths matching a glob pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PathRole {
    // bole-p8u
    /// The glob pattern that selects which paths this role applies to.
    pub glob: String,
    // bole-p8u
    /// The level of access granted on matching paths.
    pub permission: Permission,
}

// bole-p8u
/// A grant of a specific permission over all timelines whose name matches a glob pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRole {
    // bole-p8u
    /// The glob pattern that selects which timeline names this role applies to.
    pub pattern: String,
    // bole-p8u
    /// The level of access granted on matching timelines.
    pub permission: Permission,
}

// bole-p8u
/// A path-protection rule: any path matching `glob` requires an explicit `PathRole` to access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathAcl {
    // bole-p8u
    /// The glob pattern that identifies protected paths.
    pub glob: String,
}

// bole-p8u
/// A timeline-protection rule: any timeline matching `pattern` requires an explicit `TimelineRole`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineAcl {
    // bole-p8u
    /// The glob pattern that identifies protected timelines.
    pub pattern: String,
}

// bole-9mz
/// A secret-protection rule: any secret whose env-var name matches `name`
/// carries the `protected` label, so resolving it requires clearance (WS1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretAcl {
    /// The glob pattern that identifies protected secret names.
    pub name: String,
}

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

// bole-7rn
/// Renders a clearance scope for a decision trace: `None` (any) or the
/// kind-tagged glob it is restricted to.
fn render_scope(scope: &Option<ClearanceScope>) -> Option<String> {
    scope.as_ref().map(|s| match s {
        ClearanceScope::Path(g) => format!("path:{g}"),
        ClearanceScope::Timeline(g) => format!("timeline:{g}"),
        ClearanceScope::Secret(g) => format!("secret:{g}"),
    })
}

// bole-7rn
/// The evaluation of a single clearance against one capability request, as it
/// contributes to a [`CapabilityTrace`]. Every field mirrors a term the
/// enforcement logic in [`Accessor::can_read`]/[`Accessor::can_write`] tests.
#[derive(Debug, Clone)]
pub struct ClearanceEval {
    /// The clearance's ceiling label.
    pub ceiling: Label,
    /// The clearance's scope, rendered as `kind:glob`, or `None` for any resource.
    pub scope: Option<String>,
    /// Whether the clearance's scope covers the resource under evaluation.
    pub scope_applies: bool,
    /// Whether the clearance carries the requested capability bit.
    pub grants_capability: bool,
    /// Whether the ceiling dominates the resource's effective label.
    pub dominates: bool,
    /// Whether the ceiling *strictly* dominates the label (relevant to no-write-down).
    pub strictly_dominates: bool,
    /// Whether this clearance is the one that granted access (`false` on a denial).
    pub decisive: bool,
}

// bole-7rn
/// The trace of a single capability decision (read *or* write) for one
/// resource: the verdict plus the per-clearance evaluation that produced it.
#[derive(Debug, Clone)]
pub struct CapabilityTrace {
    /// The final verdict for this capability at the accessor level (before any
    /// repo-level public short-circuit).
    pub allowed: bool,
    /// Set when a dominating write clearance existed but the confined
    /// no-write-down rule refused the write.
    pub confined_write_down_block: bool,
    /// One entry per clearance the accessor holds.
    pub clearances: Vec<ClearanceEval>,
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

    // bole-7rn
    /// Evaluates one capability against `label`/`r` and produces a full trace:
    /// how each held clearance fared and which one (if any) was decisive. The
    /// verdict here is exactly [`can_read`]/[`can_write`]'s, term for term —
    /// this method is the same logic instrumented, not a re-implementation.
    fn eval_cap(&self, label: &Label, r: ResourceRef, cap: Capability) -> CapabilityTrace {
        let mut evals: Vec<ClearanceEval> = self
            .clearances
            .clearances
            .iter()
            .map(|c| ClearanceEval {
                ceiling: c.ceiling.clone(),
                scope: render_scope(&c.scope),
                scope_applies: scope_applies(&c.scope, r),
                grants_capability: c.cap.contains(cap),
                dominates: self.lattice.dominates(&c.ceiling, label),
                strictly_dominates: self.lattice.strictly_dominates(&c.ceiling, label),
                decisive: false,
            })
            .collect();

        // A clearance qualifies iff it carries the capability, is in scope, and
        // its ceiling dominates the label — the three conjuncts of `can_*`.
        let dominated = evals
            .iter()
            .any(|e| e.grants_capability && e.scope_applies && e.dominates);

        let mut confined_write_down_block = false;
        let allowed = if !dominated {
            false
        } else if cap.contains(Capability::WRITE) && self.clearances.confined {
            // Confined no-write-down: every write clearance must strictly dominate.
            let writes_down = self
                .clearances
                .clearances
                .iter()
                .all(|c| !c.cap.contains(Capability::WRITE) || self.lattice.strictly_dominates(&c.ceiling, label));
            if writes_down {
                confined_write_down_block = true;
                false
            } else {
                true
            }
        } else {
            true
        };

        // Flag the first qualifying clearance as decisive when access is granted.
        if allowed {
            if let Some(e) = evals
                .iter_mut()
                .find(|e| e.grants_capability && e.scope_applies && e.dominates)
            {
                e.decisive = true;
            }
        }

        CapabilityTrace { allowed, confined_write_down_block, clearances: evals }
    }

    // bole-7rn
    /// Produces the `(read, write)` capability traces for `label`/`r`. Callers
    /// that also apply a repo-level public short-circuit (e.g. filtered views)
    /// layer that on top of the read trace.
    pub fn explain(&self, label: &Label, r: ResourceRef) -> (CapabilityTrace, CapabilityTrace) {
        (
            self.eval_cap(label, r, Capability::READ),
            self.eval_cap(label, r, Capability::WRITE),
        )
    }
}

// bole-mhs
// bole-p8u
/// Persistent store for [`PathAcl`] and [`TimelineAcl`] rules.
///
/// `AclStore` lets administrators declare which paths and timelines are
/// protected, independently of the `Accessor` credentials held by individual
/// users.  Repository operations consult the `AclStore` to decide whether an
/// access check is even needed for a given path or timeline.
pub struct AclStore {
    backend: Box<dyn AclBackend>,
}

impl AclStore {
    // bole-p8u
    /// Creates an `AclStore` backed by the given [`AclBackend`].
    pub fn new(backend: impl AclBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    // bole-p8u
    /// Adds or replaces the path ACL rule described by `acl`.
    pub fn set_path_acl(&self, acl: PathAcl) -> Result<()> { self.backend.set_path_acl(&acl) }
    // bole-p8u
    /// Removes the path ACL rule whose glob equals `glob`.
    pub fn remove_path_acl(&self, glob: &str) -> Result<()> { self.backend.delete_path_acl(glob) }
    // bole-p8u
    /// Returns all registered path ACL rules.
    pub fn list_path_acls(&self) -> Result<Vec<PathAcl>> { self.backend.list_path_acls() }

    // bole-p8u
    /// Adds or replaces the timeline ACL rule described by `acl`.
    pub fn set_timeline_acl(&self, acl: TimelineAcl) -> Result<()> { self.backend.set_timeline_acl(&acl) }
    // bole-p8u
    /// Removes the timeline ACL rule whose pattern equals `pattern`.
    pub fn remove_timeline_acl(&self, pattern: &str) -> Result<()> { self.backend.delete_timeline_acl(pattern) }
    // bole-p8u
    /// Returns all registered timeline ACL rules.
    pub fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> { self.backend.list_timeline_acls() }

    // bole-p8u
    /// Returns `true` if any registered path ACL rule matches `path`.
    pub fn path_is_protected(&self, path: &str) -> Result<bool> {
        Ok(self.backend.list_path_acls()?.iter().any(|a| glob_matches(&a.glob, path)))
    }

    // bole-p8u
    /// Returns `true` if any registered timeline ACL rule matches `name`.
    pub fn timeline_is_protected(&self, name: &str) -> Result<bool> {
        Ok(self.backend.list_timeline_acls()?.iter().any(|a| glob_matches(&a.pattern, name)))
    }

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

    // bole-9mz
    /// Adds or replaces the secret ACL described by `acl`.
    pub fn set_secret_acl(&self, acl: SecretAcl) -> Result<()> {
        self.backend.set_secret_acl(&acl)
    }
    // bole-9mz
    /// Removes the secret ACL whose name equals `name`.
    pub fn remove_secret_acl(&self, name: &str) -> Result<()> {
        self.backend.delete_secret_acl(name)
    }
    // bole-9mz
    /// Returns all registered secret ACL rules.
    pub fn list_secret_acls(&self) -> Result<Vec<SecretAcl>> {
        self.backend.list_secret_acls()
    }

    // bole-6h7
    /// Stores an actor's clearance grant (used to build an `Accessor` for a
    /// connecting peer, WS5 §6).
    pub fn set_grant(&self, grant: crate::acl::policy_object::ClearanceGrant) -> Result<()> {
        self.backend.set_grant(&grant)
    }
}

// bole-mhs
#[cfg(test)]
mod tests {
    use super::{Accessor, PathRole, Permission, TimelineRole};

    #[test]
    fn empty_accessor_cannot_read_anything() {
        let a = Accessor::new();
        assert!(!a.can_read_path("secrets/prod.key"));
        assert!(!a.can_read_timeline("leslie/private/exp"));
    }

    #[test]
    fn matching_role_grants_read() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
        assert!(a.can_read_path("secrets/prod.key"));
        assert!(a.can_read_path("secrets/a/b"));
        assert!(!a.can_read_path("src/main.rs"));
    }

    #[test]
    fn write_role_does_not_grant_read() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Write });
        assert!(!a.can_read_path("secrets/prod.key"));
        assert!(a.can_write_path("secrets/prod.key"));
    }

    #[test]
    fn timeline_role_matching() {
        let a = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "leslie/private/**".into(), permission: Permission::Read });
        assert!(a.can_read_timeline("leslie/private/exp-foo"));
        assert!(!a.can_read_timeline("main"));
    }

    // bole-l54
    #[test]
    fn read_role_does_not_grant_write() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Read })
            .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Read });
        assert!(a.can_read_path("src/lib.rs"));
        assert!(!a.can_write_path("src/lib.rs"));
        assert!(a.can_read_timeline("main"));
        assert!(!a.can_write_timeline("main"));
    }

    // bole-l54
    #[test]
    fn write_role_grants_write() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })
            .with_timeline_role(TimelineRole { pattern: "agent/**".into(), permission: Permission::Write });
        assert!(a.can_write_path("src/lib.rs"));
        assert!(a.can_write_timeline("agent/fmt"));
        assert!(!a.can_write_timeline("main"));
    }

    // bole-l54
    #[test]
    fn roles_union_across_multiple_grants() {
        let a = Accessor::new()
            .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "docs/**".into(), permission: Permission::Read });
        // write on src is granted; not on docs (only a read role there)
        assert!(a.can_write_path("src/main.rs"));
        assert!(!a.can_write_path("docs/readme.md"));
        // read is granted on docs but not on src (only a write role there)
        assert!(a.can_read_path("docs/readme.md"));
        assert!(!a.can_read_path("src/main.rs"));
        // neither role covers an unrelated path
        assert!(!a.can_read_path("secrets/key"));
    }

    // bole-qv5
    #[test]
    fn privileged_accessor_can_read_everything() {
        let a = Accessor::privileged();
        assert!(a.can_read_path("secrets/prod.key"));
        assert!(a.can_read_path("src/main.rs"));
        assert!(a.can_read_timeline("leslie/private/exp-foo"));
        assert!(a.can_read_timeline("main"));
        // privileged does not grant write
        assert!(!a.can_write_path("src/main.rs"));
        assert!(!a.can_write_timeline("main"));
    }

    // bole-sxf
    #[test]
    fn acl_store_path_is_protected() {
        use crate::acl::memory::MemoryAclBackend;
        let store = super::AclStore::new(MemoryAclBackend::new());

        // Initially nothing is protected
        assert!(!store.path_is_protected("secrets/prod.key").unwrap());

        // After adding an ACL, matching paths are protected
        store.set_path_acl(super::PathAcl { glob: "secrets/**".into() }).unwrap();
        assert!(store.path_is_protected("secrets/prod.key").unwrap());
        assert!(store.path_is_protected("secrets/a/b/c").unwrap());
        assert!(!store.path_is_protected("src/main.rs").unwrap());

        // After removing, no longer protected
        store.remove_path_acl("secrets/**").unwrap();
        assert!(!store.path_is_protected("secrets/prod.key").unwrap());
    }

    #[test]
    fn acl_store_timeline_is_protected() {
        use crate::acl::memory::MemoryAclBackend;
        let store = super::AclStore::new(MemoryAclBackend::new());

        assert!(!store.timeline_is_protected("main").unwrap());

        store.set_timeline_acl(super::TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();
        assert!(store.timeline_is_protected("leslie/private/exp-foo").unwrap());
        assert!(!store.timeline_is_protected("main").unwrap());

        store.remove_timeline_acl("leslie/private/**").unwrap();
        assert!(!store.timeline_is_protected("leslie/private/exp-foo").unwrap());
    }

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
}
