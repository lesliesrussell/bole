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

use bytes::Bytes;

use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId, Snapshot, Tree, TreeEntry};
use crate::repo::Repository;
use crate::store::ObjectStore;

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
}
