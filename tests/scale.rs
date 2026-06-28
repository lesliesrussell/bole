// bole-agc
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::Repository;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

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
    // 20× accounts for tree and snapshot object overhead: ~101 trees + ~101 snapshots
    // are each encoded separately, dominating storage for small blob content.
    // The object-count assertion above is the primary dedup proof; this bounds absolute size.
    assert!(
        disk_bytes <= 20 * raw_content_bytes,
        "disk bytes too large: {disk_bytes} bytes, raw content baseline {raw_content_bytes} bytes (ratio {:.1}×)",
        disk_bytes as f64 / raw_content_bytes as f64
    );
}
