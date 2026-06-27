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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission { Read, Write }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PathRole {
    pub glob: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRole {
    pub pattern: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathAcl {
    pub glob: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineAcl {
    pub pattern: String,
}

// bole-mhs
#[derive(Debug, Clone, Default)]
pub struct Accessor {
    pub path_roles: HashSet<PathRole>,
    pub timeline_roles: HashSet<TimelineRole>,
}

impl Accessor {
    pub fn new() -> Self { Self::default() }

    pub fn with_path_role(mut self, role: PathRole) -> Self {
        self.path_roles.insert(role);
        self
    }

    pub fn with_timeline_role(mut self, role: TimelineRole) -> Self {
        self.timeline_roles.insert(role);
        self
    }

    // bole-qv5
    pub fn privileged() -> Self {
        Self::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
    }

    pub fn can_read_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.glob, path)
        )
    }

    pub fn can_write_path(&self, path: &str) -> bool {
        self.path_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.glob, path)
        )
    }

    pub fn can_read_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Read && glob_matches(&r.pattern, name)
        )
    }

    pub fn can_write_timeline(&self, name: &str) -> bool {
        self.timeline_roles.iter().any(|r|
            r.permission == Permission::Write && glob_matches(&r.pattern, name)
        )
    }
}

// bole-mhs
pub struct AclStore {
    backend: Box<dyn AclBackend>,
}

impl AclStore {
    pub fn new(backend: impl AclBackend + 'static) -> Self {
        Self { backend: Box::new(backend) }
    }

    pub fn set_path_acl(&self, acl: PathAcl) -> Result<()> { self.backend.set_path_acl(&acl) }
    pub fn remove_path_acl(&self, glob: &str) -> Result<()> { self.backend.delete_path_acl(glob) }
    pub fn list_path_acls(&self) -> Result<Vec<PathAcl>> { self.backend.list_path_acls() }

    pub fn set_timeline_acl(&self, acl: TimelineAcl) -> Result<()> { self.backend.set_timeline_acl(&acl) }
    pub fn remove_timeline_acl(&self, pattern: &str) -> Result<()> { self.backend.delete_timeline_acl(pattern) }
    pub fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> { self.backend.list_timeline_acls() }

    pub fn path_is_protected(&self, path: &str) -> Result<bool> {
        Ok(self.backend.list_path_acls()?.iter().any(|a| glob_matches(&a.glob, path)))
    }

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
