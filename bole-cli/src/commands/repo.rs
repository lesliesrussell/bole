// bole-0hg
//! `bole repo` — repository-level information.

use anyhow::Result;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;

/// Repo subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Print repository paths, counts, and current binding.
    Info,
}

/// Dispatches a repo subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Info => info(ctx, out).await,
    }
}

async fn info(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let objects = ctx.repo.objects.list().await?.len();
    let refs = ctx.repo.refs.list("")?.len();
    let timeline = state.current_timeline.clone();
    let actor = state.current_actor.clone();
    out.emit(
        || {
            format!(
                "work tree:   {}\nrepository:  {}\nbackend:     disk\nobjects:     {}\nrefs:        {}\ntimeline:    {}\nactor:       {}",
                ctx.work_dir.display(),
                ctx.repo_dir.display(),
                objects,
                refs,
                timeline.as_deref().unwrap_or("(none)"),
                actor.as_deref().unwrap_or("(none)"),
            )
        },
        || serde_json::json!({
            "work_dir": ctx.work_dir.display().to_string(),
            "repo_dir": ctx.repo_dir.display().to_string(),
            "backend": "disk",
            "objects": objects,
            "refs": refs,
            "timeline": timeline,
            "actor": actor,
        }),
    );
    Ok(())
}
