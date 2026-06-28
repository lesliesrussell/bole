// bole-tme
//! `bole git` — export the repository to a bare Git repo (one-way projection).

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Subcommand;

use crate::actor;
use crate::context::RepoContext;
use crate::output::Output;

/// Git subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Export the repository to a bare Git repo, filtered by the bound actor.
    Export {
        /// Destination path (created if absent; must be a bare repo if it exists).
        #[arg(long)]
        to: PathBuf,
    },
}

/// Dispatches a git subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Export { to } => export(ctx, out, to).await,
    }
}

async fn export(ctx: &RepoContext, out: &Output, to: PathBuf) -> Result<()> {
    let accessor = actor::effective_accessor(ctx)?;
    bole::project_to_git(&ctx.repo, &to, &accessor)
        .await
        .with_context(|| format!("exporting to {}", to.display()))?;
    out.emit(
        || format!("exported to {}", to.display()),
        || serde_json::json!({ "exported": to.display().to_string() }),
    );
    Ok(())
}
