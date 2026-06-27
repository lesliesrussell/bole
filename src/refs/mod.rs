// bole-s5y
pub mod backend;
pub mod disk;
pub mod memory;
pub mod name;
pub mod ref_type;
pub mod tag;
pub mod timeline;

// bole-i1v
pub use backend::RefBackend;
pub use memory::MemoryRefBackend;

// bole-fkt
pub use disk::DiskRefBackend;

// bole-prn
pub use name::RefName;
pub use ref_type::Ref;
pub use tag::Tag;
pub use timeline::{Timeline, TimelinePolicy};

// bole-2jp
pub use self::store::RefStore;

// bole-2jp
use crate::error::Error;
use crate::object::ObjectId;

// bole-2jp
mod store {
    use super::{Error, ObjectId, Ref, RefBackend, RefName, Tag, Timeline, TimelinePolicy};
    use crate::error::Result;

    pub struct RefStore {
        backend: Box<dyn RefBackend>,
    }

    impl RefStore {
        pub fn new(backend: impl RefBackend + 'static) -> Self {
            Self { backend: Box::new(backend) }
        }

        pub fn create_tag(
            &self,
            name: RefName,
            target: ObjectId,
            message: Option<String>,
            now: u64,
        ) -> Result<()> {
            self.backend.set(&name, &Ref::Tag(Tag { target, created_at: now, message }))
        }

        pub fn move_tag(&self, name: &RefName, target: ObjectId) -> Result<()> {
            match self.backend.get(name)? {
                Some(Ref::Tag(mut tag)) => {
                    tag.target = target;
                    self.backend.set(name, &Ref::Tag(tag))
                }
                Some(Ref::Timeline(_)) => Err(Error::WrongRefKind(format!(
                    "'{}' is a timeline, not a tag",
                    name.as_str()
                ))),
                None => Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
            }
        }

        pub fn get_tag(&self, name: &RefName) -> Result<Option<Tag>> {
            match self.backend.get(name)? {
                Some(Ref::Tag(t)) => Ok(Some(t)),
                Some(Ref::Timeline(_)) => Err(Error::WrongRefKind(format!(
                    "'{}' is a timeline, not a tag",
                    name.as_str()
                ))),
                None => Ok(None),
            }
        }

        pub fn create_timeline(
            &self,
            name: RefName,
            head: ObjectId,
            policy: TimelinePolicy,
            now: u64,
        ) -> Result<()> {
            self.backend.set(&name, &Ref::Timeline(Timeline { head, policy, created_at: now }))
        }

        pub fn advance_head(&self, name: &RefName, new_head: ObjectId) -> Result<()> {
            match self.backend.get(name)? {
                Some(Ref::Timeline(mut tl)) => {
                    tl.head = new_head;
                    self.backend.set(name, &Ref::Timeline(tl))
                }
                Some(Ref::Tag(_)) => Err(Error::WrongRefKind(format!(
                    "'{}' is a tag, not a timeline",
                    name.as_str()
                ))),
                None => Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
            }
        }

        pub fn get_timeline(&self, name: &RefName) -> Result<Option<Timeline>> {
            match self.backend.get(name)? {
                Some(Ref::Timeline(t)) => Ok(Some(t)),
                Some(Ref::Tag(_)) => Err(Error::WrongRefKind(format!(
                    "'{}' is a tag, not a timeline",
                    name.as_str()
                ))),
                None => Ok(None),
            }
        }

        pub fn get(&self, name: &RefName) -> Result<Option<Ref>> {
            self.backend.get(name)
        }

        pub fn delete_ref(&self, name: &RefName) -> Result<()> {
            self.backend.delete(name)
        }

        pub fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
            self.backend.list(prefix)
        }
    }
}

// bole-2jp
#[cfg(test)]
mod tests {
    use super::RefStore;
    use crate::refs::{MemoryRefBackend, RefName, TimelinePolicy};
    use crate::object::ObjectId;

    fn store() -> RefStore { RefStore::new(MemoryRefBackend::new()) }
    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }

    #[test]
    fn create_and_get_tag() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, Some("release".into()), 1000).unwrap();
        let tag = s.get_tag(&name("v1")).unwrap().unwrap();
        assert_eq!(tag.target, id);
        assert_eq!(tag.message.as_deref(), Some("release"));
    }

    #[test]
    fn move_tag_updates_target() {
        let s = store();
        let id1 = ObjectId::new([1u8; 32]);
        let id2 = ObjectId::new([2u8; 32]);
        s.create_tag(name("v1"), id1, None, 1).unwrap();
        s.move_tag(&name("v1"), id2).unwrap();
        assert_eq!(s.get_tag(&name("v1")).unwrap().unwrap().target, id2);
    }

    #[test]
    fn move_tag_on_timeline_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
        let err = s.move_tag(&name("main"), id).unwrap_err();
        assert!(matches!(err, crate::error::Error::WrongRefKind(_)));
    }

    #[test]
    fn create_and_advance_timeline() {
        let s = store();
        let s1 = ObjectId::new([1u8; 32]);
        let s2 = ObjectId::new([2u8; 32]);
        let s3 = ObjectId::new([3u8; 32]);
        s.create_timeline(name("main"), s1, TimelinePolicy::Append, 1).unwrap();
        s.advance_head(&name("main"), s2).unwrap();
        s.advance_head(&name("main"), s3).unwrap();
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, s3);
    }

    #[test]
    fn advance_head_on_tag_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        let err = s.advance_head(&name("v1"), id).unwrap_err();
        assert!(matches!(err, crate::error::Error::WrongRefKind(_)));
    }

    #[test]
    fn delete_ref_works_for_both_kinds() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        // delete a tag
        s.create_tag(name("v1"), id, None, 1).unwrap();
        s.delete_ref(&name("v1")).unwrap();
        assert!(s.get(&name("v1")).unwrap().is_none());
        // delete a timeline
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
        s.delete_ref(&name("main")).unwrap();
        assert!(s.get(&name("main")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("leslie/exp-a"), id, None, 1).unwrap();
        s.create_tag(name("leslie/exp-b"), id, None, 1).unwrap();
        s.create_tag(name("v1"), id, None, 1).unwrap();
        let listed = s.list("leslie/").unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn move_and_advance_not_found_returns_storage_error() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        let err = s.move_tag(&name("nonexistent"), id).unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
        let err = s.advance_head(&name("nonexistent"), id).unwrap_err();
        assert!(matches!(err, crate::error::Error::Storage(_)));
    }
}
