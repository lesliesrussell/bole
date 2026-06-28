// bole-gvy
//! Converting between the on-disk work tree and the object store's tree graph.
//!
//! A snapshot's tree is a pure file hierarchy (`EntryKind` is only `Blob` or
//! `Tree`), so the work tree maps onto it directly: every file becomes a blob,
//! every directory a tree. The `.bole/` repository directory is never included.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use bole::{EntryKind, Object, ObjectId, ObjectStore, Tree, TreeEntry};
use bytes::Bytes;

use crate::context::REPO_DIR;

/// Walks `work_dir`, storing each file as a blob, and returns a map from
/// forward-slash relative path to the blob's `ObjectId`. Skips `.bole/`.
pub async fn collect_blobs(
    objects: &ObjectStore,
    work_dir: &Path,
) -> Result<BTreeMap<String, ObjectId>> {
    let mut out = BTreeMap::new();
    collect_dir(objects, work_dir, work_dir, &mut out).await?;
    Ok(out)
}

async fn collect_dir(
    objects: &ObjectStore,
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    // Iterative directory walk to avoid async recursion.
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&current)
            .await
            .with_context(|| format!("reading {}", current.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("reading {}", path.display()))?;
                let id = objects.put_blob(Bytes::from(bytes)).await?;
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, id);
            }
        }
    }
    Ok(())
}

/// Builds the nested tree graph for `blobs` (path -> blob id) and returns the
/// root tree's `ObjectId`.
pub async fn build_root_tree(
    objects: &ObjectStore,
    blobs: &BTreeMap<String, ObjectId>,
) -> Result<ObjectId> {
    // Assemble an in-memory directory trie, then store it bottom-up.
    let mut root = DirNode::default();
    for (path, id) in blobs {
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
    // Post-order traversal without async recursion: resolve subtree ids first.
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for (name, id) in &node.files {
        entries.insert(name.clone(), TreeEntry { id: *id, kind: EntryKind::Blob });
    }
    for (name, child) in &node.dirs {
        let child_id = Box::pin(store_dir(objects, child)).await?;
        entries.insert(name.clone(), TreeEntry { id: child_id, kind: EntryKind::Tree });
    }
    Ok(objects.put_tree(entries).await?)
}

/// Returns the flat path -> blob id map of a snapshot's tree.
pub async fn snapshot_blobs(
    objects: &ObjectStore,
    snapshot_id: ObjectId,
) -> Result<BTreeMap<String, ObjectId>> {
    let snap = match objects.get(&snapshot_id).await? {
        Some(Object::Snapshot(s)) => s,
        _ => anyhow::bail!("not a snapshot: {snapshot_id}"),
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
            _ => anyhow::bail!("not a tree: {tid}"),
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

/// A path-level diff between two flat blob maps.
pub struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
}

/// Computes added/removed/modified paths going from `base` to `target`.
pub fn diff(base: &BTreeMap<String, ObjectId>, target: &BTreeMap<String, ObjectId>) -> Diff {
    let mut d = Diff { added: Vec::new(), removed: Vec::new(), modified: Vec::new() };
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
