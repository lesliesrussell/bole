// bole-gvy
//! `bole snapshot` — create and inspect snapshots.

use anyhow::{bail, Context as _, Result};
use bole::{DiskWorkspace, Object, Snapshot, Workspace};
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::{resolve, worktree};

/// Snapshot subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Create a snapshot from the current work tree.
    Create {
        /// Build the snapshot from the working tree (currently the only source).
        #[arg(long)]
        from_workspace: bool,
        /// Commit message.
        #[arg(long, short)]
        message: String,
        /// Author (defaults to $BOLE_AUTHOR, then $USER).
        #[arg(long)]
        author: Option<String>,
        /// Do not advance the bound timeline to the new snapshot.
        #[arg(long)]
        no_advance: bool,
    },
    /// Show a snapshot's metadata.
    Show {
        /// Snapshot reference (ref, @shortcut, or object id).
        snapshot: String,
    },
    /// List a timeline's history (newest first).
    List {
        /// Timeline to walk (defaults to the bound timeline).
        #[arg(long)]
        timeline: Option<String>,
        /// Maximum number of snapshots to show.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show a snapshot's parent ids.
    Parents {
        /// Snapshot reference.
        snapshot: String,
    },
    /// Diff two snapshots by path.
    Diff {
        /// Base snapshot.
        a: String,
        /// Target snapshot.
        b: String,
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

/// Dispatches a snapshot subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { from_workspace, message, author, no_advance } => {
            create(ctx, out, from_workspace, message, author, no_advance).await
        }
        Cmd::Show { snapshot } => show(ctx, out, snapshot).await,
        Cmd::List { timeline, limit } => list(ctx, out, timeline, limit).await,
        Cmd::Parents { snapshot } => parents(ctx, out, snapshot).await,
        Cmd::Diff { a, b } => diff(ctx, out, a, b).await,
    }
}

async fn create(
    ctx: &RepoContext,
    out: &Output,
    from_workspace: bool,
    message: String,
    author: Option<String>,
    no_advance: bool,
) -> Result<()> {
    if !from_workspace {
        bail!("snapshot create requires --from-workspace");
    }
    let state = ctx.load_state()?;
    // bole-1kz
    // A DiskWorkspace bound to the current head is the one model for
    // "walk the work dir and commit it"; base becomes the sole parent.
    let base = match &state.current_timeline {
        Some(name) => Some(resolve::timeline_head(ctx, name).await?),
        None => None,
    };
    let author = author.unwrap_or_else(default_author);
    let mut ws = match base {
        Some(b) => DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, b),
        None => DiskWorkspace::new(&ctx.repo, &ctx.work_dir),
    };
    let snap_id = ws
        .commit(&author, &message, resolve::now())
        .await
        .context("storing snapshot")?;
    // bole-1kz
    let blobs = worktree::snapshot_blobs(&ctx.repo.objects, snap_id).await?;
    let file_count = blobs.len();

    // bole-ohi0: warn loudly if this snapshot captured a file that looks like a
    // bare account seed (a private key). Publishing it would leak the account.
    let mut seed_files: Vec<String> = Vec::new();
    for (path, id) in &blobs {
        if let Some(bole::Object::Blob(b)) = ctx.repo.objects.get(id).await? {
            if bole::looks_like_private_seed(&b.data) {
                seed_files.push(path.clone());
            }
        }
    }
    if !seed_files.is_empty() {
        eprintln!("⚠️  bole: this snapshot includes files that look like private account seeds:");
        for p in &seed_files {
            eprintln!("      {p}");
        }
        eprintln!("      A seed is your private key — pushing it publishes it. Remove it or add it to .boleignore:");
        eprintln!("      bole ignore add {}", seed_files.join(" "));
    }

    // Advance the bound timeline so the snapshot is reachable, unless opted out.
    let advanced = match &state.current_timeline {
        Some(name) if !no_advance => {
            let rn = resolve::ref_name(name)?;
            // bole-ef8: advance as the bound actor (full access when none is bound).
            let accessor = crate::actor::effective_accessor(ctx)?;
            ctx.repo
                .advance_timeline(&rn, snap_id, &accessor)
                .await
                .with_context(|| format!("advancing timeline '{name}'"))?;
            Some(name.clone())
        }
        _ => None,
    };

    out.emit(
        || match &advanced {
            Some(tl) => format!("snapshot {} ({} files), {tl} advanced", short(&snap_id), file_count),
            None => format!("snapshot {} ({} files)", short(&snap_id), file_count),
        },
        || serde_json::json!({
            "snapshot": snap_id.to_string(),
            "files": file_count,
            "advanced": advanced,
        }),
    );
    Ok(())
}

async fn load_snapshot(ctx: &RepoContext, id: bole::ObjectId) -> Result<Snapshot> {
    match ctx.repo.objects.get(&id).await? {
        Some(Object::Snapshot(s)) => Ok(s),
        Some(_) => bail!("{id} is not a snapshot"),
        None => bail!("snapshot not found: {id}"),
    }
}

async fn show(ctx: &RepoContext, out: &Output, spec: String) -> Result<()> {
    let state = ctx.load_state()?;
    let id = resolve::snapshot(ctx, &state, &spec).await?;
    let snap = load_snapshot(ctx, id).await?;
    let file_count = worktree::snapshot_blobs(&ctx.repo.objects, id).await?.len();
    out.emit(
        || {
            format!(
                "snapshot:   {id}\nauthor:     {}\ncreated_at: {}\nmessage:    {}\nparents:    {}\nfiles:      {}",
                snap.author,
                snap.created_at,
                snap.message,
                if snap.parents.is_empty() {
                    "(root)".to_string()
                } else {
                    snap.parents.iter().map(short).collect::<Vec<_>>().join(", ")
                },
                file_count,
            )
        },
        || serde_json::json!({
            "snapshot": id.to_string(),
            "author": snap.author,
            "created_at": snap.created_at,
            "message": snap.message,
            "parents": snap.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            "files": file_count,
        }),
    );
    Ok(())
}

async fn list(ctx: &RepoContext, out: &Output, timeline: Option<String>, limit: usize) -> Result<()> {
    let state = ctx.load_state()?;
    let tl = timeline
        .or_else(|| state.current_timeline.clone())
        .ok_or_else(|| anyhow::anyhow!("no timeline given and none bound"))?;
    let mut id = resolve::timeline_head(ctx, &tl).await?;

    let mut rows = Vec::new();
    loop {
        if rows.len() >= limit {
            break;
        }
        let snap = load_snapshot(ctx, id).await?;
        rows.push((id, snap.message.clone(), snap.author.clone()));
        match snap.parents.first() {
            Some(p) => id = *p,
            None => break,
        }
    }

    out.emit(
        || {
            rows.iter()
                .map(|(id, msg, author)| format!("{}  {}  {}", short(id), author, msg))
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            serde_json::json!(rows
                .iter()
                .map(|(id, msg, author)| serde_json::json!({
                    "snapshot": id.to_string(),
                    "message": msg,
                    "author": author,
                }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

async fn parents(ctx: &RepoContext, out: &Output, spec: String) -> Result<()> {
    let state = ctx.load_state()?;
    let id = resolve::snapshot(ctx, &state, &spec).await?;
    let snap = load_snapshot(ctx, id).await?;
    out.emit(
        || {
            if snap.parents.is_empty() {
                "(root snapshot, no parents)".to_string()
            } else {
                snap.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")
            }
        },
        || serde_json::json!(snap.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>()),
    );
    Ok(())
}

async fn diff(ctx: &RepoContext, out: &Output, a: String, b: String) -> Result<()> {
    let state = ctx.load_state()?;
    let id_a = resolve::snapshot(ctx, &state, &a).await?;
    let id_b = resolve::snapshot(ctx, &state, &b).await?;
    let base = worktree::snapshot_blobs(&ctx.repo.objects, id_a).await?;
    let target = worktree::snapshot_blobs(&ctx.repo.objects, id_b).await?;
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
                "no differences".to_string()
            } else {
                lines.join("\n")
            }
        },
        || serde_json::json!({
            "added": d.added,
            "removed": d.removed,
            "modified": d.modified,
        }),
    );
    Ok(())
}
