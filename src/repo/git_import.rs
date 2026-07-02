// bole-mtq
//! Git import (git → bole), the inverse of [`super::git_projection::project_to_git`].
//!
//! Translates a git repository's branches and tags into bole timelines/tags and
//! their object closure into bole Blobs/Trees/Snapshots, keeping a persisted
//! [`IdentityMap`] sidecar so round-trips and incremental re-imports are stable.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::object::{EntryKind, ObjectId, Snapshot, TreeEntry};
use crate::refs::{RefName, TimelinePolicy};
use crate::repo::Repository;

// bole-mtq
/// Options controlling a git import.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Branches to import (by git short name). Empty = all branches.
    pub branches: Vec<String>,
    /// Policy for newly created timelines.
    pub timeline_policy: TimelinePolicy,
    /// Optional path to a WS1 label-rule file (one `glob` per line → protected).
    pub label_ruleset: Option<PathBuf>,
    /// Translate objects but write nothing and do not persist the sidecar.
    pub dry_run: bool,
    /// Override the timeline policy when advancing a non-fast-forward re-import.
    pub force: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            branches: Vec::new(),
            timeline_policy: TimelinePolicy::Unrestricted,
            label_ruleset: None,
            dry_run: false,
            force: false,
        }
    }
}

// bole-mtq
/// What an import did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub blobs_written: usize,
    pub trees_written: usize,
    pub snapshots_written: usize,
    pub timelines_created: usize,
    pub timelines_advanced: usize,
    pub tags_created: usize,
    pub skipped_via_identity_map: usize,
}

// bole-mtq
/// A persisted bidirectional map between git object ids (SHA-1/SHA-256 bytes) and
/// bole `ObjectId`s (BLAKE3), used to make import/export idempotent and
/// incremental.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMap {
    git_to_bole: HashMap<Vec<u8>, [u8; 32]>,
    bole_to_git: HashMap<[u8; 32], Vec<u8>>,
}

impl IdentityMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a git↔bole correspondence (both directions).
    pub fn insert(&mut self, git: Vec<u8>, bole: ObjectId) {
        self.git_to_bole.insert(git.clone(), *bole.as_bytes());
        self.bole_to_git.insert(*bole.as_bytes(), git);
    }

    /// The bole id a git id was translated to, if known.
    pub fn bole_for_git(&self, git: &[u8]) -> Option<ObjectId> {
        self.git_to_bole.get(git).map(|b| ObjectId::new(*b))
    }

    /// The git id a bole id was translated from/to, if known.
    pub fn git_for_bole(&self, bole: &ObjectId) -> Option<&[u8]> {
        self.bole_to_git.get(bole.as_bytes()).map(|v| v.as_slice())
    }

    pub fn len(&self) -> usize {
        self.git_to_bole.len()
    }

    pub fn is_empty(&self) -> bool {
        self.git_to_bole.is_empty()
    }

    /// Loads the sidecar for `source` under `dir` (empty map if absent).
    pub fn load(dir: &Path, source: &Path) -> Result<Self> {
        let path = map_path(dir, source);
        match std::fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).map_err(|e| Error::Codec(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Persists the sidecar for `source` under `dir` (atomic tmp+rename).
    pub fn save(&self, dir: &Path, source: &Path) -> Result<()> {
        let path = map_path(dir, source);
        std::fs::create_dir_all(path.parent().expect("map path has a parent")).map_err(Error::Io)?;
        let bytes = postcard::to_allocvec(self).map_err(|e| Error::Codec(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(Error::Io)?;
        std::fs::rename(&tmp, &path).map_err(Error::Io)?;
        Ok(())
    }
}

// bole-mtq
/// The sidecar path for a source: `<dir>/git-map/<fingerprint>.postcard`.
fn map_path(dir: &Path, source: &Path) -> PathBuf {
    dir.join("git-map").join(format!("{}.postcard", fingerprint(source)))
}

// bole-mtq
/// A deterministic per-source fingerprint: hex of BLAKE3 over the canonical
/// absolute path bytes (the spec's SHA-256 role — a stable filename key).
fn fingerprint(source: &Path) -> String {
    let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex().to_string()
}

// bole-mtq
/// Sanitizes a git ref name into a bole-legal `RefName` string, or `None` if it
/// is empty after sanitization. Deterministic: `.hidden` → `_hidden`,
/// `feature/.foo` → `feature/_foo`; NUL bytes stripped; consecutive slashes
/// collapsed. Slashes are otherwise preserved.
pub fn sanitize_ref_name(git_name: &str) -> Option<String> {
    let segments: Vec<String> = git_name
        .split('/')
        .filter(|s| !s.is_empty()) // collapse consecutive/leading/trailing slashes
        .map(|s| {
            let s: String = s.chars().filter(|c| *c != '\0').collect();
            if let Some(rest) = s.strip_prefix('.') {
                format!("_{rest}")
            } else {
                s
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("/"))
    }
}

// bole-mtq
/// Imports all (or the selected) branches and tags from the git repo at `source`
/// into `repo`, translating the reachable object closure into bole objects and
/// creating/advancing timelines and tags. Uses (and updates) the `IdentityMap`
/// sidecar under `identity_map_dir` for idempotent, incremental round-trips.
pub async fn git_import(
    repo: &Repository,
    source: &Path,
    identity_map_dir: &Path,
    opts: ImportOptions,
) -> Result<ImportSummary> {
    let git = gix::open(source)
        .map_err(|e| Error::GitProjection(format!("opening git repo: {e}")))?;
    let mut map = IdentityMap::load(identity_map_dir, source)?;
    let mut summary = ImportSummary::default();

    // Pass 3: collect branches and tags (filtered by opts.branches).
    let mut branches: Vec<(String, gix::ObjectId)> = Vec::new();
    let mut tags: Vec<(String, gix::ObjectId)> = Vec::new();
    for r in git
        .references()
        .map_err(|e| Error::GitProjection(e.to_string()))?
        .all()
        .map_err(|e| Error::GitProjection(e.to_string()))?
    {
        let mut r = r.map_err(|e| Error::GitProjection(e.to_string()))?;
        let full = r.name().as_bstr().to_string();
        // bole-jio: peel_to_id_in_place was deprecated in gix 0.74.
        let target = r
            .peel_to_id()
            .map_err(|e| Error::GitProjection(e.to_string()))?
            .detach();
        if let Some(short) = full.strip_prefix("refs/heads/") {
            if opts.branches.is_empty() || opts.branches.iter().any(|b| b == short) {
                branches.push((short.to_string(), target));
            }
        } else if let Some(short) = full.strip_prefix("refs/tags/") {
            tags.push((short.to_string(), target));
        }
    }

    // Passes 4-6: translate the commit closure of every branch head (parents
    // first), which recursively translates trees and blobs.
    let heads: Vec<gix::ObjectId> = branches.iter().map(|(_, h)| *h).collect();
    let ordered = topo_commits(&git, &heads)?;
    for commit_oid in &ordered {
        translate_commit(repo, &git, *commit_oid, &mut map, &mut summary, opts.dry_run).await?;
    }

    // Pass 7: branches → timelines (create or advance).
    let now = 0u64; // deterministic import timestamp (WS7 may thread a real clock)
    let mut seen_names: HashMap<String, String> = HashMap::new();
    if !opts.dry_run {
        for (short, head) in &branches {
            let name = sanitized_ref(short, &mut seen_names)?;
            let bole_head = map
                .bole_for_git(head.as_bytes())
                .ok_or_else(|| Error::GitProjection(format!("untranslated branch head: {short}")))?;
            match repo.refs.get_timeline(&name)? {
                None => {
                    repo.refs.create_timeline(
                        name,
                        bole_head,
                        opts.timeline_policy.clone(),
                        now,
                        "persistent".into(),
                        None,
                    )?;
                    summary.timelines_created += 1;
                }
                Some(tl) => {
                    // Incremental advance, policy-gated unless --force.
                    let ff = repo.find_common_ancestor(tl.head, bole_head).await? == Some(tl.head);
                    let allowed = opts.force
                        || matches!(tl.policy, TimelinePolicy::Unrestricted)
                        || ff;
                    if !allowed {
                        return Err(Error::GitProjection(format!(
                            "non-fast-forward re-import of '{}' (use force)",
                            name.as_str()
                        )));
                    }
                    if tl.head != bole_head {
                        repo.refs.advance_head(&name, bole_head)?;
                        summary.timelines_advanced += 1;
                    }
                }
            }
        }

        // Pass 8: tags (lightweight + annotated).
        for (short, target) in &tags {
            let name = sanitized_ref(short, &mut seen_names)?;
            translate_tag(repo, &git, &name, *target, &mut map, &mut summary).await?;
        }

        // Pass 9: apply label rules (WS1).
        if let Some(ruleset) = &opts.label_ruleset {
            apply_label_ruleset(repo, ruleset)?;
        }

        // Pass 10: persist the identity map.
        map.save(identity_map_dir, source)?;
    }

    Ok(summary)
}

// bole-mtq
/// Sanitizes `short` to a `RefName`, aborting on a collision with a different
/// git name already seen.
fn sanitized_ref(short: &str, seen: &mut HashMap<String, String>) -> Result<RefName> {
    let sanitized = sanitize_ref_name(short)
        .ok_or_else(|| Error::GitProjection(format!("ref name empty after sanitization: {short}")))?;
    if let Some(prev) = seen.get(&sanitized) {
        if prev != short {
            return Err(Error::GitProjection(format!(
                "git refs '{prev}' and '{short}' both sanitize to '{sanitized}'"
            )));
        }
    }
    seen.insert(sanitized.clone(), short.to_string());
    RefName::new(sanitized).map_err(|e| Error::GitProjection(e.to_string()))
}

// bole-mtq
/// Post-order (parents before children) topological sort of the commit DAG
/// reachable from `heads`.
fn topo_commits(git: &gix::Repository, heads: &[gix::ObjectId]) -> Result<Vec<gix::ObjectId>> {
    let mut visited: HashSet<gix::ObjectId> = HashSet::new();
    let mut result = Vec::new();
    let mut stack: Vec<(gix::ObjectId, bool)> = heads.iter().map(|h| (*h, false)).collect();
    while let Some((id, finishing)) = stack.pop() {
        if finishing {
            result.push(id);
            continue;
        }
        if !visited.insert(id) {
            continue;
        }
        stack.push((id, true));
        let commit = git
            .find_object(id)
            .map_err(|e| Error::GitProjection(e.to_string()))?
            .try_into_commit()
            .map_err(|e| Error::GitProjection(e.to_string()))?;
        for parent in commit.parent_ids() {
            let pid = parent.detach();
            if !visited.contains(&pid) {
                stack.push((pid, false));
            }
        }
    }
    Ok(result)
}

// bole-mtq
/// Translates one git commit (its tree already reachable) into a bole Snapshot.
async fn translate_commit(
    repo: &Repository,
    git: &gix::Repository,
    oid: gix::ObjectId,
    map: &mut IdentityMap,
    summary: &mut ImportSummary,
    dry_run: bool,
) -> Result<ObjectId> {
    if let Some(existing) = map.bole_for_git(oid.as_bytes()) {
        summary.skipped_via_identity_map += 1;
        return Ok(existing);
    }
    let commit = git
        .find_object(oid)
        .map_err(|e| Error::GitProjection(e.to_string()))?
        .try_into_commit()
        .map_err(|e| Error::GitProjection(e.to_string()))?;
    let tree_oid = commit.tree_id().map_err(|e| Error::GitProjection(e.to_string()))?.detach();
    let root = translate_tree(repo, git, tree_oid, map, summary, dry_run).await?;

    let mut parents = Vec::new();
    for parent in commit.parent_ids() {
        let pid = parent.detach();
        let bole = map.bole_for_git(pid.as_bytes()).ok_or_else(|| {
            Error::GitProjection("parent commit not translated before child".into())
        })?;
        parents.push(bole);
    }

    let author = commit.author().map_err(|e| Error::GitProjection(e.to_string()))?;
    let author_str = format!("{} <{}>", author.name, author.email);
    let created_at = author.seconds().max(0) as u64;
    let mut message = commit.message_raw_sloppy().to_string();
    if let Ok(committer) = commit.committer() {
        if committer.name != author.name || committer.email != author.email {
            message = format!(
                "{}\n\nCommitter: {} <{}> {}",
                message.trim_end(),
                committer.name,
                committer.email,
                committer.seconds()
            );
        }
    }

    let snap = Snapshot { root, parents, author: author_str, created_at, message };
    let id = if dry_run {
        // Compute the id without storing (content-addressed).
        crate::codec::object_id(&postcard::to_allocvec(&crate::object::Object::Snapshot(snap)).map_err(|e| Error::Codec(e.to_string()))?)
    } else {
        repo.objects.put_snapshot(snap).await?
    };
    map.insert(oid.as_bytes().to_vec(), id);
    summary.snapshots_written += 1;
    Ok(id)
}

// bole-mtq
/// Translates a git tree into a bole Tree, recursing into subtrees and blobs.
async fn translate_tree(
    repo: &Repository,
    git: &gix::Repository,
    oid: gix::ObjectId,
    map: &mut IdentityMap,
    summary: &mut ImportSummary,
    dry_run: bool,
) -> Result<ObjectId> {
    if let Some(existing) = map.bole_for_git(oid.as_bytes()) {
        return Ok(existing);
    }
    let tree = git
        .find_object(oid)
        .map_err(|e| Error::GitProjection(e.to_string()))?
        .try_into_tree()
        .map_err(|e| Error::GitProjection(e.to_string()))?;
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for entry in tree.iter() {
        let entry = entry.map_err(|e| Error::GitProjection(e.to_string()))?;
        let mode = entry.mode();
        let name = entry.filename().to_string();
        let child = entry.oid().to_owned();
        if mode.is_tree() {
            let id = Box::pin(translate_tree(repo, git, child, map, summary, dry_run)).await?;
            entries.insert(name, TreeEntry { id, kind: EntryKind::Tree });
        } else if mode.is_commit() {
            // Submodule (gitlink): out of scope, skip.
            continue;
        } else {
            // Blob (regular / executable / symlink-as-blob).
            let id = translate_blob(repo, git, child, map, summary, dry_run).await?;
            entries.insert(name, TreeEntry { id, kind: EntryKind::Blob });
        }
    }
    let id = if dry_run {
        crate::codec::object_id(
            &postcard::to_allocvec(&crate::object::Object::Tree(crate::object::Tree {
                entries: entries.clone(),
            }))
            .map_err(|e| Error::Codec(e.to_string()))?,
        )
    } else {
        repo.objects.put_tree(entries).await?
    };
    map.insert(oid.as_bytes().to_vec(), id);
    summary.trees_written += 1;
    Ok(id)
}

// bole-mtq
/// Translates a git blob into a bole Blob (verbatim content).
async fn translate_blob(
    repo: &Repository,
    git: &gix::Repository,
    oid: gix::ObjectId,
    map: &mut IdentityMap,
    summary: &mut ImportSummary,
    dry_run: bool,
) -> Result<ObjectId> {
    if let Some(existing) = map.bole_for_git(oid.as_bytes()) {
        return Ok(existing);
    }
    let blob = git
        .find_object(oid)
        .map_err(|e| Error::GitProjection(e.to_string()))?;
    let data = Bytes::from(blob.data.clone());
    let id = if dry_run {
        crate::codec::object_id(
            &postcard::to_allocvec(&crate::object::Object::Blob(crate::object::Blob {
                data: data.clone(),
            }))
            .map_err(|e| Error::Codec(e.to_string()))?,
        )
    } else {
        repo.objects.put_blob(data).await?
    };
    map.insert(oid.as_bytes().to_vec(), id);
    summary.blobs_written += 1;
    Ok(id)
}

// bole-mtq
/// Creates a bole Tag from a git tag ref (lightweight or annotated). Annotated
/// tag messages/taggers are preserved (tagger as a message trailer).
async fn translate_tag(
    repo: &Repository,
    git: &gix::Repository,
    name: &RefName,
    target: gix::ObjectId,
    map: &mut IdentityMap,
    summary: &mut ImportSummary,
) -> Result<()> {
    // `target` has been peeled to a commit id already; find the bole snapshot.
    let bole_target = match map.bole_for_git(target.as_bytes()) {
        Some(t) => t,
        None => return Ok(()), // target commit not imported (e.g. filtered branch)
    };
    // Distinguish annotated tags: the ref's *unpeeled* object may be a tag.
    let (message, created_at) = annotated_tag_meta(git, name.as_str(), bole_target, repo).await?;

    if let Some(existing) = repo.refs.get_tag(name)? {
        if existing.target == bole_target {
            return Ok(()); // unchanged
        }
        // Force-pushed tag: bole tags are immutable; log-and-skip (no move).
        return Ok(());
    }
    repo.refs.create_tag(name.clone(), bole_target, message, created_at)?;
    summary.tags_created += 1;
    Ok(())
}

// bole-mtq
/// Returns `(tag_message, created_at)` for a tag ref: annotated tags contribute
/// their message (+ a `Tagger:` trailer); lightweight tags default to the target
/// snapshot's `created_at` and no message.
async fn annotated_tag_meta(
    git: &gix::Repository,
    short: &str,
    bole_target: ObjectId,
    repo: &Repository,
) -> Result<(Option<String>, u64)> {
    let full = format!("refs/tags/{short}");
    if let Ok(reference) = git.find_reference(full.as_str()) {
        let target_id = reference.target().try_id().map(|id| id.to_owned());
        if let Some(tid) = target_id {
            if let Ok(obj) = git.find_object(tid) {
                if obj.kind == gix::object::Kind::Tag {
                    if let Ok(tag) = obj.try_into_tag() {
                        let decoded =
                            tag.decode().map_err(|e| Error::GitProjection(e.to_string()))?;
                        let (tagger_line, created_at) = match decoded.tagger {
                            Some(t) => (
                                format!("\n\nTagger: {} <{}> {}", t.name, t.email, t.seconds()),
                                t.seconds().max(0) as u64,
                            ),
                            None => (String::new(), 0),
                        };
                        let msg = format!("{}{}", decoded.message, tagger_line);
                        return Ok((Some(msg), created_at));
                    }
                }
            }
        }
    }
    // Lightweight tag: use the target snapshot's created_at.
    let created_at = match repo.objects.get(&bole_target).await? {
        Some(crate::object::Object::Snapshot(s)) => s.created_at,
        _ => 0,
    };
    Ok((None, created_at))
}

// bole-mtq
/// Installs WS1 `PathAcl` protection rules from a ruleset file (one glob per
/// non-empty, non-comment line → `protected`).
fn apply_label_ruleset(repo: &Repository, ruleset: &Path) -> Result<()> {
    let text = std::fs::read_to_string(ruleset).map_err(Error::Io)?;
    for line in text.lines() {
        let glob = line.trim();
        if glob.is_empty() || glob.starts_with('#') {
            continue;
        }
        repo.acls.set_path_acl(crate::acl::PathAcl { glob: glob.to_string() })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_roundtrip_postcard() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = dir.path().join("some-git-repo");
        std::fs::create_dir_all(&source).unwrap();

        let mut map = IdentityMap::new();
        let bole = ObjectId::from_content(b"snapshot");
        map.insert(vec![0xab; 20], bole);
        map.save(dir.path(), &source).unwrap();

        let loaded = IdentityMap::load(dir.path(), &source).unwrap();
        assert_eq!(loaded, map);
        assert_eq!(loaded.bole_for_git(&[0xab; 20]), Some(bole));
        assert_eq!(loaded.git_for_bole(&bole), Some(&[0xab; 20][..]));
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_missing_map_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let map = IdentityMap::load(dir.path(), Path::new("/nonexistent/repo")).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn sanitize_dot_prefixed_segments() {
        assert_eq!(sanitize_ref_name(".hidden").as_deref(), Some("_hidden"));
        assert_eq!(sanitize_ref_name("feature/.foo").as_deref(), Some("feature/_foo"));
        assert_eq!(sanitize_ref_name("main").as_deref(), Some("main"));
        assert_eq!(sanitize_ref_name("a//b").as_deref(), Some("a/b"));
        assert_eq!(sanitize_ref_name("release/1.0").as_deref(), Some("release/1.0"));
    }

    #[test]
    fn sanitize_empty_is_none() {
        assert_eq!(sanitize_ref_name(""), None);
        assert_eq!(sanitize_ref_name("///"), None);
    }

    #[test]
    fn different_sources_get_different_fingerprints() {
        let a = fingerprint(Path::new("/tmp/repo-a"));
        let b = fingerprint(Path::new("/tmp/repo-b"));
        assert_ne!(a, b);
        // Deterministic for the same path.
        assert_eq!(a, fingerprint(Path::new("/tmp/repo-a")));
    }

    async fn commit(
        repo: &Repository,
        parent: Option<ObjectId>,
        file: &str,
        content: &[u8],
    ) -> ObjectId {
        let blob = repo.objects.put_blob(Bytes::copy_from_slice(content)).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert(file.to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(e).await.unwrap();
        repo.objects
            .put_snapshot(Snapshot {
                root: tree,
                parents: parent.into_iter().collect(),
                author: "Alice <a@example.com>".into(),
                created_at: 100,
                message: "c".into(),
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn roundtrip_export_then_import() {
        use crate::acl::Accessor;
        // Build a 3-commit history on `main` in a bole repo.
        let src = Repository::memory();
        let c1 = commit(&src, None, "app.rs", b"fn main(){}").await;
        let c2 = commit(&src, Some(c1), "app.rs", b"fn main(){2}").await;
        let c3 = commit(&src, Some(c2), "app.rs", b"fn main(){3}").await;
        src.refs
            .create_timeline(RefName::new("main").unwrap(), c3, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        src.refs.create_tag(RefName::new("v1").unwrap(), c3, None, 100).unwrap();

        // Export to a bare git repo.
        let gdir = tempfile::TempDir::new().unwrap();
        let gpath = gdir.path().join("export.git");
        crate::repo::git_projection::project_to_git(&src, &gpath, &Accessor::privileged())
            .await
            .unwrap();

        // Import into a fresh bole repo.
        let bdir = tempfile::TempDir::new().unwrap();
        let dst = Repository::disk(bdir.path()).await.unwrap();
        let mapdir = bdir.path().join(".bole");
        let summary = git_import(&dst, &gpath, &mapdir, ImportOptions::default()).await.unwrap();

        assert_eq!(summary.timelines_created, 1);
        assert_eq!(summary.snapshots_written, 3, "3 commits");
        assert_eq!(summary.skipped_via_identity_map, 0, "fresh import skips nothing");
        assert_eq!(summary.tags_created, 1);

        // The imported main head has the latest file content, 3-deep history.
        let main = dst.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().unwrap();
        let filtered = dst
            .get_snapshot_filtered(main.head, &Accessor::privileged())
            .await
            .unwrap()
            .unwrap();
        assert!(filtered.visible_paths.contains_key("app.rs"));
        let blob_id = filtered.visible_paths["app.rs"];
        match dst.objects.get(&blob_id).await.unwrap().unwrap() {
            crate::object::Object::Blob(b) => assert_eq!(b.data.as_ref(), b"fn main(){3}"),
            _ => panic!("expected blob"),
        }
        // Tag present.
        assert!(dst.refs.get_tag(&RefName::new("v1").unwrap()).unwrap().is_some());
    }

    #[tokio::test]
    async fn incremental_import_skips_and_advances() {
        use crate::acl::Accessor;
        let src = Repository::memory();
        let c1 = commit(&src, None, "f", b"1").await;
        let c2 = commit(&src, Some(c1), "f", b"2").await;
        src.refs
            .create_timeline(RefName::new("main").unwrap(), c2, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        let gdir = tempfile::TempDir::new().unwrap();
        let gpath = gdir.path().join("e.git");
        crate::repo::git_projection::project_to_git(&src, &gpath, &Accessor::privileged())
            .await
            .unwrap();

        let bdir = tempfile::TempDir::new().unwrap();
        let dst = Repository::disk(bdir.path()).await.unwrap();
        let mapdir = bdir.path().join(".bole");
        let s1 = git_import(&dst, &gpath, &mapdir, ImportOptions::default()).await.unwrap();
        assert_eq!(s1.snapshots_written, 2);
        let head1 = dst.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().unwrap().head;

        // Add a third commit upstream and re-export.
        let c3 = commit(&src, Some(c2), "f", b"3").await;
        src.refs.advance_head(&RefName::new("main").unwrap(), c3).unwrap();
        crate::repo::git_projection::project_to_git(&src, &gpath, &Accessor::privileged())
            .await
            .unwrap();

        // Re-import: only the new commit is written; head advances.
        let s2 = git_import(&dst, &gpath, &mapdir, ImportOptions::default()).await.unwrap();
        assert_eq!(s2.snapshots_written, 1, "only the new commit");
        assert_eq!(s2.skipped_via_identity_map, 2, "prior two skipped via map");
        assert_eq!(s2.timelines_advanced, 1);
        let head2 = dst.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().unwrap().head;
        assert_ne!(head1, head2, "head advanced");
    }

    #[tokio::test]
    async fn annotated_tag_message_roundtrips() {
        use crate::acl::Accessor;
        let src = Repository::memory();
        let c1 = commit(&src, None, "f", b"x").await;
        src.refs
            .create_timeline(RefName::new("main").unwrap(), c1, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        src.refs
            .create_tag(RefName::new("rel").unwrap(), c1, Some("release notes".into()), 100)
            .unwrap();

        let gdir = tempfile::TempDir::new().unwrap();
        let gpath = gdir.path().join("g.git");
        crate::repo::git_projection::project_to_git(&src, &gpath, &Accessor::privileged())
            .await
            .unwrap();

        let bdir = tempfile::TempDir::new().unwrap();
        let dst = Repository::disk(bdir.path()).await.unwrap();
        git_import(&dst, &gpath, &bdir.path().join(".bole"), ImportOptions::default()).await.unwrap();

        let tag = dst.refs.get_tag(&RefName::new("rel").unwrap()).unwrap().unwrap();
        assert!(
            tag.message.as_deref().unwrap_or("").contains("release notes"),
            "annotated message preserved: {:?}",
            tag.message
        );
    }

    #[tokio::test]
    async fn label_ruleset_protects_imported_paths() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::Accessor;
        use std::sync::Arc;

        // Build a repo with a public and a private path via the workspace.
        let src = Repository::memory();
        let mut ws = src.ephemeral_workspace();
        ws.write("src/main.rs", &b"ok"[..]);
        ws.write("private/secret.rs", &b"classified"[..]);
        let snap = ws.commit("a", "m", 0).await.unwrap();
        src.refs
            .create_timeline(RefName::new("main").unwrap(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        let gdir = tempfile::TempDir::new().unwrap();
        let gpath = gdir.path().join("g.git");
        crate::repo::git_projection::project_to_git(&src, &gpath, &Accessor::privileged())
            .await
            .unwrap();

        // Import with a label ruleset marking private/** protected.
        let bdir = tempfile::TempDir::new().unwrap();
        let dst = Repository::disk(bdir.path()).await.unwrap();
        let rules_file = bdir.path().join("rules.txt");
        std::fs::write(&rules_file, "# protect secrets\nprivate/**\n").unwrap();
        let opts = ImportOptions { label_ruleset: Some(rules_file), ..Default::default() };
        git_import(&dst, &gpath, &bdir.path().join(".bole"), opts).await.unwrap();

        let head = dst.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().unwrap().head;

        // A default (no-clearance) accessor sees only the public path.
        let public_view = dst.get_snapshot_filtered(head, &Accessor::new()).await.unwrap().unwrap();
        assert!(public_view.visible_paths.contains_key("src/main.rs"));
        assert!(!public_view.visible_paths.contains_key("private/secret.rs"), "private hidden");

        // A protected-cleared accessor sees both.
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: Label::protected(),
                cap: Capability::READ,
                scope: Some(ClearanceScope::Path("**".into())),
            }],
            confined: false,
        };
        let cleared = Accessor::from_parts(Arc::new(LabelLattice::two_point()), Arc::new(LabelRuleSet::default()), clr);
        let full_view = dst.get_snapshot_filtered(head, &cleared).await.unwrap().unwrap();
        assert!(full_view.visible_paths.contains_key("private/secret.rs"), "cleared sees private");
    }

    #[tokio::test]
    async fn export_mapped_persists_sidecar() {
        use crate::acl::Accessor;
        let src = Repository::memory();
        let c1 = commit(&src, None, "f", b"x").await;
        src.refs
            .create_timeline(RefName::new("main").unwrap(), c1, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        let gpath = dir.path().join("g.git");
        let mapdir = dir.path().join("map");
        crate::repo::git_projection::project_to_git_mapped(&src, &gpath, &Accessor::privileged(), &mapdir)
            .await
            .unwrap();

        // The export recorded the snapshot ↔ commit correspondence.
        let loaded = IdentityMap::load(&mapdir, &gpath).unwrap();
        assert!(!loaded.is_empty());
        assert!(loaded.git_for_bole(&c1).is_some(), "head snapshot mapped to a git commit");
    }
}
