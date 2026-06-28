// bole-2q8
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;

fn full_write_accessor() -> Accessor {
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
}

fn bench_put_snapshot_10files(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("put_snapshot_10files", |b| {
        b.iter(|| {
            let repo = Repository::memory();
            rt.block_on(async {
                let mut entries = BTreeMap::new();
                for i in 0..10u8 {
                    let blob = repo.objects.put_blob(Bytes::from(vec![i; 256])).await.unwrap();
                    entries.insert(
                        format!("file{i}.txt"),
                        TreeEntry { id: blob, kind: EntryKind::Blob },
                    );
                }
                let tree = repo.objects.put_tree(entries).await.unwrap();
                repo.objects.put_snapshot(Snapshot {
                    root: tree,
                    parents: vec![],
                    author: "bench".into(),
                    created_at: 1,
                    message: "bench snapshot".into(),
                }).await.unwrap()
            })
        })
    });
}

fn bench_advance_timeline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let repo = Repository::memory();
    let accessor = full_write_accessor();
    let name = RefName::new("bench/main").unwrap();

    let snap0 = rt.block_on(async {
        let blob = repo.objects.put_blob(Bytes::from("v0")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("file.txt".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        repo.objects.put_snapshot(Snapshot {
            root: tree,
            parents: vec![],
            author: "bench".into(),
            created_at: 0,
            message: "init".into(),
        }).await.unwrap()
    });
    repo.refs
        .create_timeline(name.clone(), snap0, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
        .unwrap();

    let mut version = 1u64;
    c.bench_function("advance_timeline", |b| {
        b.iter(|| {
            let v = version;
            version += 1;
            rt.block_on(async {
                let blob = repo.objects
                    .put_blob(Bytes::copy_from_slice(&v.to_le_bytes()))
                    .await
                    .unwrap();
                let mut entries = BTreeMap::new();
                entries.insert("file.txt".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
                let tree = repo.objects.put_tree(entries).await.unwrap();
                let head = repo.refs.get_timeline(&name).unwrap().unwrap().head;
                let snap = repo.objects.put_snapshot(Snapshot {
                    root: tree,
                    parents: vec![head],
                    author: "bench".into(),
                    created_at: v,
                    message: "bench".into(),
                }).await.unwrap();
                repo.advance_timeline(&name, snap, &accessor).await.unwrap();
            })
        })
    });
}

fn bench_merge_timelines_clean(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let accessor = full_write_accessor();

    c.bench_function("merge_timelines_clean", |b| {
        b.iter(|| {
            let repo = Repository::memory();
            rt.block_on(async {
                // Common ancestor: 10 files
                let mut base_entries = BTreeMap::new();
                for i in 0..10u8 {
                    let blob = repo.objects.put_blob(Bytes::from(vec![i; 64])).await.unwrap();
                    base_entries.insert(
                        format!("file{i}.txt"),
                        TreeEntry { id: blob, kind: EntryKind::Blob },
                    );
                }
                let base_tree = repo.objects.put_tree(base_entries.clone()).await.unwrap();
                let base_snap = repo.objects.put_snapshot(Snapshot {
                    root: base_tree,
                    parents: vec![],
                    author: "bench".into(),
                    created_at: 0,
                    message: "base".into(),
                }).await.unwrap();

                // Branch A: change file0
                let mut a_entries = base_entries.clone();
                let a_blob = repo.objects.put_blob(Bytes::from("a-version")).await.unwrap();
                a_entries.insert("file0.txt".to_string(), TreeEntry { id: a_blob, kind: EntryKind::Blob });
                let a_tree = repo.objects.put_tree(a_entries).await.unwrap();
                let a_snap = repo.objects.put_snapshot(Snapshot {
                    root: a_tree,
                    parents: vec![base_snap],
                    author: "bench".into(),
                    created_at: 1,
                    message: "a".into(),
                }).await.unwrap();

                // Branch B: change file1
                let mut b_entries = base_entries.clone();
                let b_blob = repo.objects.put_blob(Bytes::from("b-version")).await.unwrap();
                b_entries.insert("file1.txt".to_string(), TreeEntry { id: b_blob, kind: EntryKind::Blob });
                let b_tree = repo.objects.put_tree(b_entries).await.unwrap();
                let b_snap = repo.objects.put_snapshot(Snapshot {
                    root: b_tree,
                    parents: vec![base_snap],
                    author: "bench".into(),
                    created_at: 2,
                    message: "b".into(),
                }).await.unwrap();

                let src = RefName::new("bench/a").unwrap();
                let tgt = RefName::new("bench/b").unwrap();
                repo.refs.create_timeline(src.clone(), a_snap, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();
                repo.refs.create_timeline(tgt.clone(), b_snap, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).unwrap();

                repo.merge_timelines(&src, &tgt, &accessor).await.unwrap()
            })
        })
    });
}

criterion_group!(benches, bench_put_snapshot_10files, bench_advance_timeline, bench_merge_timelines_clean);
criterion_main!(benches);
