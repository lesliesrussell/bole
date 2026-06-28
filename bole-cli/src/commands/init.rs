// bole-aqk
//! `bole init` — create a new repository.

use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use bole::Repository;

use crate::context::REPO_DIR;
use crate::output::Output;

/// Creates a `.bole/` repository under `path` (default: current directory).
pub async fn run(path: PathBuf, out: &Output) -> Result<()> {
    let repo_dir = path.join(REPO_DIR);
    if repo_dir.exists() {
        bail!("{} already exists", repo_dir.display());
    }
    // Opening a disk repository creates and initialises the backing stores.
    Repository::disk(&repo_dir)
        .await
        .with_context(|| format!("initialising repository at {}", repo_dir.display()))?;

    out.emit(
        || format!("initialised empty bole repository in {}", repo_dir.display()),
        || serde_json::json!({ "initialised": repo_dir.display().to_string() }),
    );
    Ok(())
}
