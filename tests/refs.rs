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
    store.create_timeline(name("main"), s1, TimelinePolicy::Append, 1000).unwrap();
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
    store.create_timeline(name("main"), id, TimelinePolicy::Unrestricted, 1).unwrap();
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
        store.create_timeline(name("main"), id, TimelinePolicy::Append, 1).unwrap();
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
