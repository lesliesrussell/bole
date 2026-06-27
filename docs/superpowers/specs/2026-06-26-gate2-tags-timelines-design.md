# Gate 2: Tags and Timelines

**Project:** bole — a next-generation version control system  
**Language:** Rust (sync RefBackend, async ObjectStore from Gate 1)  
**Date:** 2026-06-26  
**Spec ref:** spec.md Gate 2, Test T2

---

## Context

Gate 1 delivered a content-addressed object store (`ObjectStore`, `ObjectId`, `Blob`, `Tree`, `Snapshot`, `MemoryBackend`, `DiskBackend`). Everything in the object store is immutable — content-addressed by BLAKE3 hash.

Gate 2 adds the mutable reference layer: **Tags** (named pointers to Snapshots) and **Timelines** (ordered sequences of Snapshots with a policy). Both are cheap reference moves — no data copying, no new objects in the object store.

Key architectural decisions made during brainstorming:
- **Separate `RefStore`** — independent of `ObjectStore`, composed by callers (not a `Repository` wrapper)
- **`RefBackend` is sync** — refs are tiny (64 bytes + small string), async adds complexity with no real benefit
- **Hierarchical `RefName`** — `/` is a namespace separator, validated at construction
- **Policy stored, not enforced** — `TimelinePolicy` is metadata in Gate 2; enforcement requires DAG traversal and belongs in Gate 6

---

## Core Types

```rust
// Validated hierarchical path: "v1", "experiment/foo", "leslie/private/exp-foo"
// Rules: non-empty, no leading/trailing "/", no ".." segments, no null bytes
pub struct RefName(String);

impl RefName {
    pub fn new(s: impl Into<String>) -> Result<Self>;
    pub fn as_str(&self) -> &str;
    pub fn prefix(&self) -> &str; // everything before the last "/"
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub target: ObjectId,        // points to a Snapshot ObjectId
    pub created_at: u64,         // unix timestamp
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimelinePolicy {
    FastForwardOnly,  // head can only advance, no rewrites
    Append,           // new snapshot must have current head as direct parent
    Unrestricted,     // any snapshot can become the new head
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub head: ObjectId,
    pub policy: TimelinePolicy,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Ref {
    Tag(Tag),
    Timeline(Timeline),
}
```

Tags and Timelines share the same `RefName` namespace. All types carry serde derives for postcard serialization (consistent with Gate 1).

**Policy note:** `TimelinePolicy` is stored as intent but not enforced in Gate 2. `advance_head` accepts any `ObjectId` as the new head. Enforcement — which requires walking the Snapshot DAG via the `ObjectStore` — is deferred to Gate 6.

---

## Error Types

Two new variants added to the existing `Error` enum in `src/error.rs`:

```rust
#[error("invalid ref name: {0}")]
InvalidRefName(String),

#[error("wrong ref kind: {0}")]
WrongRefKind(String),
```

`InvalidRefName` — returned by `RefName::new` on bad input.  
`WrongRefKind` — returned when `move_tag` is called on a Timeline, or `advance_head` on a Tag.

---

## RefBackend Trait

Sync trait — no async overhead for 64-byte operations:

```rust
pub trait RefBackend: Send + Sync {
    fn get(&self, name: &RefName) -> Result<Option<Ref>>;
    fn set(&self, name: &RefName, r: &Ref) -> Result<()>;
    fn delete(&self, name: &RefName) -> Result<()>;
    fn list(&self, prefix: &str) -> Result<Vec<RefName>>;
}
```

### MemoryRefBackend

`Arc<RwLock<HashMap<String, Ref>>>` — mirrors `MemoryBackend` from Gate 1. Clone-safe. No persistence.

### DiskRefBackend

One file per ref. Path mirrors the ref name hierarchy:
- `experiment/foo` → `<root>/refs/experiment/foo`
- Contents: postcard-encoded `Ref`
- `list(prefix)` walks the directory tree under `<root>/refs/<prefix>`
- No compression (refs are tiny — overhead not justified)

`DiskRefBackend::open(root: impl AsRef<Path>) -> Result<Self>` — creates root dir on first use (sync, uses `std::fs`).

---

## RefStore API

```rust
pub struct RefStore {
    backend: Box<dyn RefBackend>,
}

impl RefStore {
    pub fn new(backend: impl RefBackend + 'static) -> Self;

    // Tags
    pub fn create_tag(&self, name: RefName, target: ObjectId,
                      message: Option<String>, now: u64) -> Result<()>;
    pub fn move_tag(&self, name: &RefName, target: ObjectId) -> Result<()>;
    pub fn get_tag(&self, name: &RefName) -> Result<Option<Tag>>;

    // Timelines
    pub fn create_timeline(&self, name: RefName, head: ObjectId,
                           policy: TimelinePolicy, now: u64) -> Result<()>;
    pub fn advance_head(&self, name: &RefName, new_head: ObjectId) -> Result<()>;
    pub fn get_timeline(&self, name: &RefName) -> Result<Option<Timeline>>;

    // Shared
    pub fn get(&self, name: &RefName) -> Result<Option<Ref>>;
    pub fn delete_ref(&self, name: &RefName) -> Result<()>;
    pub fn list(&self, prefix: &str) -> Result<Vec<RefName>>;
}
```

`delete_ref` covers both Tags and Timelines — same operation regardless of kind.  
`move_tag` and `advance_head` return `Error::WrongRefKind` if the named ref is the wrong type.  
`now: u64` is passed in (not read from `SystemTime`) — keeps the library pure and testable.

---

## Crate Structure

```
src/
├── lib.rs              # add: pub mod refs + re-exports
├── error.rs            # add: InvalidRefName, WrongRefKind variants
├── refs/
│   ├── mod.rs          # RefStore + re-exports
│   ├── name.rs         # RefName (validated hierarchical path)
│   ├── tag.rs          # Tag struct
│   ├── timeline.rs     # Timeline + TimelinePolicy
│   ├── ref_type.rs     # Ref enum (Tag | Timeline)
│   ├── backend.rs      # RefBackend sync trait
│   ├── memory.rs       # MemoryRefBackend
│   └── disk.rs         # DiskRefBackend
└── (object/, store/, codec.rs unchanged)

tests/
└── refs.rs             # integration tests (spec T2)
```

---

## Testing Approach

Tests map directly to spec T2. Both backends share a contract suite:

```rust
fn run_ref_suite(store: RefStore) {
    // tag create + move
    // timeline create + advance
    // list by prefix
    // wrong kind errors
}

#[test] fn memory_refs() { run_ref_suite(RefStore::new(MemoryRefBackend::new())) }
#[test] fn disk_refs()   { run_ref_suite(RefStore::new(DiskRefBackend::open(tmp).unwrap())) }
```

**Spec T2 test cases:**

- **t2_tag_create_and_move** — create `v1` and `experiment/foo`, move `experiment/foo` to new ObjectId; assert `v1` unchanged, `experiment/foo` updated; no new objects in store
- **t2_timeline_head_advances** — create `main` timeline, advance head S1→S2→S3; assert head is S3
- **t2_list_by_prefix** — create `leslie/exp-a`, `leslie/exp-b`, `v1`; list `leslie/` returns exactly 2
- **wrong_ref_kind_errors** — call `move_tag` on a Timeline; assert `WrongRefKind` error
- **ref_name_validation** — `RefName::new("")` errors, `RefName::new("a//b")` errors, `RefName::new("../etc")` errors
- **delete_ref** — create tag, delete it, get returns None
- **disk_persists_across_reopen** — write refs, reopen `DiskRefBackend`, assert refs still present

---

## Key Dependencies

No new crates required. All dependencies from Gate 1 apply:

| Crate | Gate 2 use |
|-------|-----------|
| `serde` + `postcard` | serialize `Ref` for disk storage |
| `thiserror` | new error variants |
| `std::sync::RwLock` | `MemoryRefBackend` interior mutability |
| `std::fs` | `DiskRefBackend` (sync I/O) |

---

## Out of Scope (Gate 2)

- `TimelinePolicy` enforcement (requires ObjectStore DAG traversal — Gate 6)
- Permission checks on ref names (Gate 3)
- `Secret` and `EnvOverlay` refs (Gate 4)
- Merge semantics / conflict resolution (Gate 6)
- Git projection of refs (Gate 7)
