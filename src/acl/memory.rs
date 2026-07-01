// bole-mhs
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
// bole-9mz
use crate::acl::SecretAcl;
use crate::error::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct MemoryAclBackend {
    path_acls: Arc<RwLock<HashMap<String, PathAcl>>>,
    timeline_acls: Arc<RwLock<HashMap<String, TimelineAcl>>>,
    // bole-9mz
    secret_acls: Arc<RwLock<HashMap<String, SecretAcl>>>,
}

impl MemoryAclBackend {
    pub fn new() -> Self { Self::default() }
}

impl AclBackend for MemoryAclBackend {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>> {
        Ok(self.path_acls.read().unwrap().get(glob).cloned())
    }
    fn set_path_acl(&self, acl: &PathAcl) -> Result<()> {
        self.path_acls.write().unwrap().insert(acl.glob.clone(), acl.clone());
        Ok(())
    }
    fn delete_path_acl(&self, glob: &str) -> Result<()> {
        self.path_acls.write().unwrap().remove(glob);
        Ok(())
    }
    fn list_path_acls(&self) -> Result<Vec<PathAcl>> {
        Ok(self.path_acls.read().unwrap().values().cloned().collect())
    }
    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>> {
        Ok(self.timeline_acls.read().unwrap().get(pattern).cloned())
    }
    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()> {
        self.timeline_acls.write().unwrap().insert(acl.pattern.clone(), acl.clone());
        Ok(())
    }
    fn delete_timeline_acl(&self, pattern: &str) -> Result<()> {
        self.timeline_acls.write().unwrap().remove(pattern);
        Ok(())
    }
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> {
        Ok(self.timeline_acls.read().unwrap().values().cloned().collect())
    }
    // bole-9mz
    fn list_secret_acls(&self) -> Result<Vec<SecretAcl>> {
        Ok(self.secret_acls.read().unwrap().values().cloned().collect())
    }
    // bole-9mz
    fn set_secret_acl(&self, acl: &SecretAcl) -> Result<()> {
        self.secret_acls.write().unwrap().insert(acl.name.clone(), acl.clone());
        Ok(())
    }
    // bole-9mz
    fn delete_secret_acl(&self, name: &str) -> Result<()> {
        self.secret_acls.write().unwrap().remove(name);
        Ok(())
    }
}

// bole-mhs
#[cfg(test)]
mod tests {
    use super::MemoryAclBackend;
    use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};

    #[test]
    fn path_acl_set_get_delete() {
        let b = MemoryAclBackend::new();
        let acl = PathAcl { glob: "secrets/**".into() };
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("secrets/**").unwrap(), Some(acl));
        b.delete_path_acl("secrets/**").unwrap();
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
    }

    #[test]
    fn timeline_acl_set_get_delete() {
        let b = MemoryAclBackend::new();
        let acl = TimelineAcl { pattern: "leslie/private/**".into() };
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
        b.set_timeline_acl(&acl).unwrap();
        assert_eq!(b.get_timeline_acl("leslie/private/**").unwrap(), Some(acl));
        b.delete_timeline_acl("leslie/private/**").unwrap();
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
    }

    #[test]
    fn list_returns_all_entries() {
        let b = MemoryAclBackend::new();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        b.set_path_acl(&PathAcl { glob: "notes/**".into() }).unwrap();
        let mut list = b.list_path_acls().unwrap();
        list.sort_by(|a, b| a.glob.cmp(&b.glob));
        assert_eq!(list[0].glob, "notes/**");
        assert_eq!(list[1].glob, "secrets/**");
    }
}
