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
    // bole-58u
    /// Import branches and tags from a local Git repository.
    Import {
        /// Path to the source Git repository (bare or non-bare).
        path: PathBuf,
        /// Import only these branches (repeatable); default: all.
        #[arg(long = "branch")]
        branches: Vec<String>,
        /// Policy for newly created timelines: ff | append | unrestricted.
        #[arg(long, default_value = "unrestricted")]
        timeline_policy: String,
        /// Apply a WS1 label-rule file (one glob per line → protected).
        #[arg(long)]
        label_ruleset: Option<PathBuf>,
        /// Translate but write nothing and do not update the identity map.
        #[arg(long)]
        dry_run: bool,
        /// Allow a non-fast-forward advance of an existing timeline.
        #[arg(long)]
        force: bool,
    },
}

/// Dispatches a git subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Export { to } => export(ctx, out, to).await,
        // bole-58u
        Cmd::Import { path, branches, timeline_policy, label_ruleset, dry_run, force } => {
            import(ctx, out, path, branches, timeline_policy, label_ruleset, dry_run, force).await
        }
    }
}

// bole-58u
fn parse_policy(s: &str) -> Result<bole::TimelinePolicy> {
    match s {
        "ff" => Ok(bole::TimelinePolicy::FastForwardOnly),
        "append" => Ok(bole::TimelinePolicy::Append),
        "unrestricted" => Ok(bole::TimelinePolicy::Unrestricted),
        other => anyhow::bail!("unknown timeline policy '{other}' (ff | append | unrestricted)"),
    }
}

// bole-58u
#[allow(clippy::too_many_arguments)]
async fn import(
    ctx: &RepoContext,
    out: &Output,
    path: PathBuf,
    branches: Vec<String>,
    timeline_policy: String,
    label_ruleset: Option<PathBuf>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let opts = bole::repo::git_import::ImportOptions {
        branches,
        timeline_policy: parse_policy(&timeline_policy)?,
        label_ruleset,
        dry_run,
        force,
    };
    let summary = bole::repo::git_import::git_import(&ctx.repo, &path, &ctx.repo_dir, opts)
        .await
        .with_context(|| format!("importing from {}", path.display()))?;

    out.emit(
        || {
            format!(
                "imported: {} snapshot(s), {} timeline(s) created, {} advanced, {} tag(s), {} skipped",
                summary.snapshots_written,
                summary.timelines_created,
                summary.timelines_advanced,
                summary.tags_created,
                summary.skipped_via_identity_map,
            )
        },
        || {
            serde_json::json!({
                "blobs": summary.blobs_written,
                "trees": summary.trees_written,
                "snapshots": summary.snapshots_written,
                "timelines_created": summary.timelines_created,
                "timelines_advanced": summary.timelines_advanced,
                "tags": summary.tags_created,
                "skipped": summary.skipped_via_identity_map,
            })
        },
    );
    Ok(())
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
