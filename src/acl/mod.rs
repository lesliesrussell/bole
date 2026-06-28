// bole-mhs
pub mod backend;
pub mod disk;
pub mod glob;
pub mod memory;

use crate::error::Result;
use backend::AclBackend;
use glob::glob_matches;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

// bole-mhs
// bole-p8u
/// The set of path and timeline roles held by a single actor.
///
/// `Accessor` is the runtime credential object: it is constructed with the
/// roles an actor has been granted and then passed to repository operations
/// that enforce access control.  An empty `Accessor` has no permissions.
#[derive(Debug, Clone, Default)]
pub struct Accessor {
    // bole-p8u
    /// Path-level grants held by this accessor.
    pub path_roles: HashSet<PathRole>,
    // bole-p8u
    /// Timeline-level grants held by this accessor.
    pub timeline_roles: HashSet<TimelineRole>,
}

impl Accessor {
    // bole-p8u
    /// Creates an `Accessor` with no roles.
    pub fn new() -> Self { Self::default() }

    // bole-p8u
    /// Returns `self` with `role` added to the path role set (builder-style).
    pub fn with_path_role(mut self, role: PathRole) -> Self {
        self.path_roles.insert(role);
        self
    }

    // bole-p8u
    /// Returns `self` with `role` added to the timeline role set (builder-style).
    pub fn with_timeline_role(mut self, role: TimelineRole) -> Self {
        self.timeline_roles.insert(role);
        self
    }

    // bole-qv5
    // bole-p8u
    /// Returns an `Accessor` that can read all paths and all timelines.
    ///
    /// Intended for internal repository operations that must bypass per-user
    /// ACL checks (e.g. walking the full tree during a merge).  Does NOT
    /// grant write access.
    pub fn privileged() -> Self {
        Self::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
    }

    // bole-p8u
    /// Returns `true` if this accessor holds a `Read` role whose glob matches `path`.
    pub fn can_read_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.glob, path)
        )
    }

    // bole-p8u
    /// Returns `true` if this accessor holds a `Write` role whose glob matches `path`.
    pub fn can_write_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.glob, path)
        )
    }

    // bole-p8u
    /// Returns `true` if this accessor holds a `Read` role whose pattern matches the timeline `name`.
    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.pattern, name)
        )
    }

    // bole-p8u
    /// Returns `true` if this accessor holds a `Write` role whose pattern matches the timeline `name`.
    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.pattern, name)
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
}
