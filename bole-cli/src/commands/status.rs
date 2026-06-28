// bole-aqk
//! `bole status` — summarise the current repository and binding.

use anyhow::Result;

use crate::context::RepoContext;
use crate::output::Output;

/// Prints the working tree, bound timeline/actor, and ref count.
pub async fn run(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let refs = ctx.repo.refs.list("")?;

    let timeline = state.current_timeline.as_deref().unwrap_or("(none)");
    let actor = state.current_actor.as_deref().unwrap_or("(none)");

    out.emit(
        || {
            format!(
                "work tree: {}\nrepository: {}\ntimeline:  {}\nactor:     {}\nrefs:      {}",
                ctx.work_dir.display(),
                ctx.repo_dir.display(),
                timeline,
                actor,
                refs.len(),
            )
        },
        || {
            serde_json::json!({
                "work_dir": ctx.work_dir.display().to_string(),
                "repo_dir": ctx.repo_dir.display().to_string(),
                "timeline": state.current_timeline,
                "actor": state.current_actor,
                "ref_count": refs.len(),
            })
        },
    );
    Ok(())
}
