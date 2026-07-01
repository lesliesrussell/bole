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
    // bole-81z
    /// Consolidate loose objects into an immutable pack.
    Repack,
    // bole-81z
    /// Garbage-collect unreachable objects (roots: refs + secret/env registries).
    Gc {
        /// Protect objects written within this many seconds (write-race grace).
        #[arg(long, default_value_t = 7200)]
        grace_secs: u64,
    },
}

/// Dispatches a store subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Stats => stats(ctx, out).await,
        Cmd::Fsck => fsck(ctx, out).await,
        Cmd::Repack => repack(ctx, out).await,
        Cmd::Gc { grace_secs } => gc(ctx, out, grace_secs).await,
    }
}

// bole-81z
async fn repack(ctx: &RepoContext, out: &Output) -> Result<()> {
    let packed = ctx.repo.objects.compact().await?;
    out.emit(
        || format!("repacked {packed} loose object(s)"),
        || serde_json::json!({ "packed": packed }),
    );
    Ok(())
}

// bole-81z
async fn gc(ctx: &RepoContext, out: &Output, grace_secs: u64) -> Result<()> {
    use crate::commands::env::ENVS_FILE;
    use crate::commands::secret::SECRETS_FILE;

    // Registry-rooted objects (secrets + overlays) are roots too, since they are
    // referenced outside the ref store (spec O8).
    let mut extra_roots = Vec::new();
    for file in [SECRETS_FILE, ENVS_FILE] {
        for id_str in crate::registry::load(ctx, file)?.values() {
            if let Ok(id) = id_str.parse::<bole::ObjectId>() {
                extra_roots.push(id);
            }
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let removed = ctx.repo.gc(&extra_roots, grace_secs, now).await?;
    out.emit(
        || format!("collected {removed} object(s)"),
        || serde_json::json!({ "removed": removed }),
    );
    Ok(())
}

async fn stats(ctx: &RepoContext, out: &Output) -> Result<()> {
    // bole-81z: count() is cheap on packs (index headers), no per-object walk.
    let objects = ctx.repo.objects.count().await? as usize;
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
