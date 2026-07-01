# WS6 — Locked Implementation Decisions (bole-mtq)

Resolves the open questions in
[specs/2026-06-29-ws6-git-import.md](../specs/2026-06-29-ws6-git-import.md).
Maintainer-approved 2026-07-01.

| OQ | Decision |
|----|----------|
| **Scope** | Full **library** round-trip: `git_import` (all passes), bidirectional `IdentityMap` sidecar, incremental re-import, `--label-ruleset`, persist the export map, and fix annotated-tag export in `project_to_git`. The `bole git import` CLI verb → **bole-58u** (WS7). |
| **OQ-1/2 — committer/tagger** | Message trailer: append `Committer: name <email> ts tz` to the Snapshot message (and `Tagger: …` to annotated Tag messages) when committer≠author. Author → `Snapshot.author`/`created_at`. Non-breaking. |
| **OQ-3 — annotated tag export** | Fix in WS6: `project_to_git` writes git annotated tag objects for `Tag { message: Some(_) }`; the identity map tracks tag OIDs. |
| **OQ-4 — re-import non-FF** | Respect the timeline's policy; a non-fast-forward advance fails unless `opts.force` (privileged override). |
| **OQ-5 — map format** | Postcard sidecar. |
| **OQ-6 — fingerprint** | SHA-256 of the canonical absolute source path. |
| **OQ-7 — ref collision** | Abort with a descriptive error (no silent renaming). |
| **OQ-8 — empty commits** | Distinct Snapshots by content hash; no special handling. |

## Object mapping (git → bole)

blob→Blob; tree entry 100644/100755→Blob entry (exec bit lost); 40000→Tree
(recurse); 120000 symlink→Blob of target bytes; 160000 submodule→skipped.
commit→Snapshot{root, parents, author="Name <email>", created_at=author_time,
message(+Committer trailer)}. branch→Timeline (Unrestricted default / `--timeline-policy`,
kind="persistent"). lightweight tag→Tag{message:None}; annotated tag→Tag{message:Some}.
Ref names sanitized (`.`-prefixed segment → `_`-prefixed; strip NUL; collapse `//`;
abort on collision).

## Identity map

`<map_dir>/git-map/<sha256(canonical_path)>.postcard` = `IdentityMap { git_to_bole:
HashMap<Vec<u8>,[u8;32]>, bole_to_git: HashMap<[u8;32],Vec<u8>> }`. Loaded to seed both
import (skip already-translated git OIDs) and export (reuse git OIDs). Written after all
objects+refs commit; crash before write → idempotent re-translation next run.

## Build order (TDD)
1. `IdentityMap` struct + postcard load/save + fingerprint (pure).
2. Ref-name sanitizer (pure).
3. `project_to_git`: `identity_map_dir: Option<&Path>` param (persist/seed map) + annotated-tag export.
4. `git_import` passes via gix (open, refs, topo, blobs→trees→commits→timelines→tags, label rules, incremental).
5. Round-trip + incremental + label integration tests.
6. Full `cargo test --workspace` green (exit code, not grep).
