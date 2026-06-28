// bole-w3a
//! `bole tag` — manage tags (immutable named pointers to snapshots).

use anyhow::{anyhow, Context as _, Result};
use bole::Ref;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::resolve;

/// Tag subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Create a tag pointing at a snapshot.
    Create {
        /// Tag name.
        name: String,
        /// Snapshot to point at (ref, @shortcut, or object id).
        #[arg(long)]
        target: String,
        /// Optional annotation.
        #[arg(long)]
        message: Option<String>,
    },
    /// List all tags.
    List,
    /// Show a tag's target and metadata.
    Show {
        /// Tag name.
        name: String,
    },
    /// Delete a tag.
    Delete {
        /// Tag name.
        name: String,
    },
}

fn short(id: &bole::ObjectId) -> String {
    id.to_string()[..12].to_string()
}

/// Dispatches a tag subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { name, target, message } => create(ctx, out, name, target, message).await,
        Cmd::List => list(ctx, out).await,
        Cmd::Show { name } => show(ctx, out, name).await,
        Cmd::Delete { name } => delete(ctx, out, name).await,
    }
}

async fn create(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    target: String,
    message: Option<String>,
) -> Result<()> {
    let state = ctx.load_state()?;
    let snap = resolve::snapshot(ctx, &state, &target).await?;
    let rn = resolve::ref_name(&name)?;
    ctx.repo
        .refs
        .create_tag(rn, snap, message, resolve::now())
        .with_context(|| format!("creating tag '{name}'"))?;
    out.emit(
        || format!("created tag {name} -> {}", short(&snap)),
        || serde_json::json!({ "created": name, "target": snap.to_string() }),
    );
    Ok(())
}

async fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let mut rows = Vec::new();
    for name in ctx.repo.refs.list("").context("listing refs")? {
        if let Some(Ref::Tag(t)) = ctx.repo.refs.get(&name)? {
            rows.push((name.as_str().to_string(), t));
        }
    }
    out.emit(
        || {
            if rows.is_empty() {
                "no tags".to_string()
            } else {
                rows.iter()
                    .map(|(n, t)| {
                        format!(
                            "{}  {}  {}",
                            n,
                            short(&t.target),
                            t.message.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        },
        || {
            serde_json::json!(rows
                .iter()
                .map(|(n, t)| serde_json::json!({
                    "name": n,
                    "target": t.target.to_string(),
                    "message": t.message,
                    "created_at": t.created_at,
                }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

async fn show(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    let t = ctx
        .repo
        .refs
        .get_tag(&rn)?
        .ok_or_else(|| anyhow!("no such tag: {name}"))?;
    out.emit(
        || {
            format!(
                "tag:        {name}\ntarget:     {}\ncreated_at: {}\nmessage:    {}",
                t.target,
                t.created_at,
                t.message.as_deref().unwrap_or("-"),
            )
        },
        || {
            serde_json::json!({
                "name": name,
                "target": t.target.to_string(),
                "created_at": t.created_at,
                "message": t.message,
            })
        },
    );
    Ok(())
}

async fn delete(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    ctx.repo
        .refs
        .get_tag(&rn)?
        .ok_or_else(|| anyhow!("no such tag: {name}"))?;
    ctx.repo.refs.delete_ref(&rn).with_context(|| format!("deleting '{name}'"))?;
    out.emit(
        || format!("deleted tag {name}"),
        || serde_json::json!({ "deleted": name }),
    );
    Ok(())
}
