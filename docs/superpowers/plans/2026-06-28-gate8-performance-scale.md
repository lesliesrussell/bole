# Gate 8: Performance and Scale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a criterion benchmark suite for key bole operations and the T8 storage deduplication test that proves structural sharing works at scale.

**Architecture:** Two independent deliverables — benchmarks in `benches/` (3 files, criterion harness) and T8 integration tests in `tests/scale.rs`. No changes to `src/`. All benchmarks use `MemoryBackend` to measure computation without disk I/O noise; the T8 disk sub-test uses `DiskBackend` to verify file system writes.

**Tech Stack:** Rust (stable, edition 2021, tokio async), criterion 0.5, tempfile 3, bole (this crate)

## Global Constraints

- No `anyhow` — `thiserror` only in library code (benches and tests may use `.unwrap()` freely)
- No feature flags — both backends always compiled
- `criterion = { version = "0.5", features = ["html_reports"] }` added to `[dev-dependencies]`
- Each contiguous block of new code tagged with `// <bead-id>` — one comment per block, not per line
- Bead workflow: create bead → claim → branch named after bead ID → implement → test → commit → merge to master → delete branch → close bead
- `cargo test` must pass before merge (all 156 existing tests + new T8 tests)

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Add criterion dev-dep + 3 `[[bench]]` entries |
| `benches/object_store.rs` | Create | `put_blob_cold`, `put_blob_dedup`, `get_blob` benchmarks |
| `benches/snapshot_ops.rs` | Create | `put_snapshot_10files`, `advance_timeline`, `merge_timelines_clean` benchmarks |
| `benches/git_projection.rs` | Create | `project_to_git_linear_10`, `project_to_git_linear_100` benchmarks |
| `tests/scale.rs` | Create | T8: object-count dedup (MemoryBackend) + disk footprint (DiskBackend) |

---

### Task 1: Benchmark Scaffold — Cargo.toml + All Three Bench Files

**Files:**
- Modify: `Cargo.toml`
- Create: `benches/object_store.rs`
- Create: `benches/snapshot_ops.rs`
- Create: `benches/git_projection.rs`

**Interfaces:**
- Consumes: `bole::store::{memory::MemoryBackend, ObjectStore}`, `bole::object::{EntryKind, Snapshot, TreeEntry}`, `bole::refs::{RefName, TimelinePolicy}`, `bole::{Accessor, PathRole, Permission, Repository, TimelineRole}`, `bole::project_to_git`
- Produces: `cargo bench` compiles and produces timing output for all 8 benchmarks

- [ ] **Step 1: Create a bead and branch**

```bash
bd create --title="G8-T1: criterion benchmark scaffold" \
  --description="Add criterion 0.5 to dev-dependencies, three [[bench]] entries to Cargo.toml, and three bench files: benches/object_store.rs (put/get benchmarks), benches/snapshot_ops.rs (snapshot/timeline/merge benchmarks), benches/git_projection.rs (project_to_git benchmarks)." \
  --type=task --priority=2
# Note the printed ID (e.g. bole-abc)
bd update <id> --claim
git checkout -b <id>
```

- [ ] **Step 2: Add criterion to Cargo.toml**

Open `Cargo.toml`. The `[dev-dependencies]` section currently ends with `flate2 = "1"`. Add criterion after it, and add the three bench entries after `[dev-dependencies]`:

```toml
[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
tempfile = "3"
flate2 = "1"
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "object_store"
harness = false

[[bench]]
name = "snapshot_ops"
harness = false

[[bench]]
name = "git_projection"
harness = false
```

- [ ] **Step 3: Verify Cargo.toml compiles**

```bash
cargo check
```

Expected: no errors. If criterion is not in the local registry, `cargo check` will fetch it.

- [ ] **Step 4: Write `benches/object_store.rs`**

```rust
// <bead-id>
use bole::store::{memory::MemoryBackend, ObjectStore};
use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};

fn make_store() -> ObjectStore {
    ObjectStore::new(MemoryBackend::new())
}

fn bench_put_blob_cold(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let payload = Bytes::from(vec![42u8; 1024]);
    c.bench_function("put_blob_cold", |b| {
        b.iter(|| {
            let store = make_store();
            let p = payload.clone();
            rt.block_on(async move { store.put_blob(p).await.unwrap() })
        })
    });
}

fn bench_put_blob_dedup(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = make_store();
    let payload = Bytes::from(vec![42u8; 1024]);
    rt.block_on(async { store.put_blob(payload.clone()).await.unwrap() });
    c.bench_function("put_blob_dedup", |b| {
        b.iter(|| {
            let p = payload.clone();
            rt.block_on(async { store.put_blob(p).await.unwrap() })
        })
    });
}

fn bench_get_blob(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = make_store();
    let payload = Bytes::from(vec![42u8; 1024]);
    let id = rt.block_on(async { store.put_blob(payload).await.unwrap() });
    c.bench_function("get_blob", |b| {
        b.iter(|| rt.block_on(async { store.get(&id).await.unwrap() }))
    });
}

criterion_group!(benches, bench_put_blob_cold, bench_put_blob_dedup, bench_get_blob);
criterion_main!(benches);
```

- [ ] **Step 5: Write `benches/snapshot_ops.rs`**

```rust
// <bead-id>
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
```

- [ ] **Step 6: Write `benches/git_projection.rs`**

```rust
// <bead-id>
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
```

- [ ] **Step 7: Verify all benchmarks compile and run**

```bash
cargo bench --no-run
```

Expected: compilation succeeds, all 3 bench binaries built. No output beyond "Compiling".

Then do a quick single-iteration smoke run (fast, not a real measurement):

```bash
cargo bench -- --test
```

Expected: each benchmark function prints "test bench_name ... ok" (criterion's `--test` mode runs one iteration and passes if it doesn't panic).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml benches/object_store.rs benches/snapshot_ops.rs benches/git_projection.rs
git commit -m "<bead-id>: G8-T1 criterion benchmark scaffold"
```

- [ ] **Step 9: Merge and close**

```bash
git checkout master
git merge <bead-id>
git branch -d <bead-id>
bd close <id>
```

---

### Task 2: T8 Storage Deduplication Test

**Files:**
- Create: `tests/scale.rs`

**Interfaces:**
- Consumes: `bole::object::{EntryKind, Snapshot, TreeEntry}`, `bole::refs::{RefName, TimelinePolicy}`, `bole::{Accessor, PathRole, Permission, Repository, TimelineRole}`, `bole::store::disk::DiskBackend`, `tempfile::TempDir`
- Produces: `cargo test --test scale` passes 2 tests: `t8_object_count_dedup` and `t8_disk_storage_footprint`

- [ ] **Step 1: Create a bead and branch**

```bash
bd create --title="G8-T2: T8 storage deduplication tests" \
  --description="Add tests/scale.rs with two sub-tests: t8_object_count_dedup (MemoryBackend, 1000 snapshots, verifies unique object count ≤ naive/3) and t8_disk_storage_footprint (DiskBackend, 100 snapshots, verifies unique object count ≤ naive/3 on disk)." \
  --type=task --priority=2
# Note the printed ID (e.g. bole-xyz)
bd update <id> --claim
git checkout -b <id>
```

- [ ] **Step 2: Write the failing tests first**

Create `tests/scale.rs` with test stubs that will fail (no implementation needed — the assertions are what we're testing):

```rust
// <bead-id>
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

fn full_write_accessor() -> Accessor {
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
}

/// Walk a directory tree, summing file sizes. Used to measure DiskBackend footprint.
fn dir_bytes(dir: &std::path::Path) -> usize {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += dir_bytes(&path);
            } else if let Ok(meta) = fs::metadata(&path) {
                total += meta.len() as usize;
            }
        }
    }
    total
}

/// T8 sub-test 1: Object-count dedup with MemoryBackend.
///
/// 1000 snapshots, each changing exactly 1 of 10 files (rotating).
/// Expected unique objects ≈ 3010:
///   - 10 initial blobs + 1000 change blobs = 1010 blobs
///   - 1001 root tree objects (initial + one per snapshot)
///   - 1001 snapshot objects (initial + 1000)
///
/// Naive (no sharing) = 1000 × 10 + 1000 + 1000 = 12000 objects.
/// Assertion: unique_objects ≤ naive / 3 (i.e., at least 66% reduction).
#[tokio::test]
async fn t8_object_count_dedup() {
    todo!()
}

/// T8 sub-test 2: Disk storage footprint with DiskBackend.
///
/// 100 snapshots, each changing exactly 1 of 10 files.
/// Measures actual bytes written to disk (objects directory).
/// Naive disk bytes = 100 snapshots × 10 blobs × avg_blob_size_compressed (no sharing).
/// Actual disk bytes should be much less because unchanged blobs are shared.
///
/// Assertion: disk_bytes ≤ naive_disk_bytes / 3.
#[tokio::test]
async fn t8_disk_storage_footprint() {
    todo!()
}
```

- [ ] **Step 3: Run to confirm tests fail (as expected)**

```bash
cargo test --test scale 2>&1 | tail -10
```

Expected: 2 tests fail with "not yet implemented" (the `todo!()` panics). This confirms the test harness is wired up correctly.

- [ ] **Step 4: Implement `t8_object_count_dedup`**

Replace the `todo!()` in `t8_object_count_dedup` with the full implementation:

```rust
#[tokio::test]
async fn t8_object_count_dedup() {
    let repo = Repository::memory();

    // Initial 10 files with unique content per file
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for i in 0..10usize {
        let content = format!("file{i} initial content {:0>200}", i);
        let blob = repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        entries.insert(format!("file{i}.txt"), TreeEntry { id: blob, kind: EntryKind::Blob });
    }
    let init_tree = repo.objects.put_tree(entries.clone()).await.unwrap();
    let mut prev_snap = repo.objects.put_snapshot(Snapshot {
        root: init_tree,
        parents: vec![],
        author: "bench".into(),
        created_at: 0,
        message: "initial".into(),
    }).await.unwrap();

    // 1000 snapshots: snapshot N changes file N%10
    for n in 1usize..=1000 {
        let file_idx = n % 10;
        let content = format!("file{file_idx} version {n}");
        let changed_blob = repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        entries.insert(
            format!("file{file_idx}.txt"),
            TreeEntry { id: changed_blob, kind: EntryKind::Blob },
        );
        let tree = repo.objects.put_tree(entries.clone()).await.unwrap();
        prev_snap = repo.objects.put_snapshot(Snapshot {
            root: tree,
            parents: vec![prev_snap],
            author: "bench".into(),
            created_at: n as u64,
            message: format!("snapshot {n}"),
        }).await.unwrap();
    }

    let unique_objects = repo.objects.list().await.unwrap().len();
    // Naive: 1000 snapshots × 10 blobs + 1000 trees + 1000 snapshots = 12000
    // (plus 1 initial tree + 1 initial snapshot = 12002, but 12000 is the round bound)
    let naive_objects = 12_000usize;
    assert!(
        unique_objects <= naive_objects / 3,
        "dedup insufficient: {unique_objects} unique objects stored, expected ≤ {} (naive={naive_objects})",
        naive_objects / 3
    );
}
```

- [ ] **Step 5: Implement `t8_disk_storage_footprint`**

Replace the `todo!()` in `t8_disk_storage_footprint` with the full implementation:

```rust
#[tokio::test]
async fn t8_disk_storage_footprint() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::disk(dir.path()).await.unwrap();

    // Initial 10 files, 256 bytes each
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for i in 0..10usize {
        let content = format!("{:0>256}", i); // 256-byte string
        let blob = repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        entries.insert(format!("file{i}.txt"), TreeEntry { id: blob, kind: EntryKind::Blob });
    }
    let init_tree = repo.objects.put_tree(entries.clone()).await.unwrap();
    let mut prev_snap = repo.objects.put_snapshot(Snapshot {
        root: init_tree,
        parents: vec![],
        author: "bench".into(),
        created_at: 0,
        message: "initial".into(),
    }).await.unwrap();

    // 100 snapshots: snapshot N changes file N%10
    for n in 1usize..=100 {
        let file_idx = n % 10;
        let content = format!("file{file_idx} v{n}");
        let changed_blob = repo.objects.put_blob(Bytes::from(content)).await.unwrap();
        entries.insert(
            format!("file{file_idx}.txt"),
            TreeEntry { id: changed_blob, kind: EntryKind::Blob },
        );
        let tree = repo.objects.put_tree(entries.clone()).await.unwrap();
        prev_snap = repo.objects.put_snapshot(Snapshot {
            root: tree,
            parents: vec![prev_snap],
            author: "bench".into(),
            created_at: n as u64,
            message: format!("snapshot {n}"),
        }).await.unwrap();
    }

    // Count unique objects on disk
    let unique_on_disk = repo.objects.list().await.unwrap().len();

    // Naive: 100 snapshots × 10 blobs + 100 trees + 100 snapshots = 1200
    // (plus 1 initial tree + 1 initial snapshot = 1202, but 1200 is the round bound)
    let naive_objects = 1_200usize;
    assert!(
        unique_on_disk <= naive_objects / 3,
        "disk dedup insufficient: {unique_on_disk} unique object files, expected ≤ {} (naive={naive_objects})",
        naive_objects / 3
    );

    // Additionally: total bytes on disk should not be wildly large.
    // Raw content bytes = 10 × 256 + 100 × ~12 = 3760 bytes.
    // With zstd framing and tree/snapshot metadata, allow up to 20× raw content.
    let disk_bytes = dir_bytes(&dir.path().join("objects"));
    // 10 initial files × 256 bytes + 100 change blobs × 12 bytes (format!("file{x} v{n}") ≈ 9-10 chars; 12 is a conservative ceiling)
    let raw_content_bytes = 10 * 256 + 100 * 12;
    assert!(
        disk_bytes <= 20 * raw_content_bytes,
        "disk bytes too large: {disk_bytes} bytes, raw content baseline {raw_content_bytes} bytes (ratio {:.1}×)",
        disk_bytes as f64 / raw_content_bytes as f64
    );
}
```

- [ ] **Step 6: Run the tests**

```bash
cargo test --test scale -- --nocapture 2>&1
```

Expected output (example):

```
running 2 tests
test t8_object_count_dedup ... ok
test t8_disk_storage_footprint ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

If `t8_object_count_dedup` fails with "dedup insufficient: N unique objects stored, expected ≤ 4000", the count logic is wrong — re-check that you're using `entries.clone()` correctly so unchanged files reuse their existing `TreeEntry` (same blob ObjectId).

If `t8_disk_storage_footprint` fails the object-count assertion: same issue as above.

If it fails the bytes assertion, print `disk_bytes` and `raw_content_bytes` to see the actual ratio.

- [ ] **Step 7: Run the full test suite to verify no regressions**

```bash
cargo test 2>&1 | grep "test result"
```

Expected: all lines show `ok. N passed; 0 failed`.

- [ ] **Step 8: Commit**

```bash
git add tests/scale.rs
git commit -m "<bead-id>: G8-T2 T8 storage deduplication tests"
```

- [ ] **Step 9: Merge and close**

```bash
git checkout master
git merge <bead-id>
git branch -d <bead-id>
bd close <id>
```

---

## After All Tasks: Record Baselines

After both tasks are merged, run the benchmarks on the target machine and record the baseline:

```bash
cargo bench -- --save-baseline gate8
```

Criterion writes baseline data to `target/criterion/`. Copy the mean values from each benchmark's output into the **Baselines** table in `docs/superpowers/specs/2026-06-28-gate8-performance-scale-design.md`.

Future runs compare against this baseline:

```bash
cargo bench -- --baseline gate8
```
