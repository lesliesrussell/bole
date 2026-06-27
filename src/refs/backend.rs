// bole-i1v
use crate::error::Result;
use crate::refs::{Ref, RefName};

pub trait RefBackend: Send + Sync {
    fn get(&self, name: &RefName) -> Result<Option<Ref>>;
    fn set(&self, name: &RefName, r: &Ref) -> Result<()>;
    fn delete(&self, name: &RefName) -> Result<()>;
    fn list(&self, prefix: &str) -> Result<Vec<RefName>>;
}
