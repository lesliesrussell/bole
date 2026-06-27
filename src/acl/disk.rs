// bole-mhs
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
use crate::error::Result;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub struct DiskAclBackend {
    root: PathBuf,
}

impl DiskAclBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }
}

impl AclBackend for DiskAclBackend {
    fn get_path_acl(&self, _glob: &str) -> Result<Option<PathAcl>> { todo!() }
    fn set_path_acl(&self, _acl: &PathAcl) -> Result<()> { todo!() }
    fn delete_path_acl(&self, _glob: &str) -> Result<()> { todo!() }
    fn list_path_acls(&self) -> Result<Vec<PathAcl>> { todo!() }
    fn get_timeline_acl(&self, _pattern: &str) -> Result<Option<TimelineAcl>> { todo!() }
    fn set_timeline_acl(&self, _acl: &TimelineAcl) -> Result<()> { todo!() }
    fn delete_timeline_acl(&self, _pattern: &str) -> Result<()> { todo!() }
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> { todo!() }
}
