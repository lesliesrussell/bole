// bole-2q8
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn full_write_accessor() -> Accessor {
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
}

/// Build a bole repo with `n` linear commits on timeline "main", 5-file flat tree.
/// Each commit changes file0.txt; the other 4 files are shared across all snapshots.
async fn build_linear_repo(n: usize) -> Repository {
    let repo = Repository::memory();
    let accessor = full_write_accessor();

    let mut entries = BTreeMap::new();
    for i in 0..5u8 {
        let blob = repo.objects.put_blob(Bytes::from(vec![i; 128])).await.unwrap();
        entries.insert(format!("file{i}.txt"), TreeEntry { id: blob, kind: EntryKind::Blob });
    }
    let tree = repo.objects.put_tree(entries.clone()).await.unwrap();
    let mut prev = repo.objects.put_snapshot(Snapshot {
        root: tree,
        parents: vec![],
        author: "bench".into(),
        created_at: 0,
        message: "init".into(),
    }).await.unwrap();

    let name = RefName::new("main").unwrap();
    repo.refs
        .create_timeline(name.clone(), prev, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
        .unwrap();

    for i in 1..n {
        let changed = repo.objects
            .put_blob(Bytes::from(format!("file0 version {i}")))
            .await
            .unwrap();
        entries.insert("file0.txt".to_string(), TreeEntry { id: changed, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries.clone()).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree,
            parents: vec![prev],
            author: "bench".into(),
            created_at: i as u64,
            message: format!("commit {i}"),
        }).await.unwrap();
        repo.advance_timeline(&name, snap, &accessor).await.unwrap();
        prev = snap;
    }

    repo
}

fn bench_project_to_git_linear_10(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let repo = rt.block_on(build_linear_repo(10));
    let accessor = full_write_accessor();

    c.bench_function("project_to_git_linear_10", |b| {
        b.iter(|| {
            let git_dir = TempDir::new().unwrap();
            rt.block_on(async {
                bole::project_to_git(&repo, git_dir.path(), &accessor)
                    .await
                    .unwrap();
            })
            // git_dir drops here, cleaning up the temp directory
        })
    });
}

fn bench_project_to_git_linear_100(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let repo = rt.block_on(build_linear_repo(100));
    let accessor = full_write_accessor();

    c.bench_function("project_to_git_linear_100", |b| {
        b.iter(|| {
            let git_dir = TempDir::new().unwrap();
            rt.block_on(async {
                bole::project_to_git(&repo, git_dir.path(), &accessor)
                    .await
                    .unwrap();
            })
        })
    });
}

criterion_group!(benches, bench_project_to_git_linear_10, bench_project_to_git_linear_100);
criterion_main!(benches);
