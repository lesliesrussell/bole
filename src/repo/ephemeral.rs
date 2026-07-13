// bole-uxt
//! In-memory worktrees and the shared work-tree ↔ tree primitives.
//!
//! [`EphemeralWorkspace`] is a pure in-RAM working tree: seed it from a
//! snapshot (or start empty), edit files as byte buffers, [`diff`] against the
//! base, and [`commit`] to a new snapshot — no filesystem involved. It is built
//! on any [`Repository`] (typically [`Repository::memory`]) and is the
//! agent/tool counterpart to the CLI's on-disk workspace.
//!
//! The free functions [`build_tree`], [`snapshot_paths`], and [`diff_paths`]
//! are the reusable core of the work-tree algorithm; the CLI's on-disk differ
//! delegates to them so there is a single implementation.
//!
//! [`diff`]: EphemeralWorkspace::diff
//! [`commit`]: EphemeralWorkspace::commit

use std::collections::BTreeMap;
// bole-1kz
use std::path::PathBuf;

use bytes::Bytes;
// bole-phxz
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
use crate::repo::Repository;
use crate::store::ObjectStore;

// bole-1kz
/// The repository metadata directory name. A `DiskWorkspace` walk excludes it,
/// whether it is a directory (primary worktree) or a pointer file (linked
/// worktree). Mirrors the CLI's `context::REPO_DIR`.
pub const REPO_DIR: &str = ".bole";

// bole-phxz
/// The workspace-root ignore file. Patterns use gitignore semantics; the file
/// is itself a tracked, versioned file (not special-cased out of snapshots).
pub const IGNORE_FILE: &str = ".boleignore";

/// A path-level diff between two flat `path → blob id` maps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathDiff {
    /// Paths present in the target but not the base.
    pub added: Vec<String>,
    /// Paths present in the base but not the target.
    pub removed: Vec<String>,
    /// Paths present in both but pointing at different blobs.
    pub modified: Vec<String>,
}

impl PathDiff {
    /// Returns `true` if there are no differences.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// Builds the nested tree graph for `files` (a flat `path → blob id` map) and
/// returns the root tree's [`ObjectId`]. Paths use `/` as the separator.
pub async fn build_tree(
    objects: &ObjectStore,
    files: &BTreeMap<String, ObjectId>,
) -> Result<ObjectId> {
    let mut root = DirNode::default();
    for (path, id) in files {
        root.insert(path, *id);
    }
    store_dir(objects, &root).await
}

#[derive(Default)]
struct DirNode {
    files: BTreeMap<String, ObjectId>,
    dirs: BTreeMap<String, DirNode>,
}

impl DirNode {
    fn insert(&mut self, path: &str, id: ObjectId) {
        match path.split_once('/') {
            Some((head, rest)) => self.dirs.entry(head.to_string()).or_default().insert(rest, id),
            None => {
                self.files.insert(path.to_string(), id);
            }
        }
    }
}

async fn store_dir(objects: &ObjectStore, node: &DirNode) -> Result<ObjectId> {
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for (name, id) in &node.files {
        entries.insert(name.clone(), TreeEntry { id: *id, kind: EntryKind::Blob });
    }
    for (name, child) in &node.dirs {
        let child_id = Box::pin(store_dir(objects, child)).await?;
        entries.insert(name.clone(), TreeEntry { id: child_id, kind: EntryKind::Tree });
    }
    objects.put_tree(entries).await
}

/// Returns the flat `path → blob id` map of a snapshot's tree.
pub async fn snapshot_paths(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
) -> Result<BTreeMap<String, ObjectId>> {
    let snap = match objects.get(&snapshot_id).await? {
        Some(Object::Snapshot(s)) => s,
        _ => return Err(Error::Storage(format!("not a snapshot: {snapshot_id}"))),
    };
    let mut out = BTreeMap::new();
    walk_tree(objects, snap.root, String::new(), &mut out).await?;
    Ok(out)
}

async fn walk_tree(
    objects: &ObjectStore,
    tree_id: ObjectId,
    prefix: String,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    let mut stack = vec![(tree_id, prefix)];
    while let Some((tid, pfx)) = stack.pop() {
        let tree = match objects.get(&tid).await? {
            Some(Object::Tree(t)) => t,
            _ => return Err(Error::Storage(format!("not a tree: {tid}"))),
        };
        let Tree { entries } = tree;
        for (name, entry) in entries {
            let path = if pfx.is_empty() { name } else { format!("{pfx}/{name}") };
            match entry.kind {
                EntryKind::Blob => {
                    out.insert(path, entry.id);
                }
                EntryKind::Tree => stack.push((entry.id, path)),
            }
        }
    }
    Ok(())
}

/// Computes added/removed/modified paths going from `base` to `target`.
pub fn diff_paths(
    base: &BTreeMap<String, ObjectId>,
    target: &BTreeMap<String, ObjectId>,
) -> PathDiff {
    let mut d = PathDiff::default();
    for (path, id) in target {
        match base.get(path) {
            None => d.added.push(path.clone()),
            Some(base_id) if base_id != id => d.modified.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in base.keys() {
        if !target.contains_key(path) {
            d.removed.push(path.clone());
        }
    }
    d
}

/// A pure in-memory working tree over a [`Repository`].
///
/// Files live in RAM as byte buffers; nothing is written to the filesystem.
/// [`commit`](Self::commit) stores blobs and a snapshot in the repository's
/// object store and returns the new snapshot id.
// bole-1kz
/// A mutable, path-keyed byte store that can be diffed against a base snapshot
/// and committed to the object graph.
///
/// Two implementations exist: [`EphemeralWorkspace`] (in-memory) and
/// [`DiskWorkspace`] (filesystem-backed). The work tree is not a second model —
/// it is one model with a filesystem backing. All methods are async so the disk
/// implementation can cross the I/O boundary uniformly; `#[async_trait]` keeps
/// `dyn Workspace` usable.
#[async_trait::async_trait]
pub trait Workspace {
    /// The snapshot this workspace considers its starting point; the sole parent
    /// when [`commit`](Self::commit) creates a new snapshot.
    fn base(&self) -> Option<ObjectId>;

    /// The bytes at `path`, or `None` if absent (owned copy).
    async fn read(&self, path: &str) -> Result<Option<Bytes>>;

    /// Creates or overwrites `path` with `bytes`.
    async fn write(&mut self, path: &str, bytes: Bytes) -> Result<()>;

    /// Deletes `path`. Returns `true` if it existed.
    async fn remove(&mut self, path: &str) -> Result<bool>;

    /// All paths in the workspace, in sorted order.
    async fn paths(&self) -> Result<Vec<String>>;

    /// Diffs the current workspace state against the base snapshot. Stores blobs
    /// in the object store (content-addressed, idempotent).
    async fn diff(&self) -> Result<PathDiff>;

    /// Commits the current workspace state as a new snapshot whose parent is
    /// [`base`](Self::base), advances `base` to it, and returns its id. Does not
    /// advance any timeline.
    async fn commit(&mut self, author: &str, message: &str, created_at: u64) -> Result<ObjectId>;
}

pub struct EphemeralWorkspace<'a> {
    repo: &'a Repository,
    base: Option<ObjectId>,
    files: BTreeMap<String, Bytes>,
}

// bole-1kz
/// Thin async wrapper over the inherent (synchronous) methods; additive, so no
/// existing caller changes. Fully-qualified `EphemeralWorkspace::method` calls
/// dodge trait/inherent name collisions (and thus infinite recursion).
#[async_trait::async_trait]
impl Workspace for EphemeralWorkspace<'_> {
    fn base(&self) -> Option<ObjectId> {
        EphemeralWorkspace::base(self)
    }

    async fn read(&self, path: &str) -> Result<Option<Bytes>> {
        Ok(EphemeralWorkspace::read(self, path).map(Bytes::copy_from_slice))
    }

    async fn write(&mut self, path: &str, bytes: Bytes) -> Result<()> {
        EphemeralWorkspace::write(self, path, bytes);
        Ok(())
    }

    async fn remove(&mut self, path: &str) -> Result<bool> {
        Ok(EphemeralWorkspace::remove(self, path))
    }

    async fn paths(&self) -> Result<Vec<String>> {
        Ok(EphemeralWorkspace::paths(self).map(str::to_owned).collect())
    }

    async fn diff(&self) -> Result<PathDiff> {
        EphemeralWorkspace::diff(self).await
    }

    async fn commit(&mut self, author: &str, message: &str, created_at: u64) -> Result<ObjectId> {
        EphemeralWorkspace::commit(self, author, message, created_at).await
    }
}

impl<'a> EphemeralWorkspace<'a> {
    /// Creates an empty workspace (no base snapshot).
    pub fn new(repo: &'a Repository) -> Self {
        Self { repo, base: None, files: BTreeMap::new() }
    }

    /// Creates a workspace seeded from `snapshot`'s files; the snapshot becomes
    /// the commit parent.
    pub async fn from_snapshot(repo: &'a Repository, snapshot: ObjectId) -> Result<Self> {
        let paths = snapshot_paths(&repo.objects, snapshot).await?;
        let mut files = BTreeMap::new();
        for (path, blob_id) in paths {
            match repo.objects.get(&blob_id).await? {
                Some(Object::Blob(b)) => {
                    files.insert(path, b.data);
                }
                _ => return Err(Error::Storage(format!("not a blob: {blob_id}"))),
            }
        }
        Ok(Self { repo, base: Some(snapshot), files })
    }

    /// The snapshot this workspace was seeded from (the next commit's parent).
    pub fn base(&self) -> Option<ObjectId> {
        self.base
    }

    /// Reads the bytes of `path`, if present.
    pub fn read(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|b| b.as_ref())
    }

    /// Writes `bytes` to `path`, creating or overwriting it.
    pub fn write(&mut self, path: impl Into<String>, bytes: impl Into<Bytes>) {
        self.files.insert(path.into(), bytes.into());
    }

    /// Removes `path`, returning `true` if it existed.
    pub fn remove(&mut self, path: &str) -> bool {
        self.files.remove(path).is_some()
    }

    /// Iterates the paths currently in the workspace, in sorted order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(|s| s.as_str())
    }

    /// Stores the current files as blobs and returns the flat `path → blob id`
    /// map. (Content-addressed: storing is idempotent.)
    async fn current_paths(&self) -> Result<BTreeMap<String, ObjectId>> {
        let mut out = BTreeMap::new();
        for (path, bytes) in &self.files {
            let id = self.repo.objects.put_blob(bytes.clone()).await?;
            out.insert(path.clone(), id);
        }
        Ok(out)
    }

    /// Diffs the current in-memory files against the base snapshot.
    ///
    /// Note: this stores the current files' blobs in the object store
    /// (content-addressed, so it is idempotent and cheap).
    pub async fn diff(&self) -> Result<PathDiff> {
        let base = match self.base {
            Some(b) => snapshot_paths(&self.repo.objects, b).await?,
            None => BTreeMap::new(),
        };
        let target = self.current_paths().await?;
        Ok(diff_paths(&base, &target))
    }

    /// Commits the current files as a new snapshot whose parent is the current
    /// base, advances the workspace's base to it, and returns its id.
    ///
    /// This does not move any timeline. To publish the snapshot on a timeline,
    /// call [`Repository::advance_timeline`] with it.
    pub async fn commit(
        &mut self,
        author: impl Into<String>,
        message: impl Into<String>,
        created_at: u64,
    ) -> Result<ObjectId> {
        let files = self.current_paths().await?;
        let root = build_tree(&self.repo.objects, &files).await?;
        let parents = self.base.map(|b| vec![b]).unwrap_or_default();
        let id = self
            .repo
            .objects
            .put_snapshot(Snapshot {
                root,
                parents,
                author: author.into(),
                created_at,
                message: message.into(),
            })
            .await?;
        self.base = Some(id);
        Ok(id)
    }
}

// bole-1kz
/// A filesystem-backed [`Workspace`]. Lazy: files are read from disk on demand,
/// and the directory walk is deferred to `paths`/`diff`/`commit`. Writes are
/// write-through (they hit the filesystem immediately). Construction is cheap
/// and reads no files.
pub struct DiskWorkspace<'a> {
    repo: &'a Repository,
    root: PathBuf,
    base: Option<ObjectId>,
}

impl<'a> DiskWorkspace<'a> {
    /// Creates a workspace rooted at `root` with no base snapshot.
    pub fn new(repo: &'a Repository, root: impl Into<PathBuf>) -> Self {
        Self { repo, root: root.into(), base: None }
    }

    /// Creates a workspace rooted at `root` with `snapshot` as the base.
    pub fn bound(repo: &'a Repository, root: impl Into<PathBuf>, snapshot: ObjectId) -> Self {
        Self { repo, root: root.into(), base: Some(snapshot) }
    }

    /// Absolute on-disk path for a workspace-relative `path`.
    fn abs(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    // bole-phxz
    /// Builds the gitignore matcher from the root `.boleignore`. A missing file
    /// yields an empty matcher (nothing ignored); a malformed pattern is dropped
    /// rather than failing the walk.
    fn ignore_matcher(&self) -> Gitignore {
        let mut builder = GitignoreBuilder::new(&self.root);
        // `add` returns `Some(err)` when the file is missing or a line fails to
        // parse; in both cases we proceed with whatever parsed successfully.
        let _ = builder.add(self.root.join(IGNORE_FILE));
        builder.build().unwrap_or_else(|_| Gitignore::empty())
    }

    /// The single disk-walk implementation: stores each non-excluded regular
    /// file as a blob and returns `path → blob id` with forward-slash paths
    /// relative to `root`. Skips `.bole` (dir or pointer file) and all symlinks.
    async fn collect(&self) -> Result<BTreeMap<String, ObjectId>> {
        let mut out = BTreeMap::new();
        // bole-phxz: gitignore-compatible matcher from the root `.boleignore`
        // (empty and harmless if the file is absent or unparseable).
        let ignore = self.ignore_matcher();
        // Iterative walk to avoid async recursion.
        let mut stack = vec![self.root.clone()];
        while let Some(current) = stack.pop() {
            let mut rd = tokio::fs::read_dir(&current).await?;
            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                // OQ4: skip all symlinks (never follow out of the workspace).
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                        continue;
                    }
                    // bole-phxz: prune ignored directories (e.g. `target/`) so
                    // the whole subtree is skipped without descending.
                    if ignore.matched(&path, true).is_ignore() {
                        continue;
                    }
                    stack.push(path);
                } else if file_type.is_file() {
                    // A linked worktree's `.bole` is a pointer file, not content.
                    if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                        continue;
                    }
                    // bole-phxz: skip ignored files.
                    if ignore.matched(&path, false).is_ignore() {
                        continue;
                    }
                    let bytes = tokio::fs::read(&path).await?;
                    let id = self.repo.objects.put_blob(Bytes::from(bytes)).await?;
                    let rel = path
                        .strip_prefix(&self.root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, id);
                }
            }
        }
        Ok(out)
    }

    // bole-wphx
    /// Read-only scan for `bole doctor`: walk the working tree and return each
    /// file whose content looks like a bare account seed, paired with whether
    /// `.boleignore` already excludes it. Stores nothing. Ignored directories
    /// are pruned (their contents can't be committed anyway); a seed file that
    /// is NOT ignored is the real risk — it would be captured by the next
    /// snapshot.
    pub async fn scan_seed_files(&self) -> Result<Vec<(String, bool)>> {
        let ignore = self.ignore_matcher();
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(current) = stack.pop() {
            let mut rd = tokio::fs::read_dir(&current).await?;
            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                let ft = entry.file_type().await?;
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                        continue;
                    }
                    if ignore.matched(&path, true).is_ignore() {
                        continue; // pruned: can't be committed
                    }
                    stack.push(path);
                } else if ft.is_file() {
                    if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                        continue;
                    }
                    // Only read small files (a seed file is tiny) to stay cheap.
                    let meta = entry.metadata().await?;
                    if meta.len() > 4096 {
                        continue;
                    }
                    let bytes = tokio::fs::read(&path).await?;
                    if crate::looks_like_private_seed(&bytes) {
                        let ignored = ignore.matched(&path, false).is_ignore();
                        let rel = path.strip_prefix(&self.root).unwrap().to_string_lossy().replace('\\', "/");
                        out.push((rel, ignored));
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

// bole-1kz
#[async_trait::async_trait]
impl Workspace for DiskWorkspace<'_> {
    fn base(&self) -> Option<ObjectId> {
        self.base
    }

    async fn read(&self, path: &str) -> Result<Option<Bytes>> {
        match tokio::fs::read(self.abs(path)).await {
            Ok(b) => Ok(Some(Bytes::from(b))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    async fn write(&mut self, path: &str, bytes: Bytes) -> Result<()> {
        let abs = self.abs(path);
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&abs, &bytes).await?;
        Ok(())
    }

    async fn remove(&mut self, path: &str) -> Result<bool> {
        match tokio::fs::remove_file(self.abs(path)).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::Io(e)),
        }
    }

    async fn paths(&self) -> Result<Vec<String>> {
        Ok(self.collect().await?.into_keys().collect())
    }

    async fn diff(&self) -> Result<PathDiff> {
        let base = match self.base {
            Some(b) => snapshot_paths(&self.repo.objects, b).await?,
            None => BTreeMap::new(),
        };
        let target = self.collect().await?;
        Ok(diff_paths(&base, &target))
    }

    async fn commit(&mut self, author: &str, message: &str, created_at: u64) -> Result<ObjectId> {
        let files = self.collect().await?;
        let root = build_tree(&self.repo.objects, &files).await?;
        let parents = self.base.map(|b| vec![b]).unwrap_or_default();
        let id = self
            .repo
            .objects
            .put_snapshot(Snapshot {
                root,
                parents,
                author: author.into(),
                created_at,
                message: message.into(),
            })
            .await?;
        self.base = Some(id);
        Ok(id)
    }
}

impl Repository {
    // bole-uxt
    /// Opens an empty in-memory workspace over this repository.
    pub fn ephemeral_workspace(&self) -> EphemeralWorkspace<'_> {
        EphemeralWorkspace::new(self)
    }

    // bole-uxt
    /// Opens an in-memory workspace seeded from `snapshot`.
    pub async fn ephemeral_workspace_from(&self, snapshot: ObjectId) -> Result<EphemeralWorkspace<'_>> {
        EphemeralWorkspace::from_snapshot(self, snapshot).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_commit_then_seed_roundtrip() {
        let repo = Repository::memory();
        let mut ws = repo.ephemeral_workspace();
        assert_eq!(ws.base(), None);
        ws.write("src/main.rs", &b"fn main() {}"[..]);
        ws.write("README.md", &b"hi"[..]);
        let snap = ws.commit("agent", "init", 0).await.unwrap();
        assert_eq!(ws.base(), Some(snap));

        // Seeding a fresh workspace from the snapshot recovers the files.
        let ws2 = repo.ephemeral_workspace_from(snap).await.unwrap();
        assert_eq!(ws2.read("src/main.rs"), Some(&b"fn main() {}"[..]));
        assert_eq!(ws2.read("README.md"), Some(&b"hi"[..]));
        assert_eq!(ws2.paths().count(), 2);
    }

    #[tokio::test]
    async fn diff_reports_add_modify_remove() {
        let repo = Repository::memory();
        let mut ws = repo.ephemeral_workspace();
        ws.write("a.txt", &b"1"[..]);
        ws.write("b.txt", &b"keep"[..]);
        let snap = ws.commit("t", "base", 0).await.unwrap();

        let mut ws2 = repo.ephemeral_workspace_from(snap).await.unwrap();
        ws2.write("c.txt", &b"new"[..]);     // added
        ws2.write("a.txt", &b"2"[..]);       // modified
        assert!(ws2.remove("b.txt"));        // removed

        let d = ws2.diff().await.unwrap();
        assert_eq!(d.added, vec!["c.txt"]);
        assert_eq!(d.modified, vec!["a.txt"]);
        assert_eq!(d.removed, vec!["b.txt"]);
        assert!(!d.is_empty());
    }

    #[tokio::test]
    async fn commit_chains_parents() {
        let repo = Repository::memory();
        let mut ws = repo.ephemeral_workspace();
        ws.write("a.txt", &b"1"[..]);
        let s1 = ws.commit("t", "one", 0).await.unwrap();
        ws.write("a.txt", &b"2"[..]);
        let s2 = ws.commit("t", "two", 1).await.unwrap();

        let snap2 = match repo.objects.get(&s2).await.unwrap().unwrap() {
            Object::Snapshot(s) => s,
            _ => panic!("expected snapshot"),
        };
        assert_eq!(snap2.parents, vec![s1]);
    }

    #[tokio::test]
    async fn clean_diff_is_empty() {
        let repo = Repository::memory();
        let mut ws = repo.ephemeral_workspace();
        ws.write("a.txt", &b"x"[..]);
        let snap = ws.commit("t", "c", 0).await.unwrap();
        let ws2 = repo.ephemeral_workspace_from(snap).await.unwrap();
        assert!(ws2.diff().await.unwrap().is_empty());
    }

    // bole-1kz
    #[tokio::test]
    async fn ephemeral_workspace_through_trait_object() {
        let repo = Repository::memory();
        let mut ws = repo.ephemeral_workspace();
        {
            // Exercise the whole surface through &mut dyn Workspace.
            let w: &mut dyn Workspace = &mut ws;
            assert_eq!(w.base(), None);
            w.write("a.txt", Bytes::from_static(b"hi")).await.unwrap();
            assert_eq!(w.read("a.txt").await.unwrap(), Some(Bytes::from_static(b"hi")));
            assert_eq!(w.read("missing").await.unwrap(), None);
            assert_eq!(w.paths().await.unwrap(), vec!["a.txt".to_string()]);
            assert!(w.remove("a.txt").await.unwrap());
            assert!(!w.remove("a.txt").await.unwrap());
            w.write("b.txt", Bytes::from_static(b"x")).await.unwrap();
            let id = w.commit("t", "m", 0).await.unwrap();
            assert_eq!(w.base(), Some(id));
        }
        // Trait commit advanced base; the snapshot has the file.
        let base = ws.base().unwrap();
        let paths = snapshot_paths(&repo.objects, base).await.unwrap();
        assert!(paths.contains_key("b.txt"));
    }

    // bole-1kz
    #[tokio::test]
    async fn disk_workspace_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let mut ws = DiskWorkspace::new(&repo, dir.path());
        ws.write("sub/a.txt", Bytes::from_static(b"hi")).await.unwrap();
        // Write-through: the file is on disk and readable immediately.
        assert_eq!(ws.read("sub/a.txt").await.unwrap(), Some(Bytes::from_static(b"hi")));
        assert_eq!(ws.read("nope").await.unwrap(), None);
        let id = ws.commit("t", "m", 0).await.unwrap();
        let paths = snapshot_paths(&repo.objects, id).await.unwrap();
        assert!(paths.contains_key("sub/a.txt"));
        assert_eq!(ws.base(), Some(id));
    }

    // bole-1kz
    #[tokio::test]
    async fn disk_workspace_diff_add_modify_remove() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let mut ws = DiskWorkspace::new(&repo, dir.path());
        ws.write("a.txt", Bytes::from_static(b"1")).await.unwrap();
        ws.write("b.txt", Bytes::from_static(b"keep")).await.unwrap();
        let base = ws.commit("t", "base", 0).await.unwrap();

        // Seed a bound workspace, then mutate the disk directly.
        let mut ws2 = DiskWorkspace::bound(&repo, dir.path(), base);
        ws2.write("c.txt", Bytes::from_static(b"new")).await.unwrap(); // added
        ws2.write("a.txt", Bytes::from_static(b"2")).await.unwrap();   // modified
        assert!(ws2.remove("b.txt").await.unwrap());                    // removed
        assert!(!ws2.remove("b.txt").await.unwrap());                   // already gone

        let d = ws2.diff().await.unwrap();
        assert_eq!(d.added, vec!["c.txt"]);
        assert_eq!(d.modified, vec!["a.txt"]);
        assert_eq!(d.removed, vec!["b.txt"]);
    }

    // bole-1kz
    #[tokio::test]
    async fn disk_workspace_excludes_bole_dir_and_file() {
        let repo = Repository::memory();

        // Primary worktree: `.bole` is a directory with content.
        let dir = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(dir.path().join(".bole")).await.unwrap();
        tokio::fs::write(dir.path().join(".bole/store"), b"x").await.unwrap();
        tokio::fs::write(dir.path().join("keep.txt"), b"y").await.unwrap();
        let ws = DiskWorkspace::new(&repo, dir.path());
        assert_eq!(ws.paths().await.unwrap(), vec!["keep.txt".to_string()]);

        // Linked worktree: `.bole` is a pointer file, not content.
        let dir2 = tempfile::TempDir::new().unwrap();
        tokio::fs::write(dir2.path().join(".bole"), b"{\"store\":\"..\"}").await.unwrap();
        tokio::fs::write(dir2.path().join("k.txt"), b"z").await.unwrap();
        let ws2 = DiskWorkspace::new(&repo, dir2.path());
        assert_eq!(ws2.paths().await.unwrap(), vec!["k.txt".to_string()]);
    }

    // bole-phxz
    #[tokio::test]
    async fn disk_workspace_honors_boleignore() {
        let repo = Repository::memory();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        tokio::fs::write(root.join(".boleignore"), b"*.log\ntarget/\n!keep.log\n")
            .await
            .unwrap();
        tokio::fs::write(root.join("app.log"), b"x").await.unwrap(); // ignored by *.log
        tokio::fs::write(root.join("keep.log"), b"x").await.unwrap(); // re-included by !keep.log
        tokio::fs::write(root.join("main.rs"), b"x").await.unwrap(); // kept
        tokio::fs::create_dir(root.join("target")).await.unwrap();
        tokio::fs::write(root.join("target/out.bin"), b"x").await.unwrap(); // subtree pruned
        tokio::fs::create_dir(root.join("src")).await.unwrap();
        tokio::fs::write(root.join("src/lib.rs"), b"x").await.unwrap(); // kept

        let ws = DiskWorkspace::new(&repo, root);
        let paths = ws.paths().await.unwrap();

        // The ignore file itself is a tracked file, not excluded.
        assert!(paths.contains(&".boleignore".to_string()));
        assert!(paths.contains(&"keep.log".to_string()));
        assert!(paths.contains(&"main.rs".to_string()));
        assert!(paths.contains(&"src/lib.rs".to_string()));
        assert!(!paths.contains(&"app.log".to_string()));
        assert!(!paths.iter().any(|p| p.starts_with("target/")));
    }

    // bole-phxz
    #[tokio::test]
    async fn disk_workspace_no_boleignore_ignores_nothing() {
        let repo = Repository::memory();
        let dir = tempfile::TempDir::new().unwrap();
        tokio::fs::write(dir.path().join("app.log"), b"x").await.unwrap();
        let ws = DiskWorkspace::new(&repo, dir.path());
        assert_eq!(ws.paths().await.unwrap(), vec!["app.log".to_string()]);
    }

    // bole-1kz
    #[tokio::test]
    async fn disk_workspace_commit_chains_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let mut ws = DiskWorkspace::new(&repo, dir.path());
        ws.write("a.txt", Bytes::from_static(b"1")).await.unwrap();
        let s1 = ws.commit("t", "one", 0).await.unwrap();
        ws.write("a.txt", Bytes::from_static(b"2")).await.unwrap();
        let s2 = ws.commit("t", "two", 1).await.unwrap();

        let snap2 = match repo.objects.get(&s2).await.unwrap().unwrap() {
            Object::Snapshot(s) => s,
            _ => panic!("expected snapshot"),
        };
        assert_eq!(snap2.parents, vec![s1]);
    }

    // bole-1kz
    #[tokio::test]
    async fn disk_workspace_trait_object_dispatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let mut dw = DiskWorkspace::new(&repo, dir.path());
        let w: &mut dyn Workspace = &mut dw;
        w.write("f.txt", Bytes::from_static(b"q")).await.unwrap();
        let id = w.commit("t", "m", 0).await.unwrap();
        assert!(snapshot_paths(&repo.objects, id).await.unwrap().contains_key("f.txt"));
    }
}
