// bole-s5y
pub mod backend;
pub mod disk;
pub mod memory;
pub mod name;
pub mod ref_type;
pub mod tag;
pub mod timeline;
// bole-sk6
pub mod transaction;

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
// bole-sk6
pub use transaction::{RefOp, RefTransaction};

// bole-2jp
use crate::error::Error;
use crate::object::ObjectId;

// bole-2jp
mod store {
    use super::{Error, ObjectId, Ref, RefBackend, RefName, Tag, Timeline, TimelinePolicy};
    use crate::error::Result;

    pub struct RefStore {
        backend: Box<dyn RefBackend>,
        // bole-bti: serializes the read-validate-write of commit_transaction so a
        // compare-and-swap is atomic against concurrent in-process committers
        // (e.g. two sync pushes on one Arc-shared server). Held only across the
        // synchronous commit — never across an await.
        commit_lock: std::sync::Mutex<()>,
    }

    impl RefStore {
        pub fn new(backend: impl RefBackend + 'static) -> Self {
            Self { backend: Box::new(backend), commit_lock: std::sync::Mutex::new(()) }
        }

        pub fn create_tag(
            &self,
            name: RefName,
            target: ObjectId,
            message: Option<String>,
            now: u64,
        ) -> Result<()> {
            if self.backend.get(&name)?.is_some() {
                return Err(Error::Storage(format!(
                    "ref already exists: {}",
                    name.as_str()
                )));
            }
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
            // bole-qv5
            kind: String,
            expires_at: Option<u64>,
        ) -> Result<()> {
            if self.backend.get(&name)?.is_some() {
                return Err(Error::Storage(format!(
                    "ref already exists: {}",
                    name.as_str()
                )));
            }
            self.backend.set(&name, &Ref::Timeline(Timeline {
                head,
                policy,
                created_at: now,
                // bole-qv5
                kind,
                expires_at,
            }))
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

        // bole-1vi
        pub(crate) fn set_raw(&self, name: &RefName, r: &Ref) -> Result<()> {
            self.backend.set(name, r)
        }

        // bole-sk6
        /// Begins an atomic multi-ref transaction.
        pub fn transaction(&self) -> super::RefTransaction<'_> {
            super::RefTransaction::new(self)
        }

        // bole-sk6
        /// Resolves and atomically applies a transaction's ops. All-or-nothing.
        pub(crate) fn commit_transaction(&self, ops: &[super::RefOp]) -> Result<()> {
            // bole-bti: hold the lock across resolve (read + CAS validate) AND
            // apply_atomic (write) so the compare-and-swap is atomic. Without it
            // two concurrent committers could both validate against the same old
            // head and both write, silently losing one update and bypassing the
            // fast-forward gate. Poisoned-lock recovers the guard (no state is
            // left inconsistent — apply_atomic is itself all-or-nothing).
            let _guard = self.commit_lock.lock().unwrap_or_else(|e| e.into_inner());
            let plan = super::transaction::resolve(&*self.backend, ops)?;
            self.backend.apply_atomic(&plan)
        }

        // bole-sk6
        /// Replays any interrupted transaction journal (called on open).
        pub fn recover(&self) -> Result<()> {
            self.backend.recover()
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
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();
        let err = s.move_tag(&name("main"), id).unwrap_err();
        assert!(matches!(err, crate::error::Error::WrongRefKind(_)));
    }

    #[test]
    fn create_and_advance_timeline() {
        let s = store();
        let s1 = ObjectId::new([1u8; 32]);
        let s2 = ObjectId::new([2u8; 32]);
        let s3 = ObjectId::new([3u8; 32]);
        s.create_timeline(name("main"), s1, TimelinePolicy::Append, 1, "persistent".into(), None).unwrap();
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
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();
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

    #[test]
    fn create_tag_on_existing_ref_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_tag(name("v1"), id, None, 1).unwrap();
        // creating again must fail
        assert!(s.create_tag(name("v1"), id, None, 2).is_err());
        // creating a timeline with the same name must also fail
        assert!(s.create_timeline(name("v1"), id, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).is_err());
    }

    #[test]
    fn create_timeline_on_existing_ref_errors() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Append, 1, "persistent".into(), None).unwrap();
        assert!(s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).is_err());
        assert!(s.create_tag(name("main"), id, None, 2).is_err());
    }

    // bole-qv5
    #[test]
    fn timeline_kind_and_expires_at_stored_and_retrieved() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(
            name("ephemeral"),
            id,
            TimelinePolicy::Unrestricted,
            1,
            "ephemeral".into(),
            Some(9999),
        ).unwrap();
        let tl = s.get_timeline(&name("ephemeral")).unwrap().unwrap();
        assert_eq!(tl.kind, "ephemeral");
        assert_eq!(tl.expires_at, Some(9999));
    }

    #[test]
    fn timeline_default_kind_is_persistent() {
        let s = store();
        let id = ObjectId::new([2u8; 32]);
        s.create_timeline(
            name("main"),
            id,
            TimelinePolicy::Unrestricted,
            1,
            "persistent".into(),
            None,
        ).unwrap();
        let tl = s.get_timeline(&name("main")).unwrap().unwrap();
        assert_eq!(tl.kind, "persistent");
        assert_eq!(tl.expires_at, None);
    }

    // bole-sk6
    #[test]
    fn transaction_commits_all_or_nothing() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // A multi-ref transaction: advance main + create a tag + a new timeline.
        let new_head = ObjectId::new([2u8; 32]);
        let mut tx = s.transaction();
        tx.advance_head(name("main"), new_head)
            .create_tag(name("v1"), id, None, 1)
            .create_timeline(name("feature"), id, TimelinePolicy::Append, 1, "persistent".into(), None);
        tx.commit().unwrap();
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, new_head);
        assert_eq!(s.get_tag(&name("v1")).unwrap().unwrap().target, id);
        assert!(s.get_timeline(&name("feature")).unwrap().is_some());
    }

    // bole-sk6
    #[test]
    fn transaction_aborts_on_failed_op_leaves_no_partial_state() {
        let s = store();
        let id = ObjectId::new([1u8; 32]);
        s.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // Second op fails (create_tag on an existing name) → nothing applied.
        let mut tx = s.transaction();
        tx.advance_head(name("main"), ObjectId::new([9u8; 32]))
            .create_tag(name("main"), id, None, 1); // 'main' already exists
        assert!(tx.commit().is_err());
        // The advance was NOT applied.
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, id);
    }

    // bole-sk6
    #[test]
    fn cas_advance_head_if_rejects_lost_update() {
        let s = store();
        let a = ObjectId::new([1u8; 32]);
        let b = ObjectId::new([2u8; 32]);
        let c = ObjectId::new([3u8; 32]);
        s.create_timeline(name("main"), a, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // Correct expected old head → succeeds.
        let mut tx = s.transaction();
        tx.advance_head_if(name("main"), a, b);
        tx.commit().unwrap();
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, b);

        // Stale expected old head → conflict, no change.
        let mut tx = s.transaction();
        tx.advance_head_if(name("main"), a, c); // expects a, but head is b
        let err = tx.commit().unwrap_err();
        assert!(matches!(err, crate::error::Error::TransactionConflict(_)), "got {err:?}");
        assert_eq!(s.get_timeline(&name("main")).unwrap().unwrap().head, b);
    }

    // bole-sk6
    #[test]
    fn disk_transaction_and_journal_recovery() {
        use crate::refs::DiskRefBackend;
        use crate::refs::Ref;
        let dir = tempfile::TempDir::new().unwrap();
        let a = ObjectId::new([1u8; 32]);
        let b = ObjectId::new([2u8; 32]);

        // A committed disk transaction persists and leaves no journal behind.
        {
            let s = RefStore::new(DiskRefBackend::open(dir.path()).unwrap());
            s.create_timeline(name("main"), a, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
            let mut tx = s.transaction();
            tx.advance_head(name("main"), b).create_tag(name("v1"), a, None, 1);
            tx.commit().unwrap();
        }
        let txn_dir = dir.path().join("refs").join(".txn");
        let leftover = std::fs::read_dir(&txn_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x == "journal")))
            .unwrap_or(false);
        assert!(!leftover, "journal should be deleted after commit");

        // Simulate a crash mid-commit: drop a journal recording an absolute
        // final value, then reopen → recovery replays it idempotently.
        std::fs::create_dir_all(&txn_dir).unwrap();
        let plan: Vec<(RefName, Option<Ref>)> = vec![(
            name("recovered"),
            Some(Ref::Tag(crate::refs::Tag { target: b, created_at: 5, message: None })),
        )];
        let bytes = postcard::to_allocvec(&plan).unwrap();
        std::fs::write(txn_dir.join("deadbeef.journal"), &bytes).unwrap();

        let s2 = RefStore::new(DiskRefBackend::open(dir.path()).unwrap());
        // Recovery applied the journalled ref and removed the journal.
        assert_eq!(s2.get_tag(&name("recovered")).unwrap().unwrap().target, b);
        assert!(!txn_dir.join("deadbeef.journal").exists());
        // The earlier committed state is intact.
        assert_eq!(s2.get_timeline(&name("main")).unwrap().unwrap().head, b);
    }
}
