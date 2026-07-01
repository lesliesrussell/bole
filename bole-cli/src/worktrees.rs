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

// bole-3hj
use crate::context::{Pointer, REPO_DIR};

// bole-3hj
/// The consistency of a registered linked worktree vs its on-disk pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Directory, pointer, id, and store all consistent.
    Ok,
    /// `entry.path` is not an existing directory.
    MissingDirectory,
    /// The directory exists but `<path>/.bole` is absent or not a file.
    MissingPointer,
    /// `.bole` exists but is not readable / not a valid `Pointer`.
    BadPointer(String),
    /// The pointer's id does not match the registry key.
    WrongId { found: String },
    /// The pointer's store path differs from the current store (store moved) but
    /// the id matches — recoverable by `repair` (R1).
    WrongStore { found: String },
}

impl Status {
    /// A stable machine string for JSON / scripting.
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::MissingDirectory => "missing-directory",
            Status::MissingPointer => "missing-pointer",
            Status::BadPointer(_) => "bad-pointer",
            Status::WrongId { .. } => "wrong-id",
            Status::WrongStore { .. } => "wrong-store",
        }
    }
    pub fn is_ok(&self) -> bool {
        matches!(self, Status::Ok)
    }
    /// Prunable by default (disconnected from any recoverable state).
    pub fn is_prunable(&self) -> bool {
        matches!(
            self,
            Status::MissingDirectory
                | Status::MissingPointer
                | Status::BadPointer(_)
                | Status::WrongId { .. }
        )
    }
    /// Recoverable by `repair` (store moved, id still matches).
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Status::WrongStore { .. })
    }
}

// bole-3hj
/// Pure classification of one registry entry against the filesystem. Reads the
/// pointer file directly (before any RepoContext is established).
pub fn classify(repo_dir: &Path, id: &str, entry: &Entry) -> Status {
    let dir = Path::new(&entry.path);
    if !dir.is_dir() {
        return Status::MissingDirectory;
    }
    let ptr_path = dir.join(REPO_DIR);
    if !ptr_path.is_file() {
        return Status::MissingPointer;
    }
    let bytes = match std::fs::read(&ptr_path) {
        Ok(b) => b,
        Err(e) => return Status::BadPointer(e.to_string()),
    };
    let ptr: Pointer = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(e) => return Status::BadPointer(e.to_string()),
    };
    if ptr.id != id {
        return Status::WrongId { found: ptr.id };
    }
    if !same_path(&ptr.store, repo_dir) {
        return Status::WrongStore { found: ptr.store };
    }
    Status::Ok
}

// bole-3hj
/// Path equality up to canonicalization (falls back to literal compare).
pub fn same_path(a: &str, b: &Path) -> bool {
    match (std::fs::canonicalize(a).ok(), std::fs::canonicalize(b).ok()) {
        (Some(x), Some(y)) => x == y,
        _ => Path::new(a) == b,
    }
}

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
