// bole-aqk
//! Repository discovery and CLI-local state.
//!
//! A bole working tree is any directory containing a `.bole/` subdirectory.
//! `.bole/` holds the library's object/ref/acl stores plus `cli-state.json`,
//! the small amount of state that belongs to the CLI and not the library
//! (the currently-bound timeline and actor).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use bole::Repository;
use serde::{Deserialize, Serialize};

/// Name of the repository directory created inside a working tree.
pub const REPO_DIR: &str = ".bole";

/// CLI-only state, persisted as `.bole/cli-state.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CliState {
    /// Timeline the working tree is currently bound to, if any.
    pub current_timeline: Option<String>,
    /// Actor whose credentials the CLI acts as, if any.
    pub current_actor: Option<String>,
}

/// An opened repository plus the paths needed to read and write CLI state.
pub struct RepoContext {
    /// The `.bole/` directory.
    pub repo_dir: PathBuf,
    /// The working tree root (parent of `.bole/`).
    pub work_dir: PathBuf,
    /// The opened library repository.
    pub repo: Repository,
}

impl RepoContext {
    /// Searches from `start` upward for a `.bole/` directory and opens it.
    pub async fn discover(start: &Path) -> Result<Self> {
        let mut dir = Some(start);
        while let Some(d) = dir {
            let candidate = d.join(REPO_DIR);
            if candidate.is_dir() {
                let repo = Repository::disk(&candidate)
                    .await
                    .with_context(|| format!("opening repository at {}", candidate.display()))?;
                return Ok(Self {
                    repo_dir: candidate,
                    work_dir: d.to_path_buf(),
                    repo,
                });
            }
            dir = d.parent();
        }
        Err(anyhow!(
            "not a bole repository (no {REPO_DIR}/ found in {} or any parent)",
            start.display()
        ))
    }

    /// Path to the CLI state file.
    fn state_path(&self) -> PathBuf {
        self.repo_dir.join("cli-state.json")
    }

    /// Reads CLI state, returning defaults if the file does not exist.
    pub fn load_state(&self) -> Result<CliState> {
        let path = self.state_path();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CliState::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Writes CLI state to disk.
    #[allow(dead_code)] // used by binding commands added in a follow-up bead
    pub fn save_state(&self, state: &CliState) -> Result<()> {
        let path = self.state_path();
        let bytes = serde_json::to_vec_pretty(state)?;
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
    }
}
