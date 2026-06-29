// bole-gvy
//! Converting between the on-disk work tree and the object store's tree graph.
//!
//! The tree-building, snapshot-reading, and diff logic now lives in the `bole`
//! library ([`bole::build_tree`], [`bole::snapshot_paths`], [`bole::diff_paths`])
//! so the CLI and the library's in-memory [`bole::EphemeralWorkspace`] share one
//! implementation. This module keeps only the disk-specific walk and re-exports
//! the library primitives under the names the CLI commands use.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use bole::{ObjectId, ObjectStore};
use bytes::Bytes;

use crate::context::REPO_DIR;

// bole-uxt: shared core lives in the library; re-export under the CLI's names.
pub use bole::{build_tree as build_root_tree, diff_paths as diff, snapshot_paths as snapshot_blobs};

/// Walks `work_dir`, storing each file as a blob, and returns a map from
/// forward-slash relative path to the blob's `ObjectId`. Skips `.bole`.
pub async fn collect_blobs(
    objects: &ObjectStore,
    work_dir: &Path,
) -> Result<BTreeMap<String, ObjectId>> {
    let mut out = BTreeMap::new();
    // Iterative directory walk to avoid async recursion.
    let mut stack = vec![work_dir.to_path_buf()];
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
                // A linked worktree's `.bole` is a pointer file, not content.
                if path.file_name().map(|n| n == REPO_DIR).unwrap_or(false) {
                    continue;
                }
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("reading {}", path.display()))?;
                let id = objects.put_blob(Bytes::from(bytes)).await?;
                let rel = path
                    .strip_prefix(work_dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, id);
            }
        }
    }
    Ok(out)
}
