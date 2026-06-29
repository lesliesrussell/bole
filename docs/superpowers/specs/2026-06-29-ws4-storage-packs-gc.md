# WS4 — Storage at Scale: Packs, Index, GC, and Atomic Refs

- **Bead:** `bole-81z`
- **Depends on:** none
- **References:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md)
  — this spec realises the "Vocabulary — storage & transfer" section, and its
  pack format is the on-wire payload that WS5 (`bole-cy6`) consumes. Where this
  spec refines shared vocabulary it says so explicitly.

This is a **design spec**, not an implementation plan.

---

## 1. Goal

The current `DiskBackend` (`src/store/disk.rs`) stores every object as an
individual zstd-compressed loose file sharded by a 2-hex prefix. This is correct
and crash-safe (tmp-write + rename) but does not scale:

- **Inode pressure.** One file per object; a repo with millions of
  blobs/trees/snapshots/secrets exhausts inodes and directory-entry caches.
- **`list()` is O(all objects).** `DiskBackend::list()` does a `readdir` over
  256 shard directories and stats every entry. `fsck`/stats walks everything.
- **No transfer story.** Loose objects are not a streamable unit; WS5 needs a
  single format that is both the on-disk layout and the on-wire payload.

WS4 introduces an **immutable pack format** (`.pack` + `.idx`), a **packed read
layer** that consults loose objects first then packs, **repack** to consolidate
loose objects, **mark-and-sweep GC** from refs, **cheap counts/listing**, and an
**atomic multi-ref transaction API**. The same pack format is designed to be the
WS5 transfer payload from day one.

Non-goals for v1: binary delta compression / thin packs (reserved in the format,
see §7 and §10); a network protocol (WS5 owns the wire, WS4 owns the payload
bytes); changing the object model (`Object` enum, `ObjectId` = BLAKE3 of postcard
bytes — both unchanged).

---

## 2. Design principles & invariants

These hold throughout and are the basis of the safety arguments:

1. **Objects are immutable and content-addressed.** `ObjectId =
   BLAKE3(postcard(Object))` (`src/codec.rs`, `src/object/id.rs`). The canonical
   bytes are the postcard serialisation; the id is their hash. Re-storing an
   object is idempotent; two writers that store the same content produce the same
   id and never conflict.
2. **One format on disk and on the wire.** A pack is a streamable, self-verifying
   sequence of object frames. Reading from disk and receiving from the network
   decode the *same* frame layout. (Foundations decision 2.)
3. **Packs are append-only artifacts, never mutated in place.** All "edits"
   (repack, GC) produce a *new* pack and atomically retire old ones. There is no
   in-pack rewrite, so a pack file is either wholly valid or absent.
4. **Storage is collectible, not corruptible.** Any interrupted or orphaned write
   yields either an invisible tmp file or a complete-but-unreferenced object. GC
   removes unreferenced objects; it can never remove a referenced one. (§4.)
5. **Refs need transactions; objects do not.** Refs are mutable named pointers,
   so multi-ref updates must be all-or-nothing (§6). Objects, being immutable and
   content-addressed, never need a transaction — `put` is idempotent and partial
   writes are harmless. (Foundations: "Atomic refs".)
6. **`StorageBackend` stays byte-level and backend-agnostic.** The trait
   (`src/store/backend.rs`) keeps mapping `ObjectId → canonical bytes`. Packs are
   an implementation detail of one disk backend; `MemoryBackend` is unaffected.

---

## 3. Pack format (`.pack`)

### 3.1 Rationale: per-object frames, not one big stream

The foundations doc describes "a zstd-compressed stream of encoded objects." We
realise that as a **stream of independently zstd-compressed frames**, one per
object. This single choice is what reconciles the two consumers:

- **Random-access reads (disk).** The `.idx` gives `(offset, len)`; a reader
  seeks to `offset`, reads exactly `len` bytes, and zstd-decodes that one frame.
  A single monolithic zstd stream would force decompression from the start of the
  pack to reach object *k* — fatal for random reads.
- **Streaming (wire).** The frame sequence is emitted and consumed in order. A
  receiver processes each frame as it arrives, verifies it by hashing, and never
  needs the index or a seek. The pack is producible and consumable as a pure
  byte stream.

The cost is losing cross-object zstd compression (no shared window/dictionary
across frames). Given content-addressing already deduplicates identical
blobs/trees/subtrees via structural sharing, and most objects compress well
individually, this is acceptable for v1. A per-pack trained zstd **dictionary**
is the natural future upgrade (§10, open question 2).

### 3.2 Byte layout

All multi-byte integers are little-endian. "varint" is LEB128 unsigned.

```
PACK FILE
┌─────────────────────────────────────────────────────────────┐
│ HEADER (fixed, 32 bytes)                                      │
│   magic        [8]  = "BOLEPACK"                              │
│   version      u32  = 1                                       │
│   flags        u32  (bit0 = has_dictionary; rest reserved=0) │
│   object_count u64                                            │
│   reserved     [8]  = 0                                       │
├─────────────────────────────────────────────────────────────┤
│ (optional) DICTIONARY BLOCK  — present iff flags.has_dictionary│
│   dict_len varint │ dict_bytes[dict_len]   (v1: never emitted)│
├─────────────────────────────────────────────────────────────┤
│ FRAME[0]                                                      │
│ FRAME[1]                                                      │
│ ...                                                           │
│ FRAME[object_count-1]                                         │
├─────────────────────────────────────────────────────────────┤
│ TRAILER (fixed, 40 bytes)                                     │
│   end_magic    [8]  = "BOLEPKND"                              │
│   pack_digest  [32] = BLAKE3(header .. last frame byte)       │
└─────────────────────────────────────────────────────────────┘

FRAME
┌─────────────────────────────────────────────────────────────┐
│ record_type   u8     (0x01 = object; 0x02 = delta, RESERVED) │
│ object_id     [32]   BLAKE3 of the canonical (uncompressed)  │
│                      object bytes                             │
│ uncompressed_len varint   (length of canonical object bytes) │
│ stored_len       varint   (length of the zstd frame below)   │
│ zstd_frame    [stored_len]  one self-contained zstd frame    │
└─────────────────────────────────────────────────────────────┘
```

Notes:

- **`object_id` is in the frame itself**, not only in the index. This is what
  makes the pack self-verifying on the wire: a streaming receiver with no index
  reads the frame header, decompresses `zstd_frame`, asserts `len ==
  uncompressed_len`, asserts `BLAKE3(bytes) == object_id`, then `postcard`-decodes
  to an `Object`. Any tampering or truncation is caught at the frame boundary.
- **`zstd_frame` is an independent zstd frame** (its own magic + checksum), so it
  decodes without reference to any other frame. The frame header is plaintext so a
  reader can skip to the next frame using `stored_len` without decompressing.
- **Object type is not duplicated** in the frame — the canonical bytes are the
  `postcard`-encoded `Object` enum, which already carries the variant tag. The
  pack layer stays object-type-agnostic, exactly like `DiskBackend` today.
- **`pack_digest` in the trailer** lets both a disk verifier and a streaming
  receiver confirm the whole pack arrived intact and unmodified. The `end_magic`
  doubles as an explicit end-of-stream marker for the wire.

### 3.3 Index format (`.idx`)

The index is a *derived* artifact: it can always be rebuilt by scanning the
`.pack` frames. It is laid out for `mmap` + binary search, sorted by `ObjectId`.

```
INDEX FILE
┌─────────────────────────────────────────────────────────────┐
│ magic        [8]  = "BOLEIDX\0"                              │
│ version      u32  = 1                                         │
│ object_count u32  = N                                         │
│ fanout       [256] u32   cumulative: fanout[b] = #objects     │
│                          whose id[0] <= b   (fanout[255]=N)   │
│ ids          [N][32]     ObjectIds, ascending                 │
│ offsets      [N] u64     offsets[i] = byte offset of FRAME for │
│                          ids[i] in the .pack                  │
│ lens         [N] u64     lens[i]    = total FRAME length      │
│ pack_digest  [32]        copy of the pack's trailer digest    │
│ idx_digest   [32]        BLAKE3(everything above)            │
└─────────────────────────────────────────────────────────────┘
```

Lookup for id `x`:
1. `lo = (x[0]==0) ? 0 : fanout[x[0]-1]`, `hi = fanout[x[0]]` — the fan-out
   narrows to the slice of ids sharing the first byte.
2. Binary search `ids[lo..hi]` for `x`. On hit at `i`, read `offsets[i]`,
   `lens[i]`.

All sections are fixed-width and contiguous, so the file can be `mmap`-ed and
indexed by pointer arithmetic with zero parsing. `pack_digest` binds the index to
exactly one pack; a mismatch means "rebuild the index." The fan-out table costs
1 KiB and bounds binary search to one byte-bucket.

### 3.4 On-disk directory layout

```
<repo>/
  objects/            existing loose store (unchanged: zstd files, 2-hex shards)
    ab/cdef…          loose objects (recent writes live here)
  packs/              NEW
    pack-<digest>.pack
    pack-<digest>.idx
    multi.midx        OPTIONAL (see §5.2)
  refs/               existing per-file refs (unchanged)
    .txn/             NEW: transaction journal + lock (see §6)
```

`<digest>` in the pack file name is the `pack_digest` (hex), giving packs
content-addressed, collision-free names and making "is this the same pack?"
trivial.

---

## 4. Read path & the packed storage layer

### 4.1 Where packs plug in

We add **one new `StorageBackend` implementation**, `PackedDiskBackend`, used by
`Repository::disk()` in place of the bare `DiskBackend`. It composes the existing
loose backend with a `PackSet`. The `StorageBackend` trait is **unchanged**
(except an optional `count()` default, §5), so `MemoryBackend` and all 247 tests
are untouched. (Internally `PackedDiskBackend` may reuse `DiskBackend` for the
loose half rather than reimplementing it.)

```
ObjectStore  (src/store/mod.rs, unchanged)
   └─ Box<dyn StorageBackend>
        ├─ MemoryBackend            (unchanged)
        ├─ DiskBackend              (unchanged; still usable directly)
        └─ PackedDiskBackend        (NEW)
             ├─ loose:  DiskBackend
             └─ packs:  PackSet { Vec<Pack>, optional MultiIndex }
```

`Pack` holds an `mmap` of its `.idx` (and lazily of its `.pack`) plus the parsed
header. `PackSet` is loaded once at `open()` by scanning `packs/*.idx`.

### 4.2 Operation semantics for `PackedDiskBackend`

The `StorageBackend` contract is "store/return *canonical bytes* by id." Both
`DiskBackend::get` and a pack read return the *decompressed* canonical
(`postcard`) bytes, so `ObjectStore` is none the wiser.

- **`get(id)`** — check loose first (`object_path`, the fast path for recent
  writes), else consult `PackSet`: fan-out + binary search each `.idx`
  (newest-first), on hit read the frame at `(offset, len)`, zstd-decode, return
  bytes. Returns `None` if absent everywhere. (Optionally verify
  `BLAKE3 == id`; see open question 11.)
- **`exists(id)`** — loose `try_exists` OR any `.idx` contains `id`. The index
  membership test touches no object data.
- **`put(id, data)`** — always writes a **loose** object (tmp + rename, as
  today). Packs are built only by repack. `put` stays idempotent and needs no
  transaction (invariant 5). If the id already exists loose *or* in a pack, `put`
  is a no-op.
- **`delete(id)`** — removes the **loose** copy only. Objects inside immutable
  packs are never individually deleted; they leave storage via repack/GC (§5,
  §4.3). `delete` on a pack-only object is a no-op (see open question 9).
- **`list()`** — union of loose ids (`readdir`) and every `.idx` `ids` table
  (sequential mmap scan, no per-object stat). See §5 for why this is acceptable
  and how `count()` avoids it entirely.

### 4.3 Why this read path is safe and fast

- **Loose-first** keeps writes O(1) and read-after-write correct without touching
  packs, and means a freshly written object is visible before any repack.
- **Index-only membership** turns `exists`/negotiation into memory scans over
  sorted id tables instead of filesystem stats.
- **Immutability** means a pack a reader has `mmap`-ed can never change under it;
  retiring a pack (§5/§4) only unlinks it after a fresh replacement is durable, so
  a reader either sees the old pack or the new one, never a half-written one.

---

## 5. repack & multiple packs

### 5.1 repack: loose → pack

`repack` consolidates loose objects into one immutable pack.

**When.** Manual `bole repack`; and automatically when the loose-object count
crosses a threshold (default 5 000 — open question 3) or as the first phase of
GC. Repack is advisory: correctness never depends on having run it.

**How (crash-safe sequence):**

1. Snapshot the current loose id set (`readdir`).
2. Stream those objects into `packs/.pack-<tmp>.pack`: for each, read loose bytes,
   re-frame (the loose file already holds zstd bytes of the *whole object*, but
   the pack frame uses its own per-object zstd frame, so decode-then-reframe; v1
   may simply recompress), appending frames; track `(id, offset, len)`.
3. Compute `pack_digest`, write the trailer, `fsync`.
4. Build the sorted `.idx` from the collected tuples, `fsync`.
5. Atomically `rename` both into `packs/pack-<digest>.{pack,idx}`; `fsync` the
   `packs/` dir.
6. Only now delete the loose objects that were packed (those in the step-1
   snapshot that are present in the new pack).

**Crash safety.** A crash before step 5 leaves only an ignored tmp pack (no
`.idx` in place → invisible to `PackSet`). A crash between 5 and 6 leaves objects
in *both* loose and pack form — harmless, because reads are loose-first and ids
are content addresses (dedup). A later repack/GC removes the redundant loose
copies. There is no window in which an object disappears.

### 5.2 Multiple packs & the multi-index

Packs are immutable, so repeated repacks accumulate packs. Two coexistence
strategies:

- **v1 (ship first): per-pack `.idx`, consulted newest-first.** `PackSet` holds
  `Vec<Pack>`; lookup probes each `.idx`. Cost is O(#packs · log N) per miss —
  fine for a handful of packs.
- **Phase 2 (optional now, recommended once #packs grows): a multi-pack-index
  `packs/multi.midx`.** A single sorted `ObjectId → (pack_ordinal, offset, len)`
  table over all packs (same fan-out + parallel-array layout as §3.3, with an
  extra `pack_ordinal` array and a list of member pack digests). One binary
  search resolves any object regardless of pack count. The `.midx` is derived and
  rebuilt by rename like any index. Whether to build it in v1 is open question 5.

A periodic **"repack-all"** (consolidate many packs + loose into one pack, retire
the rest) keeps `#packs` bounded; it reuses the §5.1 sequence with all packs'
objects as input and unlinks the superseded packs only after the new one is
durable.

---

## 6. GC — mark-and-sweep from refs

### 6.1 Roots

Reachability roots are every ref target in the `RefStore`
(`src/refs/mod.rs`):

- **Timeline heads** — `Timeline.head` for every timeline, *all kinds*. This
  explicitly includes `kind == "ephemeral"` open commits (foundations: "open
  ephemeral commits" are roots) — an ephemeral timeline's head and its ancestry
  are live until the timeline is deleted or expired-and-pruned.
- **Tag targets** — `Tag.target` for every tag.
- **A grace set** — every object whose backing file `mtime` is newer than
  `now − grace` (default grace 2 h, open question 4). This guards the
  write-before-ref-commit race (§6.4).

Expired ephemeral timelines (`expires_at < now`) are *candidates* for pruning,
but pruning the ref is a separate, explicit step (a ref transaction, §6); GC
treats whatever refs currently exist as authoritative roots.

### 6.2 Mark (closure over the object graph)

From each root id, traverse the object reference edges, accumulating a reachable
`HashSet<ObjectId>`:

| Object        | Outbound edges to follow                                  |
|---------------|----------------------------------------------------------|
| `Snapshot`    | `root` (a `Tree`) **and** every id in `parents`          |
| `Tree`        | every `TreeEntry.id` (recurse: `Tree` or leaf `Blob`)    |
| `EnvOverlay`  | every `EnvValue::Secret(id)`                              |
| `Blob`        | leaf — no edges                                           |
| `Secret`      | leaf — no edges                                           |

(Edges derived from `src/object/*`.) Traversal is a BFS/DFS with a visited set;
shared subtrees are visited once. Note the `EnvOverlay → Secret` edge: secrets
carry random nonces and are *not* deduplicated, so they are reachable **only**
through the overlay that references them — missing this edge would wrongly collect
live secrets (open question 8 tracks confirming no other secret roots exist).

### 6.3 Sweep

Any object present in storage (loose **or** in any pack) but absent from the
reachable set is garbage. Removal uses **repack-and-drop**, never in-place
mutation (invariant 3):

- **Loose garbage** → `delete` (unlink), subject to the grace window.
- **Packed garbage** → write a **fresh pack** containing only the *reachable*
  objects that currently live in packs (the §5.1 sequence), then retire (unlink)
  the old packs after the new pack + index are durable. Unreachable packed
  objects simply do not get copied forward. This is identical machinery to
  repack-all, filtered by reachability.

So GC = compute reachable set → rewrite packs keeping only reachable → unlink old
packs → unlink unreachable loose objects older than grace.

### 6.4 Safety argument

The claim from the foundations doc: *GC + content-addressing makes partial writes
collectible rather than corrupting.*

1. **Partial object writes are invisible or unreferenced.** A crashed loose write
   is a `.tmp` file never renamed → not an object at all. A crashed pack write
   lacks its renamed `.idx` → not in `PackSet`. A *completed* object that no ref
   points at is, by definition, unreferenced — exactly what GC collects. Neither
   case can corrupt history, because nothing in the reachable closure points at
   them.
2. **GC only deletes outside the reachable closure of committed refs.** A
   referenced object is reachable from a root and is therefore copied forward /
   never unlinked. Removing only unreachable objects cannot break any ref.
3. **Content-addressing makes deletion idempotent and recoverable.** If an object
   is wrongly thought dead and later needed, re-adding identical content
   reproduces the same id; there is no aliasing or partial-identity hazard.
4. **The write-then-ref race is closed by the grace window + ordering.** A writer
   may `put` objects, then commit the ref that references them. If GC runs between
   those two steps it must not collect the just-written (still-unreferenced)
   objects. The grace set (§6.1) keeps any object younger than `now − grace`,
   covering the gap. For strict multi-process safety, GC additionally takes the
   repo lock that serialises ref transactions (§6.5) so it computes the reachable
   set against a stable ref snapshot; CAS preconditions (§6) detect any lost
   update. The exact locking strength is open question 4.

### 6.5 Concurrency

GC and repack acquire the same repo-level lock used by ref transactions
(`refs/.txn/lock`, §6) for the *planning* phase (snapshot refs + compute
reachable set) and for the *retire* phase (unlink old packs). The
copy-forward write phase can run lock-free because it only *adds* a new pack.
This keeps GC mostly non-blocking while guaranteeing it never races a ref commit.

---

## 7. Cheap counts & listing

Today `store.list()` and any stats/`fsck` walk every object. Three changes:

1. **Pack counts are O(#packs), not O(objects).** Each `.idx` header carries
   `object_count`; total packed count = sum of `.idx` headers (read a few dozen
   bytes per pack). No id table scan needed for a *count*.
2. **Add `count()` to `StorageBackend` with a default impl.**
   ```rust
   async fn count(&self) -> Result<u64> { Ok(self.list().await?.len() as u64) }
   ```
   `MemoryBackend` keeps the trivial default (or `map.len()`); `DiskBackend` and
   `PackedDiskBackend` override it: `PackedDiskBackend::count = Σ idx.object_count
   + loose_readdir_count`. This gives stats a near-O(1) count without changing the
   Memory path. (Whether `count()` belongs on the trait vs a higher `StatStore`
   is open question 6.)
3. **`list()` stays available but cheap-by-construction.** After repack the loose
   set is small, so its `readdir` is bounded; packed ids come from a **sequential
   mmap scan of sorted id tables** — still O(total) but reading a few MB of packed
   bytes rather than `stat`-ing millions of inodes. For callers that only need a
   count (stats, progress), `count()` avoids the scan entirely.

Listing for sync (`have` sets) reuses the sorted `.idx` id tables directly — they
are already in `ObjectId` order, ideal for set-difference (§8).

---

## 8. Atomic refs — `Repository::transaction()`

Objects need no transaction (invariant 5). Refs do: advancing a head, moving a
tag, and (later) updating a policy object must commit **all-or-nothing**.

### 8.1 API surface

```rust
impl Repository {
    /// Begin a ref (and later policy) transaction.
    pub fn transaction(&self) -> RefTransaction;
}

pub struct RefTransaction { /* builder; records ops, applies nothing yet */ }

impl RefTransaction {
    // Mutations (mirror today's RefStore methods, but buffered):
    pub fn create_tag(&mut self, name: RefName, target: ObjectId, msg: Option<String>, now: u64) -> &mut Self;
    pub fn move_tag(&mut self, name: RefName, target: ObjectId) -> &mut Self;
    pub fn create_timeline(&mut self, name: RefName, head: ObjectId, policy: TimelinePolicy, now: u64, kind: String, expires_at: Option<u64>) -> &mut Self;
    pub fn advance_head(&mut self, name: RefName, new_head: ObjectId) -> &mut Self;
    pub fn delete_ref(&mut self, name: RefName) -> &mut Self;

    // Optimistic-concurrency precondition (compare-and-swap):
    pub fn expect(&mut self, name: RefName, expected: Option<Ref>) -> &mut Self;
    pub fn advance_head_if(&mut self, name: RefName, expected_old: ObjectId, new_head: ObjectId) -> &mut Self;

    /// Validate all preconditions, then commit atomically. On success every op
    /// is applied; on any failure none are.
    pub async fn commit(self) -> Result<()>;
}
```

The existing single-op `RefStore` methods remain (backward compat); each is
equivalent to a one-op transaction. WS1's policy updates plug into the same
transaction later (foundations: "ref + policy updates all-or-nothing").

### 8.2 Implementation — write-ahead journal

We keep the current per-file ref layout (`src/refs/disk.rs`: one file per ref,
tmp + rename) for backward compatibility, and add a **journal** for atomicity
*across* files. (Packed-refs single-file is the alternative — open question 7.)

`commit()`:

1. **Acquire** the repo ref lock `refs/.txn/lock` (advisory file lock;
   serialises transactions and excludes GC's planning/retire phases).
2. **Validate** every precondition (`expect` / `*_if` CAS, plus existing rules
   like "tag vs timeline kind", "ref already exists") by reading current state.
   Any failure ⇒ abort, release lock, no writes.
3. **Write the journal** `refs/.txn/<txid>.journal`: the *absolute intended final
   state* of every touched ref (name → new `Ref` value, or tombstone for delete),
   plus a record count and trailing checksum. `fsync` the journal file, then
   `fsync` the `.txn` dir. **This fsync is the commit point.**
4. **Apply** each op via the existing per-file `set`/`delete` (tmp + rename).
5. `fsync` the `refs/` dir, then delete the journal, `fsync` `.txn`.
6. Release the lock.

**Recovery on `open()`:** if any `*.journal` exists, a previous commit was
interrupted. Because the journal records *absolute final values* (not deltas),
replay is idempotent: re-apply every record (overwriting whatever partial state
exists), then delete the journal. A crash *before* step 3's fsync leaves no
durable journal → the transaction simply never happened (atomic). A crash *after*
→ replay completes it (atomic). There is no in-between visible state.

### 8.3 Durability & ordering guarantees

- **Atomicity:** all ops or none, across any number of refs. Guaranteed by the
  single journal commit point + idempotent absolute-value replay.
- **Durability:** once `commit()` returns `Ok`, the journal (and then the applied
  refs) are fsync'd; the change survives crash.
- **Isolation / ordering:** the `refs/.txn/lock` serialises committing
  transactions, so commits are linearizable; concurrent readers see either the
  pre- or post-commit state of each ref (per-file rename is atomic). CAS
  preconditions (`expect`, `advance_head_if`) reject lost updates across
  processes.
- **Objects excluded:** transactions never touch objects. A commit assumes the
  objects its refs point at were already `put` (which is durable and idempotent on
  its own). This keeps the hot object path lock-free.

---

## 9. WS5 anticipation — the pack as transfer payload

The pack format is the WS5 wire payload; this section shows it suffices.

- **have/want negotiation by `ObjectId`.** Peers exchange id sets. A receiver's
  `have` set comes straight from the sorted `.idx` id tables + loose ids (§7);
  set-difference against the sender's `want` heads is cheap because id tables are
  already sorted.
- **Sending exactly the missing objects.** The sender walks the object graph
  (§6.2 edges) from the `want` heads, pruning any subtree whose id is in the
  receiver's `have` set, collecting the missing closure. It streams a pack
  containing *only* those frames. Because each frame is independently
  decodable and self-identifying (§3.2), the sender needs no `.idx` to produce the
  stream and the receiver needs none to consume it.
- **Self-verifying receive.** The receiver validates each frame
  (`BLAKE3 == object_id`, `len == uncompressed_len`) as it arrives, lands the
  objects (as a received pack: build `.idx`, rename into `packs/`), confirms the
  trailer `pack_digest`/`end_magic`, then commits the new refs via a single
  §6 transaction. Objects-before-refs ordering plus content-addressing means a
  failed transfer leaves only collectible orphans (§4 invariant 4).
- **One format, two consumers.** The bytes the sender streams are byte-identical
  to a `.pack` on disk; a received pack can be kept as-is. No format translation
  between disk and wire.

**Thin packs / delta compression: OUT of scope for v1.** Rationale: bole stores
whole-object snapshots (no diffs), zstd already compresses each object, and
content-addressing deduplicates identical blobs/trees/subtrees across snapshots,
so the object-granularity wire set is already minimal. Binary deltas and thin
packs (frames whose base object is *not* in the pack, requiring a receive-side
"fix-up" pass) add delta-chain resolution, base lookup, and ordering complexity
for marginal gain at v1 scale. The format **reserves `record_type = 0x02`** for a
future delta record, so adding deltas later is forward-compatible without a format
break. Tracked as open question 1.

---

## 10. Backward compatibility & migration

- **Existing loose-only repos work unchanged, zero migration.** `PackedDiskBackend`
  reads loose-first; a repo with an empty/absent `packs/` dir behaves exactly like
  today's `DiskBackend`. Packs are purely additive.
- **`DiskBackend` is retained** and still usable directly; `Repository::disk()`
  swaps in `PackedDiskBackend`, which can delegate the loose half to `DiskBackend`.
- **`repack`/`gc` are opt-in.** Nothing breaks if they are never run; they only
  optimise.
- **Refs:** the per-file layout is unchanged; the `.txn/` dir is new and ignored
  by old read paths. Existing single-op `RefStore` methods keep working; the
  transaction API is additive.
- **`MemoryBackend` and the 247 existing tests are untouched** — the
  `StorageBackend` trait change is a `count()` method with a default impl.
- **Versioned formats:** `.pack` and `.idx` carry `version` fields; future
  changes bump the version and old readers fail loudly rather than misparse.

---

## 11. Testing strategy

- **Pack round-trip:** build a pack from N objects; read each back via `(offset,
  len)`; assert canonical bytes and recomputed `BLAKE3` match; assert
  `object_count` and trailer digest.
- **Streaming decode:** feed the frame stream incrementally (including
  byte-at-a-time and split-mid-frame) into the receiver decoder; assert each
  object verifies; assert a truncated stream / wrong `end_magic` / bad
  `pack_digest` is rejected.
- **Index:** random hits/misses; fan-out boundary ids (first/last in a bucket,
  `id[0] == 0` and `0xff`); detect index/pack digest mismatch → rebuild.
- **repack:** loose → pack; loose deleted only after pack durable; reads resolve
  post-repack; simulated crash points (tmp pack present, no idx → invisible; pack
  present + loose still present → dedup, both readable).
- **GC:** construct a snapshot DAG with shared subtrees, secrets via overlay, and
  multiple timelines/tags; drop a branch ref; assert reachable kept and
  unreachable gone; assert `EnvOverlay → Secret` edge keeps live secrets; assert
  grace window protects just-written objects; assert ephemeral heads are roots.
- **Transaction:** multi-ref commit is all-or-nothing; inject a crash after the
  journal fsync and assert recovery replays to a complete commit; inject before
  and assert no change; CAS conflict rejected; concurrent transactions serialise.
- **Cross-impl conformance / differential:** a `StorageBackend` conformance suite
  run against `MemoryBackend`, `DiskBackend`, and `PackedDiskBackend`; a
  differential test asserting a packed repo and an all-loose repo with the same
  objects answer `get`/`exists`/`list`/`count` identically.
- **Property tests:** random object graphs; invariant `reachable(before) ==
  reachable(after)` across any sequence of `repack`/`gc`; `put` idempotence.
- **Regression:** all 247 existing tests stay green.

---

## 12. Open questions (maintainer's call)

1. **Delta compression / thin packs in v1?** Spec recommends *no* (format reserves
   `record_type = 0x02`). Revisit if profiling shows the whole-object wire set is
   too large.
2. **Per-pack zstd dictionary** (trained over the pack's objects, stored in the
   header) to recover cross-object compression lost by per-frame framing — worth
   the complexity for v1?
3. **Pack/repack thresholds:** loose-count vs total-bytes vs age trigger for
   auto-repack; max pack size before splitting; repack-all cadence.
4. **GC concurrency model:** grace-window-only vs full repo lock vs CAS-guarded;
   exact grace duration; multi-process safety guarantees.
5. **Multi-pack-index (`.midx`) in v1 or phase 2?** And the `#packs` threshold
   that triggers building it.
6. **Does `count()` (and `list()`) belong on `StorageBackend`,** or on a separate
   higher-level `StatStore` trait to keep `MemoryBackend` minimal?
7. **Ref store: journal over per-file refs (this spec) vs a packed-refs
   single-file** as the canonical store. Packed-refs gives O(1) atomic commit by
   one rename and O(1) listing, but breaks the per-file layout external tooling
   may rely on.
8. **Secret reachability:** confirm secrets are reachable *only* via
   `EnvOverlay`; if any future object (or a "loose secret" workflow) can root a
   secret independently, GC's edge set must be extended.
9. **`delete` on a pack-resident object:** no-op (current proposal), hard error,
   or schedule a targeted repack? Affects callers expecting `delete` to be
   immediately observable.
10. **Verify-on-read:** always recompute `BLAKE3` on `get` (safe, costs a hash per
    read) vs trust-on-disk and verify only during `fsck`/receive?
11. **fsync granularity / durability level** for object writes (per-object vs
    batched) and its throughput cost during bulk import and pack receive.
