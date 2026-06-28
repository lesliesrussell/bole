# bole

A next-generation version control library crate for Rust, designed for fine-grained visibility, pluggable storage, typed secrets, multi-actor workflows, and backward-compatible Git export.

## What bole is

bole reimagines the core VCS abstraction. Instead of "files in a directory plus a commit DAG," bole's primitives are:

- **Snapshots** — the only durable state unit. A snapshot is a typed map from logical paths to content IDs, plus metadata (author, timestamp, parents). Every state change — commit, merge, agent action — produces a new snapshot. Nothing is mutable.
- **Timelines** — ordered views over the snapshot DAG. A timeline is a named pointer (like a branch) with a configurable policy for how new snapshots are added.
- **Tags** — lightweight named pointers to a snapshot or timeline head.
- **ACLs** — every path and timeline participates in an access-control lattice. Private files, private timelines, and policy-driven merges are first-class, not conventions layered on top.
- **Secrets and env overlays** — typed object-graph nodes with separate encryption and visibility. A workspace view is computed as base files + env overlays + secret bindings; `.env` files are never committed.

bole is primarily a **library crate**: it provides the data model and storage layer, and you build applications and tools on top. A command-line interface (`bole-cli`, binary `bole`) ships alongside it as a thin wrapper over the library — see the [CLI reference](docs/CLI.md).

## Features (Gates 1–8)

| Gate | Feature |
|------|---------|
| G1 | Content-addressed object store (BLAKE3), immutable snapshots |
| G2 | Tags and timelines as movable references |
| G3 | Granular ACLs — private paths, private timelines, policy-driven merge |
| G4 | Secrets (`chacha20poly1305`) and env overlays as first-class typed nodes |
| G5 | Pluggable storage backends — `MemoryBackend` and `DiskBackend` |
| G6 | Multi-actor workflows — ephemeral timelines, agent capability enforcement |
| G7 | Git projection — export a filtered timeline to a bare Git repo |
| G8 | Storage deduplication verified, criterion benchmark suite |

## Quick Start

```rust
use bole::{Repository, Accessor};
use bytes::Bytes;

#[tokio::main]
async fn main() {
    // In-memory repo — no disk I/O
    let repo = Repository::memory();

    // Full-access accessor (owns everything)
    let accessor = Accessor::privileged();

    // Store a blob and build a snapshot
    let blob_id = repo.objects.put_blob(Bytes::from("hello, world")).await.unwrap();

    use bole::{Snapshot, TreeEntry, EntryKind};
    use std::collections::BTreeMap;

    let mut entries = BTreeMap::new();
    entries.insert("src/main.rs".to_string(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();

    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id,
        parents: vec![],
        author: "alice".to_string(),
        created_at: 1_700_000_000,
        message: "initial commit".to_string(),
    }).await.unwrap();

    // Create a timeline pointing at the snapshot
    use bole::refs::{RefName, TimelinePolicy};
    let name = RefName::new("main").unwrap();
    repo.refs.create_timeline(
        name, snap_id, TimelinePolicy::Unrestricted, 1_700_000_000,
        "persistent".to_string(), None,
    ).unwrap();

    println!("snapshot: {snap_id}");
}
```

### Disk-backed repo

```rust
use bole::{Repository, DiskBackend, ObjectStore};

let repo = Repository::disk("/path/to/repo").await?;
```

### ACL-filtered snapshot

```rust
use bole::{Accessor, PathRole, Permission};

// Accessor that can only read src/**
let accessor = Accessor::new()
    .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Read });

let view = repo.get_snapshot_filtered(snap_id, &accessor).await?.unwrap();
// view.visible_paths contains only paths matching src/**
```

### Git export

```rust
use bole::repo::git_projection::project_to_git;
use std::path::Path;

project_to_git(&repo, Path::new("/tmp/out.git"), &Accessor::privileged()).await?;
// /tmp/out.git is now a valid bare Git repo
```

### Secrets

```rust
let key: [u8; 32] = /* ... */;
let secret_id = repo.objects.put_secret(b"postgres://prod:secret@db/app", &key).await?;
let plaintext = repo.objects.get_secret(&secret_id, &key).await?.unwrap();
```

## CLI quick start

```bash
cargo build --release -p bole-cli      # binary at target/release/bole

bole init .
echo 'fn main() {}' > src/main.rs
SNAP=$(bole snapshot create --from-workspace -m "initial" --json | jq -r .snapshot)
bole workspace open main --create --from "$SNAP"
# edit files...
bole snapshot create --from-workspace -m "changes"   # advances main
bole snapshot list
bole git export --to /tmp/export.git
```

See the [CLI reference](docs/CLI.md) for the full command tree (timelines, tags,
snapshots, workspace, merge, actors, ACLs, secrets, env overlays, git export,
and object/ref/store plumbing).

## Architecture

```
Repository
├── objects: ObjectStore          content-addressed blob/tree/snapshot/secret/env store
│   ├── MemoryBackend             HashMap<ObjectId, Bytes> — fast, ephemeral
│   └── DiskBackend               zstd-compressed loose objects, sharded by 2-hex prefix
├── refs: RefStore                named references (timelines, tags)
│   ├── MemoryRefBackend
│   └── DiskRefBackend
└── acls: AclStore                path and timeline ACL rules
    ├── MemoryAclBackend
    └── DiskAclBackend            URL-encoded filenames under acls/{paths,timelines}/
```

Objects are content-addressed using BLAKE3. An `ObjectId` is a 32-byte hash, displayed as 64 lowercase hex characters. Identical content always produces the same `ObjectId`; no object is ever rewritten.

Secrets use ChaCha20-Poly1305 with a random 96-bit nonce per write, so two `put_secret` calls with identical plaintext produce different `ObjectId`s — no equality leakage through the store.

## Storage layout (DiskBackend)

```
<root>/
├── objects/
│   └── <2-hex>/          shard directory (256 shards)
│       └── <62-hex>      zstd-compressed serialized Object
├── refs/
│   ├── timelines/
│   └── tags/
└── acls/
    ├── paths/
    └── timelines/
```

## Cargo

```toml
[dependencies]
bole = { path = "." }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
bytes = "1"
```

## Running tests

```bash
cargo test
```

## Running benchmarks

```bash
cargo bench
```

Benchmarks cover object store put/get, snapshot operations, and git projection at 10 and 100 commits. Results are saved as a criterion baseline (`--save-baseline gate8`).

## License

Licensed under the [MIT License](LICENSE).
