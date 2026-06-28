# Gate 8: Performance and Scale

**Project:** bole — a next-generation version control system
**Language:** Rust (async, tokio)
**Date:** 2026-06-28
**Spec ref:** spec.md Gate 8 (G8), Test T8

---

## Context

Gates 1–7 delivered content-addressed object storage, mutable references, granular ACL enforcement, secrets and env overlays, pluggable backends, multi-actor merge semantics, and git projection. Gate 8 adds measurement and verification: a criterion benchmark suite for key operations and the T8 storage deduplication test that proves structural sharing works at scale.

**Scope decisions:**
- **No pack files** — the existing loose-object layout (one zstd file per object, sharded by 2-hex prefix) is sufficient for the scale targets. Pack files are deferred to a future gate.
- **No read cache or other optimizations** — Gate 8 measures the existing implementation and records baselines. Optimization is a follow-up bead if benchmarks reveal unacceptable numbers.
- **No new public API** — all new code lives in `benches/` and `tests/scale.rs`.

---

## Deduplication Guarantee

The content-addressed design (BLAKE3) provides automatic structural sharing:
- Two blobs with identical bytes → one stored object, any number of referencing trees.
- A subtree that doesn't change across snapshots → same `ObjectId`, stored once.
- A snapshot that changes 1 of 10 files → 9 blobs shared, 1 new blob, 1 new root tree, 1 new snapshot object.

Gate 8 verifies this guarantee holds under load via T8.

---

## T8: Storage Deduplication Test

**File:** `tests/scale.rs`

### Sub-test 1: Object-count dedup (MemoryBackend)

Setup:
- 10 files in a flat tree, each containing ~256 bytes of unique initial content (`format!("file{i} initial content {:0>200}", i)`)
- 1000 snapshots in a linear chain (each parents the previous)
- Each snapshot changes exactly one file: snapshot N changes file `N % 10` with content `format!("file{file_idx} version {n}")`
- Uses `Repository::memory()` (no disk I/O, runs in CI without tempdir)

After all snapshots, call `repo.objects.list().await` and count unique objects.

Expected counts:
- 10 initial blobs (one per file)
- 1000 change blobs (one per snapshot, unique content)
- 1000 root tree objects (one per snapshot; root changes because one entry changed)
- 1000 snapshot objects
- Total unique ≈ 3010

Naive count (no sharing): `1000 snapshots × 10 blobs + 1000 trees + 1000 snapshots = 12000`

**Assertion:** `unique_objects ≤ naive_objects / 3`
(i.e., at least 66% reduction from structural sharing)

### Sub-test 2: Disk footprint (DiskBackend)

Setup:
- Same pattern as sub-test 1 but only 100 snapshots, using `DiskBackend` with `tempfile::TempDir`
- After all snapshots, measure total bytes on disk by walking the objects directory and summing file sizes (no `du` subprocess — use `std::fs::metadata`)

Raw content size = 10 initial blobs × 256 bytes + 100 change blobs × ~30 bytes ≈ 5560 bytes of unique blob content.

**Assertion:** `disk_bytes ≤ 20 × raw_content_bytes`

The 20× multiplier reflects actual overhead: zstd framing per small object, tree object encoding (BTreeMap serialized via postcard), snapshot object encoding, and the 2-level directory structure. The original 5× estimate was too optimistic — it counted only blob content bytes but did not account for the ~101 tree objects and ~101 snapshot objects that are each encoded and stored separately. For small-blob workloads, tree/snapshot metadata dominates storage and drives the ratio well above 5×. The object-count dedup assertion (`unique_on_disk ≤ naive_objects / 3`) is the primary structural-sharing proof; this assertion bounds absolute size.

---

## Benchmark Suite

**Harness:** `criterion` 0.5 with `html_reports` feature. All benchmarks use `MemoryBackend` to measure pure computation without disk I/O noise.

### `benches/object_store.rs`

Raw storage layer benchmarks.

| Benchmark | What it measures |
|---|---|
| `put_blob_cold` | `ObjectStore::put_blob(1KB payload)` — first write, no dedup |
| `put_blob_dedup` | Same 1KB payload twice — second call hits the exists-check fast path |
| `get_blob` | `ObjectStore::get(id)` after a single put |

### `benches/snapshot_ops.rs`

Repository layer benchmarks.

| Benchmark | What it measures |
|---|---|
| `put_snapshot_10files` | `put_blob` × 10 + `put_tree` + `put_snapshot` for a 10-file flat tree |
| `advance_timeline` | `Repository::advance_timeline` — ref update only, no object writes |
| `merge_timelines_clean` | `Repository::merge_timelines` on two non-conflicting 10-file trees |

### `benches/git_projection.rs`

Git export benchmarks.

| Benchmark | What it measures |
|---|---|
| `project_to_git_linear_10` | `project_to_git` — 1 timeline, 10 commits, flat 5-file tree |
| `project_to_git_linear_100` | Same but 100 commits — verifies O(n) scaling |

`project_to_git` writes to a `tempfile::TempDir` (disk I/O included — this is intentional, since projection is inherently a disk operation).

---

## Baseline Measurement Protocol

Gate 8 ships with a saved criterion baseline:

```bash
cargo bench -- --save-baseline gate8
```

After the first run on the reference machine, copy the measured means from criterion's JSON output into this spec (see **Baselines** section below). Future CI runs compare against the saved baseline:

```bash
cargo bench -- --baseline gate8
```

Criterion's default regression threshold (10%) serves as the gate. A benchmark regressing by > 10% fails the bench run.

**Baselines** (populated after first run):

| Benchmark | Mean (ns) | Std dev (ns) | Threshold (mean × 1.2) |
|---|---|---|---|
| `put_blob_cold` | TBD | TBD | TBD |
| `put_blob_dedup` | TBD | TBD | TBD |
| `get_blob` | TBD | TBD | TBD |
| `put_snapshot_10files` | TBD | TBD | TBD |
| `advance_timeline` | TBD | TBD | TBD |
| `merge_timelines_clean` | TBD | TBD | TBD |
| `project_to_git_linear_10` | TBD | TBD | TBD |
| `project_to_git_linear_100` | TBD | TBD | TBD |

*Reference machine:* TBD (recorded after first run).

---

## Cargo Changes

```toml
# Cargo.toml — add to [dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

# Cargo.toml — add bench entries
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

---

## New Files

```
benches/
├── object_store.rs      # put/get/dedup benchmarks
├── snapshot_ops.rs      # put_snapshot, advance_timeline, merge benchmarks
└── git_projection.rs    # project_to_git benchmarks

tests/
└── scale.rs             # T8: object-count dedup + disk footprint tests
```

No changes to `src/`.

---

## Out of Scope (Gate 8)

- Pack file format
- Read cache / LRU
- Async-aware zstd (eliminating `spawn_blocking` per object)
- Remote operation latency
- CLI ergonomics
- Any optimization work — Gate 8 measures; optimization is a follow-up bead if needed
