// bole-i1v
use crate::error::Result;
use crate::refs::{backend::RefBackend, Ref, RefName};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct MemoryRefBackend {
    store: Arc<RwLock<HashMap<String, Ref>>>,
}

impl MemoryRefBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RefBackend for MemoryRefBackend {
    fn get(&self, name: &RefName) -> Result<Option<Ref>> {
        Ok(self.store.read().unwrap().get(name.as_str()).cloned())
    }

    fn set(&self, name: &RefName, r: &Ref) -> Result<()> {
        self.store
            .write()
            .unwrap()
            .insert(name.as_str().to_owned(), r.clone());
        Ok(())
    }

    fn delete(&self, name: &RefName) -> Result<()> {
        self.store.write().unwrap().remove(name.as_str());
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
        let store = self.store.read().unwrap();
        let mut names: Vec<RefName> = store
            .keys()
            .filter(|k| k.starts_with(prefix))
            .map(|k| RefName::new(k.as_str()).unwrap())
            .collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        let r = b.get(&name("v1")).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let b = MemoryRefBackend::new();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].as_str(), "leslie/a");
        assert_eq!(names[1].as_str(), "leslie/b");
    }

    #[test]
    fn list_empty_prefix_returns_all() {
        let b = MemoryRefBackend::new();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("a"), &tag(id)).unwrap();
        b.set(&name("b/c"), &tag(id)).unwrap();
        assert_eq!(b.list("").unwrap().len(), 2);
    }
}
