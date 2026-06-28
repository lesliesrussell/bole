// bole-tme
//! `bole merge` — check and perform three-way timeline merges.

use anyhow::{Context as _, Result};
use bole::{MergeCheck, Snapshot};
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::{actor, resolve, worktree};

/// Merge subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Check whether merging source into target is permitted (dry run).
    Check {
        /// Source timeline.
        source: String,
        /// Target timeline.
        target: String,
    },
    /// Merge source into target, advancing target when the merge is clean.
    Run {
        /// Source timeline.
        source: String,
        /// Target timeline.
        target: String,
        /// Merge commit message.
        #[arg(long, short, default_value = "merge")]
        message: String,
    },
}

fn default_author() -> String {
    std::env::var("BOLE_AUTHOR")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn short(id: &bole::ObjectId) -> String {
    id.to_string()[..12].to_string()
}

/// Dispatches a merge subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Check { source, target } => check(ctx, out, source, target).await,
        Cmd::Run { source, target, message } => do_run(ctx, out, source, target, message).await,
    }
}

async fn check(ctx: &RepoContext, out: &Output, source: String, target: String) -> Result<()> {
    let src = resolve::ref_name(&source)?;
    let tgt = resolve::ref_name(&target)?;
    let accessor = actor::effective_accessor(ctx)?;
    let result = ctx
        .repo
        .check_merge(&src, &tgt, &accessor)
        .await
        .context("checking merge")?;

    let (verdict, exposed) = match &result {
        MergeCheck::Allowed => ("allowed", vec![]),
        MergeCheck::RequiresApproval(acls) => {
            ("requires-approval", acls.iter().map(|a| a.glob.clone()).collect())
        }
        MergeCheck::Rejected(acls) => ("rejected", acls.iter().map(|a| a.glob.clone()).collect()),
    };
    let exposed2 = exposed.clone();
    out.emit(
        || {
            if exposed.is_empty() {
                verdict.to_string()
            } else {
                format!("{verdict} (protected: {})", exposed.join(", "))
            }
        },
        || serde_json::json!({ "verdict": verdict, "protected_paths": exposed2 }),
    );
    Ok(())
}

async fn do_run(
    ctx: &RepoContext,
    out: &Output,
    source: String,
    target: String,
    message: String,
) -> Result<()> {
    let src = resolve::ref_name(&source)?;
    let tgt = resolve::ref_name(&target)?;
    let accessor = actor::effective_accessor(ctx)?;

    let source_head = resolve::timeline_head(ctx, &source).await?;
    let target_head = resolve::timeline_head(ctx, &target).await?;

    let result = ctx
        .repo
        .merge_timelines(&src, &tgt, &accessor)
        .await
        .context("merging timelines")?;

    if !result.conflicts.is_empty() {
        let conflicts: Vec<String> = result.conflicts.iter().map(|c| c.path.clone()).collect();
        let conflicts2 = conflicts.clone();
        out.emit(
            || format!("merge has conflicts ({} paths):\n{}", conflicts.len(), conflicts.join("\n")),
            || serde_json::json!({ "clean": false, "conflicts": conflicts2 }),
        );
        anyhow::bail!("merge not applied: {} conflicting paths", result.conflicts.len());
    }

    // Clean merge: materialise the merged tree and advance the target.
    let root = worktree::build_root_tree(&ctx.repo.objects, &result.merged).await?;
    let merged_snap = ctx
        .repo
        .objects
        .put_snapshot(Snapshot {
            root,
            parents: vec![target_head, source_head],
            author: default_author(),
            created_at: resolve::now(),
            message,
        })
        .await
        .context("storing merge snapshot")?;
    ctx.repo
        .advance_timeline(&tgt, merged_snap, &accessor)
        .await
        .with_context(|| format!("advancing '{target}' to merge snapshot"))?;

    out.emit(
        || format!("merged {source} into {target} -> {}", short(&merged_snap)),
        || serde_json::json!({ "clean": true, "snapshot": merged_snap.to_string(), "target": target }),
    );
    Ok(())
}
