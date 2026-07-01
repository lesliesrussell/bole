# WS4 — Locked Implementation Decisions (bole-81z)

Resolves the open questions in
[specs/2026-06-29-ws4-storage-packs-gc.md](../specs/2026-06-29-ws4-storage-packs-gc.md).
Maintainer-approved 2026-07-01.

| OQ | Decision |
|----|----------|
| **Scope** | Build **pack format + PackedDiskBackend + repack + mark-sweep GC + count()** now. Atomic ref transactions (§8) → follow-up **bole-sk6**. |
| **O1 — deltas/thin packs** | No for v1. Reserve `record_type = 0x02` in the frame for a future delta record. |
| **O2 — per-pack zstd dictionary** | No for v1 (per-frame independent zstd). `flags.has_dictionary` reserved = 0. |
| **O3 — repack thresholds** | Manual `repack`; no auto-trigger wired in v1 (advisory). Threshold constant documented (5000 loose) for a later auto-hook. |
| **O4 — GC concurrency/grace** | Grace window (default 2h) protects recently-written objects; GC takes the repo lock for planning + retire. v1 ships the grace-set + reachable-closure logic; multi-process lock hardening lands with bole-sk6's `refs/.txn/lock`. |
| **O5 — multi-pack-index** | Phase 2. v1 uses per-pack `.idx`, probed newest-first. |
| **O6 — count()/list()** | `count()` on `StorageBackend` with a default (`list().len()`); `PackedDiskBackend` overrides (Σ idx.object_count + loose readdir). |
| **O7 — ref atomicity** | Journal over per-file refs (not packed-refs). Built in bole-sk6. |
| **O8 — secret reachability** | Secrets are reachable **only** via `EnvOverlay(EnvValue::Secret)`. GC edge set: Snapshot→{root,parents}, Tree→entries, EnvOverlay→secret refs; Blob/Secret/SecretV2 are leaves. |
| **O9 — delete on packed object** | No-op (packs are immutable; packed objects leave via repack/GC). |
| **O10 — verify-on-read** | Trust-on-disk for `get`; verify BLAKE3 on pack **receive** and in `fsck`/GC copy-forward. |
| **O11 — fsync granularity** | Per-object write (as today); pack + idx fsync before rename. |

## Pack format (v1)

`.pack` = 32-byte header (`BOLEPACK`, version=1, flags, object_count, reserved) +
per-object frames (`record_type` `0x01`, `object_id[32]`, `uncompressed_len`,
`stored_len`, independent zstd frame) + 40-byte trailer (`BOLEPKND`,
`pack_digest = BLAKE3(header..last frame)`). `.idx` = `BOLEIDX\0`, version=1,
count, 256-entry fanout, sorted ids, offsets, lens, pack_digest, idx_digest.
Little-endian; varint = LEB128. Frames are self-verifying (id = BLAKE3 of
canonical postcard bytes) so the same bytes serve disk and the WS5 wire.

## Directory layout

`<repo>/objects/` (loose, unchanged) · `<repo>/packs/pack-<digest>.{pack,idx}`
(new). `PackedDiskBackend` = loose `DiskBackend` + `PackSet` (scan `packs/*.idx`
at open). get: loose-first then packs newest-first. put: always loose. delete:
loose only (no-op on packed). repack: loose→pack, delete loose only after pack
durable. GC: reachable closure from ref roots → rewrite packs keeping reachable →
unlink old packs → unlink unreachable loose older than grace.

## Build order (TDD)

1. `store::pack` — frame/pack/index encode+decode+verify (pure bytes).
2. `store::packed::PackedDiskBackend` — loose + PackSet; get/exists/put/delete/list/count.
3. `repack` — loose→pack crash-safe sequence.
4. GC — mark (object-graph closure from refs) + sweep (repack-and-drop + loose unlink w/ grace).
5. Wire `Repository::disk()` onto PackedDiskBackend; `count()` on the trait.
6. Full `cargo test --workspace` green (exit code, not grep).
