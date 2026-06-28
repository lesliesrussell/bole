// bole-0hg
//! `bole store` — object-store administration.

use anyhow::Result;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;

/// Store subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Print object and ref counts.
    Stats,
    /// Verify every object decodes; report any that fail.
    Fsck,
}

/// Dispatches a store subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Stats => stats(ctx, out).await,
        Cmd::Fsck => fsck(ctx, out).await,
    }
}

async fn stats(ctx: &RepoContext, out: &Output) -> Result<()> {
    let objects = ctx.repo.objects.list().await?.len();
    let refs = ctx.repo.refs.list("")?.len();
    let path_acls = ctx.repo.acls.list_path_acls()?.len();
    let timeline_acls = ctx.repo.acls.list_timeline_acls()?.len();
    out.emit(
        || format!("objects:        {objects}\nrefs:           {refs}\npath acls:      {path_acls}\ntimeline acls:  {timeline_acls}"),
        || serde_json::json!({
            "objects": objects,
            "refs": refs,
            "path_acls": path_acls,
            "timeline_acls": timeline_acls,
        }),
    );
    Ok(())
}

async fn fsck(ctx: &RepoContext, out: &Output) -> Result<()> {
    let ids = ctx.repo.objects.list().await?;
    let total = ids.len();
    let mut bad = Vec::new();
    for id in &ids {
        // A decode failure surfaces as an error from get().
        if ctx.repo.objects.get(id).await.is_err() {
            bad.push(id.to_string());
        }
    }
    let bad2 = bad.clone();
    out.emit(
        || {
            if bad.is_empty() {
                format!("ok: {total} objects verified")
            } else {
                format!("{} of {total} objects failed to decode:\n{}", bad.len(), bad.join("\n"))
            }
        },
        || serde_json::json!({ "total": total, "bad": bad2 }),
    );
    Ok(())
}
