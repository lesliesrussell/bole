// bole-hrk
//! Registry of linked worktrees, persisted at `<store>/worktrees.json`.
//!
//! Each linked worktree has an id; its metadata (binding) lives under
//! `<store>/worktrees/<id>/` and its on-disk location is recorded here so
//! `workspace list` / `remove` can find it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// One registered linked worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Absolute path to the worktree directory.
    pub path: String,
}

/// The full registry: worktree id -> entry.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub worktrees: BTreeMap<String, Entry>,
}

fn registry_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join("worktrees.json")
}

/// Directory holding a worktree's metadata (`state.json`).
pub fn meta_dir(repo_dir: &Path, id: &str) -> PathBuf {
    repo_dir.join("worktrees").join(id)
}

/// Loads the registry, returning an empty one if absent.
pub fn load(repo_dir: &Path) -> Result<Registry> {
    let p = registry_path(repo_dir);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

/// Persists the registry.
pub fn save(repo_dir: &Path, registry: &Registry) -> Result<()> {
    let p = registry_path(repo_dir);
    let bytes = serde_json::to_vec_pretty(registry)?;
    std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))
}

/// Derives a unique worktree id from a directory path, based on its file name.
pub fn allocate_id(registry: &Registry, path: &Path) -> String {
    let base: String = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "worktree".to_string())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let base = if base.is_empty() { "worktree".to_string() } else { base };
    if !registry.worktrees.contains_key(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !registry.worktrees.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}
