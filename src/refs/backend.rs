// bole-i1v
use crate::error::Result;
use crate::refs::{Ref, RefName};

pub trait RefBackend: Send + Sync {
    fn get(&self, name: &RefName) -> Result<Option<Ref>>;
    fn set(&self, name: &RefName, r: &Ref) -> Result<()>;
    fn delete(&self, name: &RefName) -> Result<()>;
    fn list(&self, prefix: &str) -> Result<Vec<RefName>>;

    // bole-sk6
    /// Applies a plan of absolute final ref values (`None` = delete) atomically.
    /// The default applies each op in turn — adequate for the in-memory backend,
    /// which is effectively atomic under its own lock. Crash-atomic backends (the
    /// disk journal) override this.
    fn apply_atomic(&self, plan: &[(RefName, Option<Ref>)]) -> Result<()> {
        for (name, val) in plan {
            match val {
                Some(r) => self.set(name, r)?,
                None => self.delete(name)?,
            }
        }
        Ok(())
    }

    // bole-sk6
    /// Replays any interrupted transaction journal. Default: nothing to recover.
    fn recover(&self) -> Result<()> {
        Ok(())
    }
}
