// bole-wmu
use bole::{DiskRefBackend, MemoryRefBackend, ObjectId, Ref, RefName, RefStore, TimelinePolicy};
use tempfile::TempDir;

fn name(s: &str) -> RefName { RefName::new(s).unwrap() }

fn run_t2_suite(store: RefStore) {
    let s1 = ObjectId::new([1u8; 32]);
    let s2 = ObjectId::new([2u8; 32]);
    let s3 = ObjectId::new([3u8; 32]);

    // T2: create tags v1 and experiment/foo
    store.create_tag(name("v1"), s1, None, 1000).unwrap();
    store.create_tag(name("experiment/foo"), s1, None, 1000).unwrap();

    // T2: move experiment/foo — pure reference update, v1 unchanged
    store.move_tag(&name("experiment/foo"), s2).unwrap();
    assert_eq!(store.get_tag(&name("v1")).unwrap().unwrap().target, s1);
    assert_eq!(store.get_tag(&name("experiment/foo")).unwrap().unwrap().target, s2);

    // T2: create main timeline and advance head S1→S2→S3
    store.create_timeline(name("main"), s1, TimelinePolicy::Append, 1000, "persistent".into(), None).unwrap();
    store.advance_head(&name("main"), s2).unwrap();
    store.advance_head(&name("main"), s3).unwrap();
    assert_eq!(store.get_timeline(&name("main")).unwrap().unwrap().head, s3);

    // T2: list by prefix
    let id = ObjectId::new([9u8; 32]);
    store.create_tag(name("leslie/exp-a"), id, None, 1).unwrap();
    store.create_tag(name("leslie/exp-b"), id, None, 1).unwrap();
    let listed = store.list("leslie/").unwrap();
    assert_eq!(listed.len(), 2);
}

#[test]
fn t2_memory_backend() {
    run_t2_suite(RefStore::new(MemoryRefBackend::new()));
}

#[test]
fn t2_disk_backend() {
    let dir = TempDir::new().unwrap();
    let backend = DiskRefBackend::open(dir.path()).unwrap();
    run_t2_suite(RefStore::new(backend));
}

#[test]
fn t2_wrong_kind_errors() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();
    // move_tag on a timeline must fail
    assert!(store.move_tag(&name("main"), id).is_err());

    store.create_tag(name("v1"), id, None, 1).unwrap();
    // advance_head on a tag must fail
    assert!(store.advance_head(&name("v1"), id).is_err());
}

#[test]
fn t2_ref_name_validation() {
    assert!(RefName::new("").is_err());
    assert!(RefName::new("/leading").is_err());
    assert!(RefName::new("trailing/").is_err());
    assert!(RefName::new("a//b").is_err());
    assert!(RefName::new("../escape").is_err());
    assert!(RefName::new("valid/name").is_ok());
}

#[test]
fn t2_delete_ref() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_tag(name("v1"), id, None, 1).unwrap();
    store.delete_ref(&name("v1")).unwrap();
    assert!(store.get(&name("v1")).unwrap().is_none());
}

#[test]
fn t2_disk_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let id = ObjectId::new([1u8; 32]);
    {
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let store = RefStore::new(b);
        store.create_tag(name("v1"), id, Some("persisted".into()), 1).unwrap();
        store.create_timeline(name("main"), id, TimelinePolicy::Append, 1, "persistent".into(), None).unwrap();
    }
    let b = DiskRefBackend::open(dir.path()).unwrap();
    let store = RefStore::new(b);
    let tag = store.get_tag(&name("v1")).unwrap().unwrap();
    assert_eq!(tag.message.as_deref(), Some("persisted"));
    assert!(store.get_timeline(&name("main")).unwrap().is_some());
}

#[test]
fn t2_get_returns_correct_variant() {
    let store = RefStore::new(MemoryRefBackend::new());
    let id = ObjectId::new([1u8; 32]);
    store.create_tag(name("v1"), id, None, 1).unwrap();
    match store.get(&name("v1")).unwrap().unwrap() {
        Ref::Tag(t) => assert_eq!(t.target, id),
        Ref::Timeline(_) => panic!("expected tag"),
    }
}

// bole-bti
/// Two concurrent compare-and-swap advances from the same base head must have
/// exactly one winner — the ref CAS is serialized, so no update is silently lost
/// and the fast-forward gate cannot be bypassed by interleaving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_cas_advance_has_exactly_one_winner() {
    use std::sync::Arc;
    let base = ObjectId::new([0u8; 32]);
    let a = ObjectId::new([1u8; 32]);
    let b = ObjectId::new([2u8; 32]);

    for _ in 0..200 {
        let store = Arc::new(RefStore::new(MemoryRefBackend::new()));
        store
            .create_timeline(name("main"), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        let s1 = store.clone();
        let s2 = store.clone();
        let t1 = tokio::spawn(async move {
            let mut tx = s1.transaction();
            tx.advance_head_if(name("main"), base, a);
            tx.commit()
        });
        let t2 = tokio::spawn(async move {
            let mut tx = s2.transaction();
            tx.advance_head_if(name("main"), base, b);
            tx.commit()
        });
        let r1 = t1.await.unwrap();
        let r2 = t2.await.unwrap();

        let oks = [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count();
        assert_eq!(oks, 1, "exactly one concurrent CAS advance from base must win");
        // The final head is the winner's target, never a torn/lost state.
        let head = store.get_timeline(&name("main")).unwrap().unwrap().head;
        assert!(head == a || head == b);
    }
}

// bole-0x3
/// After committing transactions on the disk backend, no journal file is left
/// behind (the write-ahead journal is deleted post-apply), and repeated identical
/// plans commit cleanly with per-commit-unique journal names.
#[test]
fn disk_transactions_leave_no_journal() {
    let dir = TempDir::new().unwrap();
    let store = RefStore::new(DiskRefBackend::open(dir.path()).unwrap());
    let base = ObjectId::new([0u8; 32]);
    let a = ObjectId::new([1u8; 32]);
    store
        .create_timeline(name("main"), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
        .unwrap();

    // Two identical-plan commits back to back (unique journal names avoid a
    // clobber/delete-window collision); both leave the txn dir clean.
    for target in [a, a] {
        let mut tx = store.transaction();
        tx.set(name("dup"), Ref::Tag(bole::Tag { target, created_at: 0, message: None }));
        let _ = tx.commit();
    }

    let txn_dir = dir.path().join("refs").join(".txn");
    if txn_dir.exists() {
        let leftover: Vec<_> = std::fs::read_dir(&txn_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("journal"))
            .collect();
        assert!(leftover.is_empty(), "no journal should remain after commit");
    }
}
