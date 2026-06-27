// bole-mhs
use crate::error::Result;
use crate::acl::{PathAcl, TimelineAcl};

pub trait AclBackend: Send + Sync {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>>;
    fn set_path_acl(&self, acl: &PathAcl) -> Result<()>;
    fn delete_path_acl(&self, glob: &str) -> Result<()>;
    fn list_path_acls(&self) -> Result<Vec<PathAcl>>;

    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>>;
    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()>;
    fn delete_timeline_acl(&self, pattern: &str) -> Result<()>;
    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>>;
}
