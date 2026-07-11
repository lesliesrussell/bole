// bole-aqk
//! Repository discovery and CLI-local state.
//!
//! A bole working tree is any directory containing a `.bole` entry:
//!
//! - a `.bole/` **directory** is a *primary* repository — it holds the
//!   library's object/ref/acl stores plus `cli-state.json` (the bound timeline
//!   and actor for the primary worktree).
//! - a `.bole` **file** is a *linked* worktree (see [`bole-hrk`]): it points at
//!   a primary store and carries its own binding under
//!   `<store>/worktrees/<id>/state.json`, so many directories can share one
//!   store while each tracks a different timeline.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use bole::Repository;
use serde::{Deserialize, Serialize};

/// Name of the repository directory (primary) or pointer file (linked).
pub const REPO_DIR: &str = ".bole";

/// CLI-only state, persisted per worktree.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CliState {
    /// Timeline the working tree is currently bound to, if any.
    pub current_timeline: Option<String>,
    /// Actor whose credentials the CLI acts as, if any.
    pub current_actor: Option<String>,
}

// bole-hrk
/// Contents of a linked worktree's `.bole` pointer file.
#[derive(Debug, Serialize, Deserialize)]
pub struct Pointer {
    /// Absolute path to the primary `.bole/` store directory.
    pub store: String,
    /// This worktree's id (its metadata lives at `<store>/worktrees/<id>/`).
    pub id: String,
}

/// An opened repository plus the paths needed to read and write CLI state.
pub struct RepoContext {
    /// The shared `.bole/` store directory.
    pub repo_dir: PathBuf,
    /// The working tree root for this context.
    pub work_dir: PathBuf,
    /// The opened library repository.
    pub repo: Repository,
    // bole-hrk
    /// Where this worktree's binding is stored (primary or linked).
    state_path: PathBuf,
    /// `None` for the primary worktree, `Some(id)` for a linked one. Recorded as
    /// part of the worktree model; not read on the current code paths.
    #[allow(dead_code)]
    pub worktree_id: Option<String>,
}

impl RepoContext {
    /// Searches from `start` upward for a `.bole` entry and opens it, handling
    /// both primary repositories (`.bole/` dir) and linked worktrees
    /// (`.bole` file).
    pub async fn discover(start: &Path) -> Result<Self> {
        let mut dir = Some(start);
        while let Some(d) = dir {
            let candidate = d.join(REPO_DIR);
            if candidate.is_dir() {
                // Primary repository.
                let mut repo = Repository::disk(&candidate)
                    .await
                    .with_context(|| format!("opening repository at {}", candidate.display()))?;
                // bole-ehx: register persisted policy hooks so advance/merge enforce them.
                for spec in crate::commands::policy::load_hooks(&candidate)? {
                    repo.register_hook(spec);
                }
                // bole-eean: install the audit sink if $BOLE_AUDIT_LOG is set.
                let repo = crate::audit::install(repo)?;
                let state_path = candidate.join("cli-state.json");
                return Ok(Self {
                    repo_dir: candidate,
                    work_dir: d.to_path_buf(),
                    repo,
                    state_path,
                    worktree_id: None,
                });
            }
            // bole-hrk
            if candidate.is_file() {
                // Linked worktree: follow the pointer to the shared store.
                let bytes = std::fs::read(&candidate)
                    .with_context(|| format!("reading worktree pointer {}", candidate.display()))?;
                let ptr: Pointer = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing worktree pointer {}", candidate.display()))?;
                // bole-1hd: a crafted pointer's id is joined into a store path;
                // reject anything that isn't a safe single path component so it
                // cannot escape <store>/worktrees.
                crate::worktrees::validate_id(&ptr.id)
                    .with_context(|| format!("in worktree pointer {}", candidate.display()))?;
                let store = PathBuf::from(&ptr.store);
                let mut repo = Repository::disk(&store)
                    .await
                    .with_context(|| format!("opening shared store at {}", store.display()))?;
                // bole-ehx: register persisted policy hooks so advance/merge enforce them.
                for spec in crate::commands::policy::load_hooks(&store)? {
                    repo.register_hook(spec);
                }
                // bole-eean: install the audit sink if $BOLE_AUDIT_LOG is set.
                let repo = crate::audit::install(repo)?;
                let state_path = store.join("worktrees").join(&ptr.id).join("state.json");
                return Ok(Self {
                    repo_dir: store,
                    work_dir: d.to_path_buf(),
                    repo,
                    state_path,
                    worktree_id: Some(ptr.id),
                });
            }
            dir = d.parent();
        }
        Err(anyhow!(
            "not a bole repository (no {REPO_DIR} found in {} or any parent)",
            start.display()
        ))
    }

    /// Reads this worktree's CLI state, returning defaults if absent.
    pub fn load_state(&self) -> Result<CliState> {
        match std::fs::read(&self.state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", self.state_path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CliState::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", self.state_path.display())),
        }
    }

    /// Writes this worktree's CLI state.
    pub fn save_state(&self, state: &CliState) -> Result<()> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        std::fs::write(&self.state_path, bytes)
            .with_context(|| format!("writing {}", self.state_path.display()))
    }
}
