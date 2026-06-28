## Context

We want a source/version system designed for:

- Fine‑grained visibility: private files, private branches, delayed/partial merges as a core concept, not a policy wrapper on top of a global DAG. [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)
- Non‑file backing stores: in‑memory repos, object stores, databases, “virtual” trees; no required 1:1 mapping to an OS directory. [tigrisdata](https://www.tigrisdata.com/blog/snapshots/)
- Secret/material separation: ENV and secret management as a natural property of the model instead of a brittle convention layered over Git. [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)
- Agentic workflows: multiple automated actors operating concurrently, with ephemeral and persistent timelines, snapshots and tags as cheap primitives. [cloudbees](https://www.cloudbees.com/blog/git-tag-guide-to-managing-snapshots)

JJ/Jujutsu, Pijul, etc. explore better history semantics and UIs, but they still live near “files in a tree + DAG of commits” as the core abstraction. The idea here is to rebase the model itself. [soundbarrier](https://www.soundbarrier.io/posts/git_alternatives/)

***

## Goals

G1. Snapshots as the only durable state primitive  
Every durable project state is a **snapshot**: a typed mapping from logical paths to content blobs (files, secrets, generated artifacts, structured objects). [missing-semester-pt.github](https://missing-semester-pt.github.io/2020/version-control/)

G2. Tags and timelines instead of branches  
Human‑meaningful labels (**tags**) and ordered views (**timelines**) are first‑class, decoupled from storage; they are cheap movable pointers over a pool of snapshots. [blog.ninapanickssery](https://blog.ninapanickssery.com/p/git)

G3. Granular, structural permissions  
Every path and tag participates in an ACL/visibility lattice, so you can have private files, private timelines, and delayed reconciliation into shared state. [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)

G4. Secrets and envs as typed resources  
Secrets, environment overlays, and configuration bundles are typed graph nodes with their own visibility and lifecycle, not just files in the tree. [tigrisdata](https://www.tigrisdata.com/blog/snapshots/)

G5. In‑memory and virtual repos  
Repositories can be backed by memory, embedded KV stores, blob stores, or remote APIs; the API never assumes “local POSIX directory.” [learn.microsoft](https://learn.microsoft.com/en-us/devops/develop/git/centralized-to-git)

G6. Multi‑actor, multi‑timeline workflows  
Multiple humans and agents can maintain ephemeral or long‑lived timelines, forked from or merging into shared baselines, with programmable policies. [news.ycombinator](https://news.ycombinator.com/item?id=45362755)

G7. Backward‑compatible export  
It must be possible to project a view into a normal Git repo for interoperability and migration. [learn.microsoft](https://learn.microsoft.com/en-us/devops/develop/git/centralized-to-git)

***

## Gates

Gate 1 (Snapshots core):  
- There is exactly one durable primitive representing project state, `Snapshot`, with:  
  - A content map `path → content_id` plus metadata (author, created_at, parents, type).  
  - Content‑addressed storage for blobs/objects, independent of host filesystem. [missing-semester-pt.github](https://missing-semester-pt.github.io/2020/version-control/)
- Any operation that changes visible state (commit, merge, rebase, apply patch, agent action) creates a new `Snapshot`. No mutable shared workspace state.

Gate 2 (Tags & timelines):  
- `Tag` is a named pointer to a `Snapshot` (or a `Timeline` head), with metadata; tags are mutable, snapshots are immutable. [cloudbees](https://www.cloudbees.com/blog/git-tag-guide-to-managing-snapshots)
- `Timeline` is an ordered sequence of snapshots (a view of the DAG) plus policy: “how do new snapshots get added?”  
- Operations exist to create, move, and delete tags and timelines without copying data (pure reference moves).

Gate 3 (Granular visibility):  
- Every `path` and `Tag`/`Timeline` is associated with an ACL or policy object (roles, groups, capabilities). [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)
- The system supports at least:  
  - Private paths within a repo (hidden from users lacking access even when they fetch a snapshot).  
  - Private timelines/tags (e.g., “leslie/private/exp‑foo”).  
  - Policy‑driven merge: a merge attempt that would expose restricted paths to an unauthorized timeline fails or requires an explicit approval object.

Gate 4 (Secrets and env graph):  
- `Secret` and `EnvOverlay` are first‑class node types in the object store, not regular blobs.  
- They have separate encryption, audit, and visibility settings from plain files. [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)
- A snapshot’s “workspace view” is computed as: base files + env overlays + secret bindings, so you can mount different envs (dev, prod) on the same code snapshot without committing `.env`‑like files. [tigrisdata](https://www.tigrisdata.com/blog/snapshots/)

Gate 5 (In‑memory / virtual repos):  
- Core APIs (`open_repo`, `create_snapshot`, `materialize_view`) accept a storage backend abstraction that can be: memory, local disk, remote object store, or custom.  
- You can create a repo that never touches disk (e.g., for agents), yet still use full snapshot/tag semantics. [tigrisdata](https://www.tigrisdata.com/blog/snapshots/)
- A “materialize” operation lets you project a snapshot or timeline into a directory or container filesystem, but this is optional and reversible.

Gate 6 (Multi‑actor / agents):  
- The model supports multiple concurrent writers using isolated timelines, with deterministic merge semantics (e.g., CRDT‑ish or explicitly policy‑driven). [soundbarrier](https://www.soundbarrier.io/posts/git_alternatives/)
- Timelines have labels like `ephemeral`, `review`, `release`, with configurable retention and merge rules.  
- Agents can be assigned capabilities to specific paths or timelines (e.g., “AI‑formatter can touch *.zig under src/, but not secrets/”). [rewind](https://rewind.com/blog/git-security-issues-watch-out-for/)

Gate 7 (Git projection):  
- There is a defined mapping from a `Timeline` + visibility filter to a Git repository: snapshots → commits, tags → Git tags/branches. [blog.ninapanickssery](https://blog.ninapanickssery.com/p/git)
- A CLI command (e.g., `vcs export git`) can emit a Git repo and keep it updated incrementally.

Gate 8 (Performance and scale):  
- Content storage deduplicates blobs across snapshots; structural sharing in trees is required. [missing-semester-pt.github](https://missing-semester-pt.github.io/2020/version-control/)
- It supports large monorepos and many small repos with reasonable latency for local and remote operations, on par with or better than Git’s common workflows. [blog.ninapanickssery](https://blog.ninapanickssery.com/p/git)

***

## Test Plan

This is written TDD‑style: tests are the primary embodiment of the gates.

T1 → Gate 1 (Snapshots core)  
- Create a repo, add files `a`, `b`, modify `a` twice, and verify that each state is only reachable via an explicit `Snapshot`.  
- Assert snapshots are immutable (any “edit” produces a new snapshot id) and that removing materialized files does not affect the stored snapshot history.

T2 → Gate 2 (Tags & timelines)  
- Create snapshots S1–S3 and attach tags `v1`, `experiment/foo` to different snapshots. Move `experiment/foo` and assert no data is copied.  
- Create a `Timeline` `main`, append snapshots via merges, and validate that moving the `main` head updates only references, not content.

T3 → Gate 3 (Granular visibility)  
- Create paths `src/app.zep`, `secrets/prod.key`, `notes/private.md` with different ACLs.  
- As user A (no secret access), list files and fetch snapshots; `secrets/prod.key` is not present. As user B, it is.  
- Attempt to merge a timeline with `secrets/*` into a public timeline; assert the merge is rejected or produces a policy‑required status.

T4 → Gate 4 (Secrets and env graph)  
- Define `EnvOverlay dev` with `DB_URL=sqlite://dev.db` and `EnvOverlay prod` with `DB_URL=postgres://prod`.  
- Attach the same code snapshot to both envs and materialize views; verify that `.env` artifacts never appear in snapshots, but the running environment differs.  
- Rotate a `Secret` and ensure snapshots referencing it see the new value only where permitted (e.g., at runtime, not in history diffs).

T5 → Gate 5 (In‑memory / virtual repos)  
- Create an in‑memory repo, perform 1000 snapshot operations, then serialize it to disk and reload; hashes and tags must be identical.  
- Create a virtual repo backed by a mock KV store; materialize a snapshot into a temp directory and verify file contents match, then delete the directory and still re‑materialize later.

T6 → Gate 6 (Multi‑actor / agents)  
- Spawn two clients/agents editing overlapping paths on independent timelines, then merge according to a policy (e.g., last‑writer‑wins or three‑way merge).  
- Ensure an agent restricted to `src/**` cannot modify `secrets/**` even via bulk or ref‑level operations.  
- Create ephemeral timelines per agent session; assert that after session end and TTL expiry, only snapshots tagged as “promoted” remain.

T7 → Gate 7 (Git projection)  
- From a test repo with multiple timelines and private paths, export a Git view and confirm:  
  - Only permitted paths appear.  
  - Snapshot ordering and merge structure is preserved enough that `git log` matches the projected history.  
  - Tags are visible as Git tags/branches with expected commit hashes. [cloudbees](https://www.cloudbees.com/blog/git-tag-guide-to-managing-snapshots)

T8 → Gate 8 (Performance and scale)  
- Create 100k snapshots with small changes and ensure the total storage footprint is within a small multiple of the raw data size (dedup effective). [missing-semester-pt.github](https://missing-semester-pt.github.io/2020/version-control/)
- Measure typical operations (snapshot creation, tag move, diff, export to Git) and set budget thresholds, e.g., P95 latency < X ms on a defined hardware baseline.

***

## Non‑functional constraints and guardrails

- Performance:  
  - Local operations should be comparable to Git for typical workflows: commit‑equivalent, log/history queries, diffs. [blog.ninapanickssery](https://blog.ninapanickssery.com/p/git)
  - Remote ops must degrade gracefully with latency and support partial fetch of timelines.

- Security:  
  - Secrets are encrypted at rest and in transit and never appear in plaintext diffs.  
  - ACL evaluation is centralized and auditable; “leakage by projection” (e.g., via Git export) must be covered in tests.

- Storage:  
  - Content‑addressed blobs with compression and structural sharing to keep storage low even for snapshot‑heavy workloads. [missing-semester-pt.github](https://missing-semester-pt.github.io/2020/version-control/)

- Interop:  
  - Must support round‑trip basic Git scenarios well enough that you can adopt it incrementally in a Git shop.  
  - CLI and API ergonomics allow plugging into existing CI systems with minimal glue.

- Agent safety:  
  - Capability‑based access tokens for agents, scoping them to timelines + path patterns.  
  - Logging of all agent‑initiated state transitions with snapshot ids and metadata for forensics.

***

## Implementation notes

- Core data model likely wants a content‑addressed store with typed objects: `Blob`, `Tree`, `Snapshot`, `Tag`, `Timeline`, `Secret`, `EnvOverlay`, `Policy`, akin to Git’s objects but with richer typing and separate secret/overlay graphs. [blog.ninapanickssery](https://blog.ninapanickssery.com/p/git)
- A Zepoid/logic‑style policy layer could define merge/visibility rules as constraints (e.g., “no object tagged `secret` may appear on timeline `public/*`”).  
- File‑system integration happens via projection drivers (POSIX, in‑container, editor virtual FS) rather than being the ground truth.  
- For usability, the initial UX should feel familiar to Git users: `snap`, `tag`, `timeline`, `merge`, `export git`, then gradually expose the richer permission/env model.

***

## Out‑of‑scope

- Detailed CRDT vs DAG vs patch‑theoretic conflict semantics (could be pluggable and chosen later). [soundbarrier](https://www.soundbarrier.io/posts/git_alternatives/)
- Full multi‑tenant hosting architecture (authn/authz backend, billing, org management).  
- UI/UX tooling like GUIs, IDE plugins, or visual history browsers.  
- Binary artifact management, build caching, and CI orchestration—assumed to integrate, not be reimplemented here.

