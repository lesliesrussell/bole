# Gate 7: Git Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `project_to_git(repo, target_path, accessor)` that writes a complete bole repository history into a standard bare git repo so `git log`, `git diff`, and `git blame` work against it.

**Architecture:** Five sequential passes — init bare repo, topo-sort all reachable snapshots, write git blobs/trees/commits, write branch refs, write tag refs. One-shot (no persistent mapping table); an in-memory `HashMap` deduplicates shared ancestor snapshots within a single run. ACL filtering reuses the existing `walk_tree_filtered` helper.

**Tech Stack:** Rust (async, tokio), `gix` crate (pure Rust git), `thiserror` (existing), `tempfile` (existing dev-dep).

## Global Constraints

- **No `anyhow`** — `thiserror` only in library code; all new error variants use `#[error("...")] VariantName(...)` format.
- **No feature flags** — all code always compiled; no `#[cfg(feature = ...)]`.
- **Bead comment on every contiguous block of new code** — `// <bead-id>`, one comment per block, not per line.
- **Branch name = bead ID exactly** — `git checkout -b <bead-id>` before any file edits.
- **Tests must pass before merge** — `cargo test` clean, `cargo clippy -- -D warnings` clean.
- **Delete branch after merge** — `git branch -d <bead-id>` after `git merge <bead-id>`.
- **gix version** — `gix = { version = "0.70", default-features = false, features = ["max-performance-safe"] }` in `[dependencies]`.
- **`gix::ObjectId` as the type** for git object IDs throughout `git_projection.rs`.
- **Loose ref files** — write `refs/heads/{name}` and `refs/tags/{name}` as plain files with `"{sha}\n"` content (no gix ref transaction API needed for a fresh bare repo).
- **Secret / EnvOverlay entries silently skipped** — `Object::Blob` only goes to git; any other variant is dropped without error.
- **Synthetic git identity** — `"{author} <bole@local> {created_at} +0000"` for both author and committer; if `author` is empty, use `"bole"`.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Add `gix` to `[dependencies]` |
| `src/error.rs` | Modify | Add `GitProjection(String)` variant |
| `src/repo/mod.rs` | Modify | Add `pub mod git_projection` |
| `src/repo/git_projection.rs` | Create | All projection logic |
| `src/lib.rs` | Modify | Re-export `project_to_git` |
| `tests/git_projection.rs` | Create | T7 integration tests |

---

## Task 1: Scaffolding

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/error.rs`
- Modify: `src/repo/mod.rs`
- Create: `src/repo/git_projection.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `pub async fn project_to_git(repo: &Repository, target_path: &Path, accessor: &Accessor) -> Result<()>` (stub, returns `Ok(())`)
- Produces: `Error::GitProjection(String)` variant

**Bead workflow:**
```bash
bd create --title="G7-T1: git projection scaffolding" --description="Add gix dep, GitProjection error variant, stub project_to_git, wire module" --type=task --priority=2
# note the bead ID printed (e.g. bole-abc)
bd update bole-abc --claim
git checkout -b bole-abc
```

- [ ] **Step 1: Write the scaffolding test**

In `src/repo/git_projection.rs` (create the file):

```rust
// bole-abc
use std::path::Path;
use crate::acl::Accessor;
use crate::error::Result;
use crate::repo::Repository;

pub async fn project_to_git(
    _repo: &Repository,
    _target_path: &Path,
    _accessor: &Accessor,
) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn stub_returns_ok() {
        let dir = tempdir().unwrap();
        let repo = Repository::memory();
        let accessor = Accessor::privileged();
        let result = project_to_git(&repo, dir.path(), &accessor).await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Add gix to Cargo.toml**

```toml
gix = { version = "0.70", default-features = false, features = ["max-performance-safe"] }
```

Add that line to `[dependencies]` in `Cargo.toml`.

- [ ] **Step 3: Add GitProjection error variant**

In `src/error.rs`, add the new variant to the `Error` enum:

```rust
    #[error("git projection failed: {0}")] GitProjection(String),
```

Place it after the existing variants. Do not remove any existing variant.

- [ ] **Step 4: Wire the module**

In `src/repo/mod.rs`, add after the existing `pub mod merge;` line:

```rust
// bole-abc
pub mod git_projection;
```

In `src/lib.rs`, add after the existing `pub use repo::{copy_objects, materialize::materialize, Repository};` line:

```rust
// bole-abc
pub use repo::git_projection::project_to_git;
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass, including the new `stub_returns_ok`. The test count should increase by 1.

- [ ] **Step 6: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | tail -10
```

Expected: no warnings, exit 0.

- [ ] **Step 7: Commit and merge**

```bash
git add Cargo.toml Cargo.lock src/error.rs src/repo/mod.rs src/repo/git_projection.rs src/lib.rs
git commit -m "bole-abc: G7-T1 git projection scaffold"
git checkout master
git merge bole-abc
git branch -d bole-abc
bd close bole-abc
```

---

## Task 2: project\_to\_git Implementation

**Files:**
- Modify: `src/repo/git_projection.rs`

**Interfaces:**
- Consumes: `super::walk_tree_filtered(&repo.objects, &repo.acls, snap.root, "", accessor, &mut flat)` — private fn in `src/repo/mod.rs`, accessible from a child module via `super::`.
- Consumes: `repo.refs.list("")?` — `RefStore::list(prefix) -> Result<Vec<RefName>>`
- Consumes: `repo.refs.get(&name)?` — `RefStore::get(name: &RefName) -> Result<Option<Ref>>`
- Consumes: `Accessor::privileged()` — from Gate 6 (bole-qv5)
- Consumes: `repo.objects.get(&id).await?` — `ObjectStore::get(id) -> Result<Option<Object>>`
- Produces: `project_to_git` fully implemented (all 5 passes)

**Bead workflow:**
```bash
bd create --title="G7-T2: implement project_to_git" --description="Five-pass git projection: init, topo-sort, write objects, write branch refs, write tag refs" --type=feature --priority=2
# e.g. bole-def
bd update bole-def --claim
git checkout -b bole-def
```

- [ ] **Step 1: Write the driving integration test**

Add a test at the bottom of `src/repo/git_projection.rs` (inside the existing `#[cfg(test)]` block, replacing the stub test):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bole_test_helpers::*;
    use tempfile::tempdir;

    // helpers live in this module
    fn privileged_read_accessor() -> Accessor {
        use crate::acl::{PathRole, Permission, TimelineRole};
        Accessor::privileged()
    }

    async fn linear_repo() -> (crate::repo::Repository, crate::object::ObjectId, crate::object::ObjectId, crate::object::ObjectId) {
        use crate::object::{Blob, EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use bytes::Bytes;
        use std::collections::BTreeMap;

        let repo = crate::repo::Repository::memory();

        let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
        let mut e1 = BTreeMap::new();
        e1.insert("app.rs".into(), TreeEntry { id: b1, kind: EntryKind::Blob });
        let t1 = repo.objects.put_tree(e1).await.unwrap();
        let s1 = repo.objects.put_snapshot(Snapshot {
            root: t1, parents: vec![],
            author: "alice".into(), created_at: 1000, message: "first".into(),
        }).await.unwrap();

        let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
        let mut e2 = BTreeMap::new();
        e2.insert("app.rs".into(), TreeEntry { id: b2, kind: EntryKind::Blob });
        let t2 = repo.objects.put_tree(e2).await.unwrap();
        let s2 = repo.objects.put_snapshot(Snapshot {
            root: t2, parents: vec![s1],
            author: "alice".into(), created_at: 2000, message: "second".into(),
        }).await.unwrap();

        let b3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
        let mut e3 = BTreeMap::new();
        e3.insert("app.rs".into(), TreeEntry { id: b3, kind: EntryKind::Blob });
        let t3 = repo.objects.put_tree(e3).await.unwrap();
        let s3 = repo.objects.put_snapshot(Snapshot {
            root: t3, parents: vec![s2],
            author: "alice".into(), created_at: 3000, message: "third".into(),
        }).await.unwrap();

        let name = crate::refs::RefName::new("main").unwrap();
        repo.refs.create_timeline(name, s3, TimelinePolicy::Unrestricted, 3000, "persistent".into(), None).unwrap();
        (repo, s1, s2, s3)
    }

    #[tokio::test]
    async fn linear_timeline_exports_branch_ref() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        assert!(target.join("refs").join("heads").join("main").exists());
    }

    #[tokio::test]
    async fn exported_commit_chain_has_three_commits() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();

        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let sha_hex = sha_hex.trim();
        let count = count_commit_chain(&target, sha_hex);
        assert_eq!(count, 3, "expected 3 commits in chain, got {count}");
    }

    /// Walk the parent chain in the exported git repo, counting commits.
    /// Parses commit text directly (no gix reading needed).
    fn count_commit_chain(repo_path: &std::path::Path, start_sha: &str) -> usize {
        let mut count = 0usize;
        let mut current = start_sha.to_owned();
        loop {
            let text = read_git_object_text(repo_path, &current);
            count += 1;
            if let Some(parent_line) = text.lines().find(|l| l.starts_with("parent ")) {
                current = parent_line.strip_prefix("parent ").unwrap().to_owned();
            } else {
                break;
            }
        }
        count
    }

    /// Read and zlib-decompress a loose git object, returning its text payload (header stripped).
    fn read_git_object_text(repo_path: &std::path::Path, sha: &str) -> String {
        use std::io::Read;
        let obj_path = repo_path.join("objects").join(&sha[..2]).join(&sha[2..]);
        let file = std::fs::File::open(&obj_path).expect("object file missing");
        let mut decoder = flate2::read::ZlibDecoder::new(file);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        // strip "commit {size}\0" header
        let nul = raw.iter().position(|&b| b == 0).expect("no nul in object");
        String::from_utf8(raw[nul + 1..].to_vec()).unwrap()
    }

    /// Return top-level entry names from a raw git tree binary.
    fn tree_top_entry_names(tree_data: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut i = 0;
        while i < tree_data.len() {
            // mode ends at space
            let sp = match tree_data[i..].iter().position(|&b| b == b' ') {
                Some(p) => p,
                None => break,
            };
            let after_mode = i + sp + 1;
            // name ends at nul
            let nl = match tree_data[after_mode..].iter().position(|&b| b == 0) {
                Some(p) => p,
                None => break,
            };
            let name = std::str::from_utf8(&tree_data[after_mode..after_mode + nl]).unwrap();
            names.push(name.to_owned());
            i = after_mode + nl + 1 + 20; // nul + 20-byte sha
        }
        names
    }
}
```

Add `flate2 = "1"` to `[dev-dependencies]` in `Cargo.toml` (needed to decompress git objects in tests).

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test git_projection 2>&1 | tail -20
```

Expected: `linear_timeline_exports_branch_ref` and `exported_commit_chain_has_three_commits` fail because `project_to_git` is still a stub.

- [ ] **Step 3: Implement the full module**

Replace the entire content of `src/repo/git_projection.rs` with:

```rust
// bole-def
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use gix::prelude::Write as _;

use crate::acl::Accessor;
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::refs::Ref;
use crate::repo::Repository;

pub async fn project_to_git(
    repo: &Repository,
    target_path: &Path,
    accessor: &Accessor,
) -> Result<()> {
    // Pass 1: init bare git repo
    let git_repo = gix::init::bare(target_path)
        .map_err(|e| Error::GitProjection(e.to_string()))?
        .to_thread_local();

    // Pass 2: collect timeline heads, then topo-sort reachable snapshots
    let all_refs = repo.refs.list("")?;
    let mut timeline_heads: Vec<(crate::refs::RefName, ObjectId)> = Vec::new();
    for name in &all_refs {
        if let Some(Ref::Timeline(tl)) = repo.refs.get(name)? {
            if accessor.can_read_timeline(name.as_str()) {
                timeline_heads.push((name.clone(), tl.head));
            }
        }
    }
    let starts: Vec<ObjectId> = timeline_heads.iter().map(|(_, h)| *h).collect();
    let ordered = collect_topo(&repo.objects, starts).await?;

    // Pass 3: write blobs, trees, commits; build bole → git id map
    let mut id_map: HashMap<ObjectId, gix::ObjectId> = HashMap::new();
    for snap_id in &ordered {
        let snap = match repo.objects.get(snap_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => continue,
        };
        let mut flat = BTreeMap::new();
        super::walk_tree_filtered(
            &repo.objects, &repo.acls, snap.root, "", accessor, &mut flat,
        ).await?;
        let git_tree = write_git_tree_level(&flat, &repo.objects, &git_repo).await?;

        let parents: Vec<gix::ObjectId> =
            snap.parents.iter().filter_map(|p| id_map.get(p)).cloned().collect();
        let author_name = if snap.author.is_empty() { "bole" } else { snap.author.as_str() };
        let identity = format!("{} <bole@local> {} +0000", author_name, snap.created_at);
        let commit_bytes = encode_commit(git_tree, &parents, &identity, &snap.message);
        let git_commit = git_repo
            .objects
            .write_buf(gix::object::Kind::Commit, &commit_bytes)
            .map_err(|e| Error::GitProjection(e.to_string()))?;
        id_map.insert(*snap_id, git_commit);
    }

    // Pass 4: write branch refs
    for (name, head) in &timeline_heads {
        if let Some(git_id) = id_map.get(head) {
            write_loose_ref(target_path, &format!("refs/heads/{}", name.as_str()), git_id)?;
        }
    }

    // Pass 5: write tag refs (lightweight; target must be in id_map)
    for name in &all_refs {
        if let Some(Ref::Tag(tag)) = repo.refs.get(name)? {
            if let Some(git_id) = id_map.get(&tag.target) {
                write_loose_ref(target_path, &format!("refs/tags/{}", name.as_str()), git_id)?;
            }
        }
    }

    Ok(())
}

/// DFS post-order topological sort of all snapshots reachable from `starts`.
/// Parents always precede their children in the result.
async fn collect_topo(
    objects: &crate::store::ObjectStore,
    starts: Vec<ObjectId>,
) -> Result<Vec<ObjectId>> {
    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut result: Vec<ObjectId> = Vec::new();
    // stack entries: (id, finishing)
    // When finishing=true, append to result. When false, push with finishing=true
    // then push unvisited parents (so parents are processed before the node).
    let mut stack: Vec<(ObjectId, bool)> =
        starts.iter().map(|id| (*id, false)).collect();
    while let Some((id, finishing)) = stack.pop() {
        if finishing {
            result.push(id);
            continue;
        }
        if !visited.insert(id) {
            continue; // already processed via another path
        }
        stack.push((id, true));
        if let Some(Object::Snapshot(snap)) = objects.get(&id).await? {
            for parent in snap.parents.iter().rev() {
                stack.push((*parent, false));
            }
        }
    }
    Ok(result)
}

/// Convert a flat ACL-filtered path map (from walk_tree_filtered) to nested
/// git tree objects. Returns the root git tree's ObjectId.
///
/// `flat` contains paths relative to the current level (e.g. "src/app.rs" at
/// the root level, or "app.rs" when recursing into the "src" subtree).
async fn write_git_tree_level(
    flat: &BTreeMap<String, ObjectId>,
    objects: &crate::store::ObjectStore,
    git_repo: &gix::Repository,
) -> Result<gix::ObjectId> {
    // (name, is_dir, git_sha)
    let mut entries: Vec<(String, bool, gix::ObjectId)> = Vec::new();
    let mut dirs_seen: HashSet<String> = HashSet::new();

    for (path, blob_id) in flat {
        if let Some(slash) = path.find('/') {
            let dir = &path[..slash];
            if dirs_seen.insert(dir.to_owned()) {
                let prefix = format!("{}/", dir);
                let sub_flat: BTreeMap<String, ObjectId> = flat
                    .iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .map(|(k, v)| (k[prefix.len()..].to_owned(), *v))
                    .collect();
                let sub_id = Box::pin(write_git_tree_level(&sub_flat, objects, git_repo)).await?;
                entries.push((dir.to_owned(), true, sub_id));
            }
        } else {
            match objects.get(blob_id).await? {
                Some(Object::Blob(b)) => {
                    let git_blob = git_repo
                        .objects
                        .write_buf(gix::object::Kind::Blob, b.data.as_ref())
                        .map_err(|e| Error::GitProjection(e.to_string()))?;
                    entries.push((path.clone(), false, git_blob));
                }
                _ => {} // Secret, EnvOverlay, or missing — skip silently
            }
        }
    }

    let tree_bytes = encode_tree(&entries);
    git_repo
        .objects
        .write_buf(gix::object::Kind::Tree, &tree_bytes)
        .map_err(|e| Error::GitProjection(e.to_string()))
}

/// Encode git tree binary format.
/// Entry format: `"{mode} {name}\0{20-byte-sha}"`
/// Sort order: directories sort as if they have a trailing `/`.
fn encode_tree(entries: &[(String, bool, gix::ObjectId)]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|(a_name, a_dir, _), (b_name, b_dir, _)| {
        let a_key = if *a_dir { format!("{}/", a_name) } else { a_name.clone() };
        let b_key = if *b_dir { format!("{}/", b_name) } else { b_name.clone() };
        a_key.as_bytes().cmp(b_key.as_bytes())
    });
    let mut buf = Vec::new();
    for (name, is_dir, sha) in &sorted {
        let mode: &[u8] = if *is_dir { b"40000" } else { b"100644" };
        buf.extend_from_slice(mode);
        buf.push(b' ');
        buf.extend_from_slice(name.as_bytes());
        buf.push(0u8);
        buf.extend_from_slice(sha.as_bytes());
    }
    buf
}

/// Encode git commit text format (no leading header — gix adds it via write_buf).
fn encode_commit(
    tree: gix::ObjectId,
    parents: &[gix::ObjectId],
    identity: &str,
    message: &str,
) -> Vec<u8> {
    let mut s = format!("tree {}\n", tree);
    for p in parents {
        s.push_str(&format!("parent {}\n", p));
    }
    s.push_str(&format!("author {}\n", identity));
    s.push_str(&format!("committer {}\n", identity));
    s.push('\n');
    s.push_str(message);
    if !message.ends_with('\n') {
        s.push('\n');
    }
    s.into_bytes()
}

/// Write a loose ref file in a bare git repo.
/// `ref_name` examples: `"refs/heads/main"`, `"refs/tags/v1.0"`.
/// Content: `"{sha_hex}\n"`.
fn write_loose_ref(repo_path: &Path, ref_name: &str, sha: &gix::ObjectId) -> Result<()> {
    let ref_path = repo_path.join(ref_name);
    if let Some(parent) = ref_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&ref_path, format!("{}\n", sha)).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::Accessor;
    use crate::object::{Blob, EntryKind, Snapshot, TreeEntry};
    use crate::refs::{RefName, TimelinePolicy};
    use bytes::Bytes;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    // ---------- repo builders ----------

    async fn linear_repo() -> (Repository, ObjectId, ObjectId, ObjectId) {
        let repo = Repository::memory();

        let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
        let mut e1 = BTreeMap::new();
        e1.insert("app.rs".into(), TreeEntry { id: b1, kind: EntryKind::Blob });
        let t1 = repo.objects.put_tree(e1).await.unwrap();
        let s1 = repo.objects.put_snapshot(Snapshot {
            root: t1, parents: vec![],
            author: "alice".into(), created_at: 1000, message: "first".into(),
        }).await.unwrap();

        let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
        let mut e2 = BTreeMap::new();
        e2.insert("app.rs".into(), TreeEntry { id: b2, kind: EntryKind::Blob });
        let t2 = repo.objects.put_tree(e2).await.unwrap();
        let s2 = repo.objects.put_snapshot(Snapshot {
            root: t2, parents: vec![s1],
            author: "alice".into(), created_at: 2000, message: "second".into(),
        }).await.unwrap();

        let b3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
        let mut e3 = BTreeMap::new();
        e3.insert("app.rs".into(), TreeEntry { id: b3, kind: EntryKind::Blob });
        let t3 = repo.objects.put_tree(e3).await.unwrap();
        let s3 = repo.objects.put_snapshot(Snapshot {
            root: t3, parents: vec![s2],
            author: "alice".into(), created_at: 3000, message: "third".into(),
        }).await.unwrap();

        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name, s3, TimelinePolicy::Unrestricted, 3000, "persistent".into(), None).unwrap();
        (repo, s1, s2, s3)
    }

    // ---------- test helpers ----------

    /// Walk the parent chain in the exported git repo, counting commits.
    fn count_commit_chain(repo_path: &std::path::Path, start_sha: &str) -> usize {
        let mut count = 0usize;
        let mut current = start_sha.trim().to_owned();
        loop {
            let text = read_git_object_text(repo_path, &current);
            count += 1;
            if let Some(line) = text.lines().find(|l| l.starts_with("parent ")) {
                current = line.strip_prefix("parent ").unwrap().trim().to_owned();
            } else {
                break;
            }
        }
        count
    }

    /// Read and zlib-decompress a loose git object, returning its text payload (header stripped).
    fn read_git_object_text(repo_path: &std::path::Path, sha: &str) -> String {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let obj_path = repo_path.join("objects").join(&sha[..2]).join(&sha[2..]);
        let file = std::fs::File::open(&obj_path)
            .unwrap_or_else(|_| panic!("object file missing: {}", obj_path.display()));
        let mut decoder = ZlibDecoder::new(file);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        let nul = raw.iter().position(|&b| b == 0).expect("no nul in object");
        String::from_utf8(raw[nul + 1..].to_vec()).unwrap()
    }

    /// Return raw git tree payload bytes for the tree referenced in a commit text.
    fn read_tree_of_commit(repo_path: &std::path::Path, commit_sha: &str) -> Vec<u8> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let commit_text = read_git_object_text(repo_path, commit_sha);
        let tree_line = commit_text.lines().find(|l| l.starts_with("tree ")).unwrap();
        let tree_sha = tree_line.strip_prefix("tree ").unwrap().trim();
        let obj_path = repo_path.join("objects").join(&tree_sha[..2]).join(&tree_sha[2..]);
        let file = std::fs::File::open(&obj_path).unwrap();
        let mut decoder = ZlibDecoder::new(file);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        let nul = raw.iter().position(|&b| b == 0).unwrap();
        raw[nul + 1..].to_vec()
    }

    /// Return top-level entry names from a raw git tree binary payload.
    fn tree_top_entry_names(tree_data: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut i = 0;
        while i < tree_data.len() {
            let sp = match tree_data[i..].iter().position(|&b| b == b' ') {
                Some(p) => p,
                None => break,
            };
            let after_mode = i + sp + 1;
            let nl = match tree_data[after_mode..].iter().position(|&b| b == 0) {
                Some(p) => p,
                None => break,
            };
            let name = std::str::from_utf8(&tree_data[after_mode..after_mode + nl]).unwrap();
            names.push(name.to_owned());
            i = after_mode + nl + 1 + 20;
        }
        names
    }

    // ---------- unit tests for helpers ----------

    #[test]
    fn encode_tree_sorts_dirs_with_trailing_slash() {
        use gix::ObjectId;
        // "ab" file should sort before "ab" dir because "ab" < "ab/"
        // but git dirs use trailing-slash comparison, so "a/" < "b"
        // meaning a dir named "a" sorts before a file named "b"
        let null_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let entries = vec![
            ("b.rs".to_owned(), false, null_id),
            ("a".to_owned(), true, null_id),
        ];
        let encoded = encode_tree(&entries);
        // First entry should start with "40000 a\0"
        let first_mode_end = encoded.iter().position(|&b| b == b' ').unwrap();
        assert_eq!(&encoded[..first_mode_end], b"40000");
    }

    #[test]
    fn encode_commit_includes_author_and_parent() {
        let null_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let bytes = encode_commit(null_id, &[null_id], "alice <bole@local> 1000 +0000", "msg");
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("author alice <bole@local> 1000 +0000"));
        assert!(text.contains("parent "));
        assert!(text.ends_with("msg\n"));
    }

    #[tokio::test]
    async fn collect_topo_orders_parents_before_children() {
        let repo = Repository::memory();
        let s1 = repo.objects.put_snapshot(Snapshot {
            root: ObjectId::default(), parents: vec![],
            author: "".into(), created_at: 1, message: "".into(),
        }).await.unwrap();
        let s2 = repo.objects.put_snapshot(Snapshot {
            root: ObjectId::default(), parents: vec![s1],
            author: "".into(), created_at: 2, message: "".into(),
        }).await.unwrap();
        let ordered = collect_topo(&repo.objects, vec![s2]).await.unwrap();
        assert_eq!(ordered, vec![s1, s2]);
    }

    // ---------- integration tests ----------

    #[tokio::test]
    async fn linear_timeline_exports_branch_ref() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        assert!(target.join("refs").join("heads").join("main").exists());
    }

    #[tokio::test]
    async fn exported_commit_chain_has_three_commits() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let count = count_commit_chain(&target, sha_hex.trim());
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn commit_author_contains_bole_local() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let text = read_git_object_text(&target, sha_hex.trim());
        assert!(text.contains("<bole@local>"), "commit author missing bole@local: {text}");
    }

    #[tokio::test]
    async fn acl_denied_timeline_has_no_branch_ref() {
        use crate::acl::{PathRole, Permission, TimelineRole};
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        // accessor that cannot read the "main" timeline
        let restricted = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "other/**".into(), permission: Permission::Read })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });
        project_to_git(&repo, &target, &restricted).await.unwrap();
        assert!(!target.join("refs/heads/main").exists());
    }

    #[tokio::test]
    async fn secret_entry_silently_skipped_in_tree() {
        use crate::object::Object;
        let dir = tempdir().unwrap();
        let repo = Repository::memory();
        // Store a secret; put its ObjectId directly into a tree as if it were a blob
        let key = [0u8; 32];
        let secret_id = repo.objects.put_secret(b"top-secret", &key).await.unwrap();
        // Build a tree with one real blob and one secret "blob" (wrong kind)
        let real_blob = repo.objects.put_blob(Bytes::from("safe")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("safe.rs".into(), TreeEntry { id: real_blob, kind: EntryKind::Blob });
        entries.insert("secret.key".into(), TreeEntry { id: secret_id, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![],
            author: "".into(), created_at: 1, message: "".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name, snap_id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();

        let target = dir.path().join("out.git");
        let result = project_to_git(&repo, &target, &Accessor::privileged()).await;
        assert!(result.is_ok(), "project_to_git returned error: {:?}", result.err());

        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let tree_bytes = read_tree_of_commit(&target, sha_hex.trim());
        let names = tree_top_entry_names(&tree_bytes);
        assert!(names.contains(&"safe.rs".to_owned()));
        assert!(!names.contains(&"secret.key".to_owned()), "secret entry must be excluded");
    }
}
```

- [ ] **Step 4: Add flate2 dev-dependency**

In `Cargo.toml`, add to `[dev-dependencies]`:

```toml
flate2 = "1"
```

- [ ] **Step 5: Run tests**

```bash
cargo test git_projection 2>&1 | tail -20
```

Expected: all 8 tests in `git_projection` pass. If `gix::ObjectId::null(gix::hash::Kind::Sha1)` doesn't compile, check the gix docs (`cargo doc --open -p gix`) for the correct way to create a null SHA1 ObjectId — alternatives: `gix::ObjectId::null_sha1()` or `gix::hash::ObjectId::null(gix::hash::Kind::Sha1)`.

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -5
```

Expected: all existing tests still pass (no regressions).

- [ ] **Step 7: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | tail -10
```

Fix any warnings before proceeding.

- [ ] **Step 8: Commit and merge**

```bash
git add src/repo/git_projection.rs Cargo.toml Cargo.lock
git commit -m "bole-def: G7-T2 implement project_to_git"
git checkout master
git merge bole-def
git branch -d bole-def
bd close bole-def
```

---

## Task 3: T7 Integration Tests

**Files:**
- Create: `tests/git_projection.rs`

**Interfaces:**
- Consumes: `bole::project_to_git` (re-exported from Task 1)
- Consumes: `bole::Repository`, `bole::Accessor`, `bole::PathRole`, `bole::TimelineRole`, `bole::Permission`
- Consumes: `bole::object::{Blob, EntryKind, Snapshot, TreeEntry}`
- Consumes: `bole::refs::{RefName, TimelinePolicy, Tag}`
- Produces: 5 T7 integration tests (`t7_*`)

**Bead workflow:**
```bash
bd create --title="G7-T3: T7 integration tests" --description="Five T7 integration tests verifying git projection from the library's public API" --type=task --priority=2
# e.g. bole-ghi
bd update bole-ghi --claim
git checkout -b bole-ghi
```

- [ ] **Step 1: Create the test file**

Create `tests/git_projection.rs`:

```rust
// bole-ghi
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, Tag, TimelinePolicy};
use bole::{Accessor, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use flate2::read::ZlibDecoder;
use std::collections::BTreeMap;
use std::io::Read;
use tempfile::tempdir;

// ---------- helpers ----------

fn read_git_object_text(repo_path: &std::path::Path, sha: &str) -> String {
    let obj_path = repo_path.join("objects").join(&sha[..2]).join(&sha[2..]);
    let file = std::fs::File::open(&obj_path)
        .unwrap_or_else(|_| panic!("object missing: {sha}"));
    let mut decoder = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let nul = raw.iter().position(|&b| b == 0).unwrap();
    String::from_utf8(raw[nul + 1..].to_vec()).unwrap()
}

fn read_tree_bytes(repo_path: &std::path::Path, tree_sha: &str) -> Vec<u8> {
    let obj_path = repo_path.join("objects").join(&tree_sha[..2]).join(&tree_sha[2..]);
    let file = std::fs::File::open(&obj_path).unwrap();
    let mut decoder = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let nul = raw.iter().position(|&b| b == 0).unwrap();
    raw[nul + 1..].to_vec()
}

fn tree_entry_names(tree_bytes: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = 0;
    while i < tree_bytes.len() {
        let sp = match tree_bytes[i..].iter().position(|&b| b == b' ') {
            Some(p) => p,
            None => break,
        };
        let after_mode = i + sp + 1;
        let nl = match tree_bytes[after_mode..].iter().position(|&b| b == 0) {
            Some(p) => p,
            None => break,
        };
        let name = std::str::from_utf8(&tree_bytes[after_mode..after_mode + nl]).unwrap();
        names.push(name.to_owned());
        i = after_mode + nl + 1 + 20;
    }
    names
}

fn commit_parent_sha(commit_text: &str) -> Option<String> {
    commit_text
        .lines()
        .find(|l| l.starts_with("parent "))
        .map(|l| l.strip_prefix("parent ").unwrap().trim().to_owned())
}

fn commit_tree_sha(commit_text: &str) -> String {
    commit_text
        .lines()
        .find(|l| l.starts_with("tree "))
        .unwrap()
        .strip_prefix("tree ")
        .unwrap()
        .trim()
        .to_owned()
}

fn count_chain(repo_path: &std::path::Path, start: &str) -> usize {
    let mut count = 0;
    let mut current = start.trim().to_owned();
    loop {
        let text = read_git_object_text(repo_path, &current);
        count += 1;
        match commit_parent_sha(&text) {
            Some(p) => current = p,
            None => break,
        }
    }
    count
}

fn object_exists(repo_path: &std::path::Path, sha: &str) -> bool {
    repo_path.join("objects").join(&sha[..2]).join(&sha[2..]).exists()
}

// ---------- T7 tests ----------

/// T7-1: A linear 3-commit timeline projects a branch ref with correct parent chain.
#[tokio::test]
async fn t7_linear_timeline_projects() {
    let repo = Repository::memory();

    let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
    let mut e1 = BTreeMap::new();
    e1.insert("app.rs".into(), TreeEntry { id: b1, kind: EntryKind::Blob });
    let t1 = repo.objects.put_tree(e1).await.unwrap();
    let s1 = repo.objects.put_snapshot(Snapshot {
        root: t1, parents: vec![],
        author: "alice".into(), created_at: 1000, message: "first".into(),
    }).await.unwrap();

    let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
    let mut e2 = BTreeMap::new();
    e2.insert("app.rs".into(), TreeEntry { id: b2, kind: EntryKind::Blob });
    let t2 = repo.objects.put_tree(e2).await.unwrap();
    let s2 = repo.objects.put_snapshot(Snapshot {
        root: t2, parents: vec![s1],
        author: "alice".into(), created_at: 2000, message: "second".into(),
    }).await.unwrap();

    let b3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
    let mut e3 = BTreeMap::new();
    e3.insert("app.rs".into(), TreeEntry { id: b3, kind: EntryKind::Blob });
    let t3 = repo.objects.put_tree(e3).await.unwrap();
    let s3 = repo.objects.put_snapshot(Snapshot {
        root: t3, parents: vec![s2],
        author: "alice".into(), created_at: 3000, message: "third".into(),
    }).await.unwrap();

    let name = RefName::new("main").unwrap();
    repo.refs.create_timeline(name, s3, TimelinePolicy::Unrestricted, 3000, "persistent".into(), None).unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();

    // branch ref exists
    let ref_path = target.join("refs/heads/main");
    assert!(ref_path.exists(), "refs/heads/main missing");

    // 3-commit chain
    let head_sha = std::fs::read_to_string(&ref_path).unwrap();
    let head_sha = head_sha.trim();
    assert_eq!(count_chain(&target, head_sha), 3, "expected 3 commits");

    // head commit author contains bole@local
    let head_text = read_git_object_text(&target, head_sha);
    assert!(head_text.contains("<bole@local>"), "author missing bole@local: {head_text}");

    // second commit is the parent of third
    let parent_sha = commit_parent_sha(&head_text).expect("head commit has no parent");
    let parent_text = read_git_object_text(&target, &parent_sha);
    assert!(parent_text.contains("second"), "parent commit message wrong: {parent_text}");
}

/// T7-2: ACL-filtered projection excludes private paths.
#[tokio::test]
async fn t7_private_paths_excluded() {
    use bole::AclStore;

    let repo = Repository::memory();

    // Mark secrets/ as protected
    repo.acls.set_path_acl(bole::PathAcl {
        glob: "secrets/**".into(),
        protected: true,
    }).unwrap();

    let pub_blob = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
    let sec_blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/app.rs".into(), TreeEntry { id: pub_blob, kind: EntryKind::Blob });
    entries.insert("secrets/key".into(), TreeEntry { id: sec_blob, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![],
        author: "bob".into(), created_at: 1, message: "init".into(),
    }).await.unwrap();
    repo.refs.create_timeline(
        RefName::new("main").unwrap(), snap_id,
        TimelinePolicy::Unrestricted, 1, "persistent".into(), None,
    ).unwrap();

    // Projection 1: accessor that can only read src/**
    let restricted = Accessor::new()
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
        .with_path_role(PathRole { glob: "src/**".into(), permission: Permission::Read });
    let dir1 = tempdir().unwrap();
    let t1 = dir1.path().join("out.git");
    bole::project_to_git(&repo, &t1, &restricted).await.unwrap();

    let sha1 = std::fs::read_to_string(t1.join("refs/heads/main")).unwrap();
    let tree1 = read_tree_bytes(&t1, &commit_tree_sha(&read_git_object_text(&t1, sha1.trim())));
    let names1 = tree_entry_names(&tree1);
    assert!(names1.contains(&"src".to_owned()), "src dir missing from restricted projection");
    assert!(!names1.contains(&"secrets".to_owned()), "secrets dir must be excluded");

    // Projection 2: privileged accessor sees both
    let dir2 = tempdir().unwrap();
    let t2 = dir2.path().join("out.git");
    bole::project_to_git(&repo, &t2, &Accessor::privileged()).await.unwrap();

    let sha2 = std::fs::read_to_string(t2.join("refs/heads/main")).unwrap();
    let tree2 = read_tree_bytes(&t2, &commit_tree_sha(&read_git_object_text(&t2, sha2.trim())));
    let names2 = tree_entry_names(&tree2);
    assert!(names2.contains(&"src".to_owned()));
    assert!(names2.contains(&"secrets".to_owned()), "privileged projection must include secrets");
}

/// T7-3: Two branches sharing an ancestor write the ancestor commit exactly once.
#[tokio::test]
async fn t7_shared_ancestry_deduplicated() {
    let repo = Repository::memory();

    // Common base snapshot
    let b = repo.objects.put_blob(Bytes::from("base")).await.unwrap();
    let mut be = BTreeMap::new();
    be.insert("base.rs".into(), TreeEntry { id: b, kind: EntryKind::Blob });
    let bt = repo.objects.put_tree(be).await.unwrap();
    let base = repo.objects.put_snapshot(Snapshot {
        root: bt, parents: vec![],
        author: "".into(), created_at: 1, message: "base".into(),
    }).await.unwrap();

    // Branch A: one commit on top of base
    let ba = repo.objects.put_blob(Bytes::from("a")).await.unwrap();
    let mut ae = BTreeMap::new();
    ae.insert("a.rs".into(), TreeEntry { id: ba, kind: EntryKind::Blob });
    let at = repo.objects.put_tree(ae).await.unwrap();
    let sa = repo.objects.put_snapshot(Snapshot {
        root: at, parents: vec![base],
        author: "".into(), created_at: 2, message: "a".into(),
    }).await.unwrap();

    // Branch B: one commit on top of base
    let bb = repo.objects.put_blob(Bytes::from("b")).await.unwrap();
    let mut be2 = BTreeMap::new();
    be2.insert("b.rs".into(), TreeEntry { id: bb, kind: EntryKind::Blob });
    let bt2 = repo.objects.put_tree(be2).await.unwrap();
    let sb = repo.objects.put_snapshot(Snapshot {
        root: bt2, parents: vec![base],
        author: "".into(), created_at: 3, message: "b".into(),
    }).await.unwrap();

    repo.refs.create_timeline(RefName::new("branch-a").unwrap(), sa, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).unwrap();
    repo.refs.create_timeline(RefName::new("branch-b").unwrap(), sb, TimelinePolicy::Unrestricted, 3, "persistent".into(), None).unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();

    assert!(target.join("refs/heads/branch-a").exists());
    assert!(target.join("refs/heads/branch-b").exists());

    let sha_a = std::fs::read_to_string(target.join("refs/heads/branch-a")).unwrap();
    let sha_b = std::fs::read_to_string(target.join("refs/heads/branch-b")).unwrap();

    // Each branch head should have the same parent commit (the common ancestor)
    let text_a = read_git_object_text(&target, sha_a.trim());
    let text_b = read_git_object_text(&target, sha_b.trim());
    let parent_a = commit_parent_sha(&text_a).expect("branch-a has no parent");
    let parent_b = commit_parent_sha(&text_b).expect("branch-b has no parent");
    assert_eq!(parent_a, parent_b, "both branches should share the same ancestor commit SHA");

    // The ancestor object exists exactly once (file-based, so presence is sufficient)
    assert!(object_exists(&target, &parent_a), "common ancestor object file missing");
}

/// T7-4: A bole tag projects to refs/tags/{name} pointing at the correct commit.
#[tokio::test]
async fn t7_tags_projected() {
    let repo = Repository::memory();

    let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
    let mut e1 = BTreeMap::new();
    e1.insert("main.rs".into(), TreeEntry { id: b1, kind: EntryKind::Blob });
    let t1 = repo.objects.put_tree(e1).await.unwrap();
    let s1 = repo.objects.put_snapshot(Snapshot {
        root: t1, parents: vec![],
        author: "".into(), created_at: 1, message: "initial".into(),
    }).await.unwrap();

    let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
    let mut e2 = BTreeMap::new();
    e2.insert("main.rs".into(), TreeEntry { id: b2, kind: EntryKind::Blob });
    let t2 = repo.objects.put_tree(e2).await.unwrap();
    let s2 = repo.objects.put_snapshot(Snapshot {
        root: t2, parents: vec![s1],
        author: "".into(), created_at: 2, message: "second".into(),
    }).await.unwrap();

    repo.refs.create_timeline(RefName::new("main").unwrap(), s2, TimelinePolicy::Unrestricted, 2, "persistent".into(), None).unwrap();
    // Tag pointing at the first snapshot
    repo.refs.create_tag(RefName::new("v1.0").unwrap(), s1, None, 1).unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();

    let tag_path = target.join("refs/tags/v1.0");
    assert!(tag_path.exists(), "refs/tags/v1.0 missing");

    // tag SHA should equal the git commit for s1 (the first snapshot)
    let tag_sha = std::fs::read_to_string(&tag_path).unwrap();
    let tag_sha = tag_sha.trim();
    let tag_text = read_git_object_text(&target, tag_sha);
    assert!(tag_text.contains("initial"), "tag should point to initial commit: {tag_text}");

    // second commit exists and the branch points to it
    let head_sha = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
    let head_sha = head_sha.trim();
    let head_text = read_git_object_text(&target, head_sha);
    assert!(head_text.contains("second"), "head should be second commit: {head_text}");

    // the two SHAs should differ
    assert_ne!(tag_sha, head_sha, "tag and head should point to different commits");
}

/// T7-5: Secret objects stored in a tree are silently excluded; project_to_git returns Ok.
#[tokio::test]
async fn t7_secret_entries_skipped() {
    let repo = Repository::memory();

    // Store a secret; its ObjectId is used as a fake "blob" tree entry
    let key = [0u8; 32];
    let secret_id = repo.objects.put_secret(b"very-secret", &key).await.unwrap();

    let safe_blob = repo.objects.put_blob(Bytes::from("safe content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("safe.rs".into(), TreeEntry { id: safe_blob, kind: EntryKind::Blob });
    entries.insert("secret.key".into(), TreeEntry { id: secret_id, kind: EntryKind::Blob });
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo.objects.put_snapshot(Snapshot {
        root: tree_id, parents: vec![],
        author: "".into(), created_at: 1, message: "with secret".into(),
    }).await.unwrap();
    repo.refs.create_timeline(
        RefName::new("main").unwrap(), snap_id,
        TimelinePolicy::Unrestricted, 1, "persistent".into(), None,
    ).unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    let result = bole::project_to_git(&repo, &target, &Accessor::privileged()).await;
    assert!(result.is_ok(), "project_to_git must not error on secret entries: {:?}", result.err());

    let head_sha = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
    let tree_sha = commit_tree_sha(&read_git_object_text(&target, head_sha.trim()));
    let tree_bytes = read_tree_bytes(&target, &tree_sha);
    let names = tree_entry_names(&tree_bytes);

    assert!(names.contains(&"safe.rs".to_owned()), "safe.rs must be present");
    assert!(!names.contains(&"secret.key".to_owned()), "secret.key must be excluded");
}
```

- [ ] **Step 2: Run to verify tests compile and pass**

```bash
cargo test --test git_projection 2>&1 | tail -20
```

Expected: 5 T7 tests pass. If `bole::AclStore` or `bole::PathAcl` import fails in `t7_private_paths_excluded`, check what is re-exported from `src/lib.rs` and adjust the import. If `repo.acls.set_path_acl(...)` doesn't exist, use the `AclStore` API that was established in Gate 3 for setting path ACLs.

> **Note on `t7_private_paths_excluded`:** This test uses `repo.acls.set_path_acl(PathAcl { glob, protected })` — check the exact `AclStore` API in `src/acl/mod.rs` and adjust if the method name differs. The test intent is: mark `secrets/**` as ACL-protected, then project with a restricted accessor (no read on `secrets/**`), verify secrets dir is absent from the git tree.

- [ ] **Step 3: Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass (no regressions from Tasks 1 or 2).

- [ ] **Step 4: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1 | tail -10
```

Fix any warnings.

- [ ] **Step 5: Commit and merge**

```bash
git add tests/git_projection.rs
git commit -m "bole-ghi: G7-T3 T7 integration tests"
git checkout master
git merge bole-ghi
git branch -d bole-ghi
bd close bole-ghi
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] `pub async fn project_to_git(repo, target_path, accessor)` — Task 2
- [x] `Error::GitProjection(String)` — Task 1
- [x] `gix = "0.70"` dependency — Task 1
- [x] `pub mod git_projection` in `src/repo/mod.rs` — Task 1
- [x] `pub use repo::git_projection::project_to_git` in `src/lib.rs` — Task 1
- [x] Five passes (init, topo-sort, write objects, branch refs, tag refs) — Task 2
- [x] Topo sort (parents before children) — `collect_topo`, Task 2
- [x] ACL filtering via `walk_tree_filtered` — Task 2
- [x] Secret/EnvOverlay silently skipped — `write_git_tree_level`, Task 2
- [x] Synthetic identity `{author} <bole@local> {created_at} +0000` — `encode_commit`, Task 2
- [x] `refs/heads/{name}` for timelines — Task 2
- [x] `refs/tags/{name}` for tags (lightweight, target in id_map) — Task 2
- [x] ACL-denied timelines excluded from projection — Task 2 (accessor check before pushing to `timeline_heads`)
- [x] T7-1 linear timeline — Task 3
- [x] T7-2 private paths excluded — Task 3
- [x] T7-3 shared ancestry deduplicated — Task 3
- [x] T7-4 tags projected — Task 3
- [x] T7-5 secret entries skipped — Task 3

**Type consistency:**
- `collect_topo` takes `&crate::store::ObjectStore, Vec<ObjectId>` — consistent with Task 2 usage.
- `write_git_tree_level` takes `&BTreeMap<String, ObjectId>` — matches `walk_tree_filtered` output.
- `encode_tree` takes `&[(String, bool, gix::ObjectId)]` — consistent with `write_git_tree_level` `entries` vec.
- `encode_commit` takes `gix::ObjectId`, `&[gix::ObjectId]`, `&str`, `&str` — consistent with Task 2 call.
- `write_loose_ref` takes `&Path`, `&str`, `&gix::ObjectId` — consistent with Task 2 calls.

**gix API note for implementer:** If `git_repo.objects.write_buf(kind, data)` does not compile with the installed gix 0.70, run `cargo doc --open -p gix` and look for the write API on `Repository.objects`. Common alternatives: `git_repo.write_object(obj)` where `obj: impl gix::objs::WriteTo`, or `gix::odb::Write::write_buf`. The `use gix::prelude::Write as _` import brings the trait method into scope.
