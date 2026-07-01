# bole — access-controlled version control for multi-actor workflows

bole is a version control library for multi-actor, access-controlled workflows. Unlike Git — where access control lives outside the repository in a hosting platform or filesystem permissions — bole encodes actor identities, visibility labels, and operation policies as first-class objects in the same content-addressed store as your files and history. That means a real **label lattice** with scoped clearances, **ACL-filtered snapshot views**, **policy-hook-gated** timeline advancement and merges, and **envelope-encrypted secrets** stored alongside source files with access gated by the same rules — making bole a foundation for agent-safe workflows where every actor's capability is declared, enforced, and auditable without a separate service. Distributed sync (content-addressed pack transfer with compare-and-swap on heads, over TCP or HTTP), signed policy verification with a per-repo trust store, and a git import/export round-trip make it interoperable; TLS/SSH transports and cloud KMS adapters are the remaining incremental work.

## What bole is

The access model — actors, labels, and policy — is the reason for the design. The snapshot/timeline model below is the mechanism.

- **Actors and access** — named actors carry labeled grants (path globs, timeline patterns) evaluated against a bounded label lattice with scoped clearances; access-controlled views filter what an actor can see. An automated agent and a human developer are the same concept — just different grant sets.
- **Timelines with policy** — named movable pointers with configurable advancement policies (`ff`, `append`, `unrestricted`) and programmable `PolicyHook`s (e.g. approval-gated merge into `release/**`) enforced at the API boundary.
- **Secrets and env overlays** — envelope-encrypted values and environment bundles stored as content-addressed objects, access-gated by the same actor model, never committed as plaintext.
- **Snapshots** — the only durable state: immutable typed file trees plus metadata. Every operation produces a new snapshot; nothing is rewritten.
- **Tags** — fixed named pointers to a snapshot.

bole is primarily a **library crate**: it provides the data model, access engine, and storage layer, and you build applications and tools on top. A command-line interface (`bole-cli`, binary `bole`) ships alongside it as a thin wrapper over the library — see the [CLI reference](docs/CLI.md).

## Capabilities

| Capability | Description |
|-----------|-------------|
| Content-addressed store | Immutable snapshots; identical content is stored once; BLAKE3-verified integrity |
| Timelines and tags | Named history views with configurable advancement policy |
| Label lattice + clearances | Bounded partial-order labels, scoped actor clearances, ACL-filtered snapshot views; glob ACLs are the degenerate two-point case |
| PolicyHook | Programmable, content-addressed policy gating `advance`/`merge` (e.g. N approvals for `release/**`) |
| Secrets and env overlays | Envelope-encrypted typed objects (per-secret data key wrapped by a master key) in the same store; env bundles mixing plain and secret values |
| Packs + GC | Immutable pack format (disk + wire payload), mark-sweep GC from refs, atomic multi-ref transactions |
| Distributed sync | Negotiated pack transfer (missing-closure) with CAS-on-heads fetch/push/clone |
| Git interop | One-way ACL-filtered export to a bare Git repo, and git → bole import with an identity map for round-trips |
| Pluggable storage | In-memory (agents, tests) and packed disk-backed (CLI) backends behind one interface |

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

## Status and roadmap

The design direction is realized in library form. This table is the canonical honesty record.

| Capability | Today | Notes / follow-up |
|-----------|-------|-------------------|
| Content-addressed object store, timelines, tags, ff/append/unrestricted policy | Realized | — |
| Real label lattice + scoped clearances (glob ACLs as degenerate case) | Realized | WS1 (`bole-fo2`) |
| PolicyHook — policy-gated merge/advance | Realized | WS1 (`bole-fo2`) |
| Signed approval attestations (Ed25519, head-bound) | Realized | `bole-fz1` |
| Workspace trait (in-memory + disk-backed) | Realized | WS2 (`bole-1kz`) |
| Envelope-encrypted secrets (per-secret data key + master key), `env resolve` / `run` / `secret rekey` | Realized | WS3 (`bole-9mz`) |
| KMS integration slot (`KmsClient` + `KmsKeyProvider`, feature `kms`) | Realized | `bole-vw9`; cloud/HSM adapters are third-party `KmsClient` impls |
| Pack format + mark-sweep GC + atomic multi-ref transactions | Realized | WS4 (`bole-81z`) + `bole-sk6` |
| Distributed sync — fetch/push/clone with CAS on heads | Realized | WS5 (`bole-cy6`) |
| Networked transports — TCP + minimal HTTP over the sync session | Realized | `bole-6qy` (wire/session) + `bole-vih` (TCP/HTTP); TLS/SSH are further work |
| Policy authority — signed `PolicyRoot` chain, `TrustStore`, highest-rooted-wins | Realized | `bole-0tp` |
| Sync authn — principal → actor → `Accessor`, signed refs | Realized | `bole-6h7` |
| Git import / round-trip + identity map, `bole git import` | Realized | WS6 (`bole-mtq`) + `bole-58u` (CLI) |
| Linked-worktree hardening (`prune`/`repair`/`list --check`) | Realized | WS7 (`bole-3hj`) |

Present-tense claims above describe what runs today. Remaining open work is
incremental: TLS/proxy-grade HTTP and an SSH transport, cloud/HSM `KmsClient`
adapters, and deeper sync CLI porcelain.

## License

Licensed under the [MIT License](LICENSE).
