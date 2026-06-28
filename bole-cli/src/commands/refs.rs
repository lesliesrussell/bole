// bole-0hg
//! `bole ref` — low-level access to the reference store.

use anyhow::{anyhow, Result};
use bole::Ref;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::resolve;

/// Ref subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// List ref names under an optional prefix.
    List {
        /// Name prefix (default: all).
        #[arg(default_value = "")]
        prefix: String,
    },
    /// Show a ref's kind and target.
    Get {
        /// Ref name.
        name: String,
    },
    /// Delete a ref (timeline or tag).
    Delete {
        /// Ref name.
        name: String,
    },
}

/// Dispatches a ref subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::List { prefix } => list(ctx, out, prefix),
        Cmd::Get { name } => get(ctx, out, name),
        Cmd::Delete { name } => delete(ctx, out, name),
    }
}

fn list(ctx: &RepoContext, out: &Output, prefix: String) -> Result<()> {
    let names: Vec<String> = ctx.repo.refs.list(&prefix)?.iter().map(|n| n.as_str().to_string()).collect();
    out.emit(
        || if names.is_empty() { "no refs".to_string() } else { names.join("\n") },
        || serde_json::json!(names),
    );
    Ok(())
}

fn get(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    let r = ctx.repo.refs.get(&rn)?.ok_or_else(|| anyhow!("no such ref: {name}"))?;
    let (kind, target) = match &r {
        Ref::Timeline(t) => ("timeline", t.head.to_string()),
        Ref::Tag(t) => ("tag", t.target.to_string()),
    };
    out.emit(
        || format!("{name}\nkind:   {kind}\ntarget: {target}"),
        || serde_json::json!({ "name": name, "kind": kind, "target": target }),
    );
    Ok(())
}

fn delete(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    ctx.repo.refs.get(&rn)?.ok_or_else(|| anyhow!("no such ref: {name}"))?;
    ctx.repo.refs.delete_ref(&rn)?;
    out.emit(|| format!("deleted ref {name}"), || serde_json::json!({ "deleted": name }));
    Ok(())
}
