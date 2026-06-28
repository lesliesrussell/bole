// bole-gvy
//! `bole workspace` — bind the work tree to a timeline and materialise files.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::{resolve, worktree};

/// Workspace subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Bind the work tree to a timeline and materialise its head.
    Open {
        /// Timeline name.
        timeline: String,
        /// Create the timeline first, pointing at `--from`.
        #[arg(long)]
        create: bool,
        /// Snapshot to create the timeline at (required with --create).
        #[arg(long)]
        from: Option<String>,
        /// Bind the CLI to act as this actor.
        #[arg(long = "as")]
        actor: Option<String>,
    },
    /// Show the current binding and pending changes.
    Show,
    /// Materialise a snapshot's files into a directory.
    Materialize {
        /// Snapshot reference.
        #[arg(long)]
        snapshot: String,
        /// Destination directory.
        #[arg(long)]
        to: PathBuf,
    },
    /// Show how the work tree differs from the bound timeline's head.
    Diff,
    /// Unbind the work tree from its timeline.
    Clear,
}

fn short(id: &bole::ObjectId) -> String {
    id.to_string()[..12].to_string()
}

/// Dispatches a workspace subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Open { timeline, create, from, actor } => open(ctx, out, timeline, create, from, actor).await,
        Cmd::Show => show(ctx, out).await,
        Cmd::Materialize { snapshot, to } => materialize(ctx, out, snapshot, to).await,
        Cmd::Diff => diff(ctx, out).await,
        Cmd::Clear => clear(ctx, out).await,
    }
}

async fn open(
    ctx: &RepoContext,
    out: &Output,
    timeline: String,
    create: bool,
    from: Option<String>,
    actor: Option<String>,
) -> Result<()> {
    let mut state = ctx.load_state()?;

    // bole-ef8: bind the requested actor before any access-controlled work.
    if let Some(name) = &actor {
        crate::actor::bind(ctx, name)?;
        state.current_actor = Some(name.clone());
    }

    if create {
        let from = from
            .ok_or_else(|| anyhow::anyhow!("--create requires --from <snapshot>"))?;
        let head = resolve::snapshot(ctx, &state, &from).await?;
        let rn = resolve::ref_name(&timeline)?;
        ctx.repo
            .refs
            .create_timeline(rn, head, bole::TimelinePolicy::Unrestricted, resolve::now(), "persistent".into(), None)
            .with_context(|| format!("creating timeline '{timeline}'"))?;
    }

    let head = resolve::timeline_head(ctx, &timeline).await?;
    bole::materialize(&ctx.repo.objects, head, &ctx.work_dir)
        .await
        .context("materialising timeline head into work tree")?;

    state.current_timeline = Some(timeline.clone());
    ctx.save_state(&state)?;

    out.emit(
        || format!("bound to {timeline} @ {} and materialised work tree", short(&head)),
        || serde_json::json!({ "timeline": timeline, "head": head.to_string() }),
    );
    Ok(())
}

async fn show(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let (timeline, head) = match &state.current_timeline {
        Some(name) => {
            let head = resolve::timeline_head(ctx, name).await?;
            (Some(name.clone()), Some(head))
        }
        None => (None, None),
    };

    // Pending changes against the bound head.
    let pending = match head {
        Some(h) => {
            let base = worktree::snapshot_blobs(&ctx.repo.objects, h).await?;
            let target = worktree::collect_blobs(&ctx.repo.objects, &ctx.work_dir).await?;
            Some(worktree::diff(&base, &target))
        }
        None => None,
    };

    out.emit(
        || {
            let tl = timeline.as_deref().unwrap_or("(none)");
            let actor = state.current_actor.as_deref().unwrap_or("(none)");
            let changes = pending
                .as_ref()
                .map(|d| format!("{} added, {} removed, {} modified", d.added.len(), d.removed.len(), d.modified.len()))
                .unwrap_or_else(|| "(no timeline bound)".to_string());
            format!(
                "timeline: {tl}\nactor:    {actor}\nhead:     {}\npending:  {changes}",
                head.map(|h| short(&h)).unwrap_or_else(|| "-".into()),
            )
        },
        || serde_json::json!({
            "timeline": timeline,
            "actor": state.current_actor,
            "head": head.map(|h| h.to_string()),
            "pending": pending.as_ref().map(|d| serde_json::json!({
                "added": d.added, "removed": d.removed, "modified": d.modified,
            })),
        }),
    );
    Ok(())
}

async fn materialize(ctx: &RepoContext, out: &Output, snapshot: String, to: PathBuf) -> Result<()> {
    let state = ctx.load_state()?;
    let id = resolve::snapshot(ctx, &state, &snapshot).await?;
    bole::materialize(&ctx.repo.objects, id, &to)
        .await
        .with_context(|| format!("materialising {} into {}", id, to.display()))?;
    out.emit(
        || format!("materialised {} into {}", short(&id), to.display()),
        || serde_json::json!({ "snapshot": id.to_string(), "to": to.display().to_string() }),
    );
    Ok(())
}

async fn diff(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let name = state
        .current_timeline
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no timeline bound; use `bole workspace open <timeline>`"))?;
    let head = resolve::timeline_head(ctx, name).await?;
    let base = worktree::snapshot_blobs(&ctx.repo.objects, head).await?;
    let target = worktree::collect_blobs(&ctx.repo.objects, &ctx.work_dir).await?;
    let d = worktree::diff(&base, &target);
    out.emit(
        || {
            let mut lines = Vec::new();
            for p in &d.added {
                lines.push(format!("+ {p}"));
            }
            for p in &d.removed {
                lines.push(format!("- {p}"));
            }
            for p in &d.modified {
                lines.push(format!("~ {p}"));
            }
            if lines.is_empty() {
                "clean (work tree matches head)".to_string()
            } else {
                lines.join("\n")
            }
        },
        || serde_json::json!({ "added": d.added, "removed": d.removed, "modified": d.modified }),
    );
    Ok(())
}

async fn clear(ctx: &RepoContext, out: &Output) -> Result<()> {
    let mut state = ctx.load_state()?;
    state.current_timeline = None;
    ctx.save_state(&state)?;
    out.emit(|| "cleared timeline binding".to_string(), || serde_json::json!({ "cleared": true }));
    Ok(())
}
