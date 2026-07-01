// bole-gvy
//! `bole workspace` — bind the work tree to a timeline and materialise files.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Subcommand;

use crate::context::{Pointer, RepoContext};
use crate::output::Output;
use bole::{DiskWorkspace, Workspace};
use crate::{resolve, worktrees};

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
    // bole-hrk
    /// Create a linked worktree directory bound to a timeline (shares this store).
    Add {
        /// Directory for the new worktree.
        path: PathBuf,
        /// Timeline to bind (must already exist).
        #[arg(long)]
        timeline: String,
        /// Actor to act as in the new worktree.
        #[arg(long = "as")]
        actor: Option<String>,
    },
    /// List the primary and all linked worktrees.
    List,
    /// Remove a linked worktree's registration (leaves your files untouched).
    Remove {
        /// Worktree directory.
        path: PathBuf,
    },
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
        // bole-hrk
        Cmd::Add { path, timeline, actor } => add(ctx, out, path, timeline, actor).await,
        Cmd::List => list(ctx, out).await,
        Cmd::Remove { path } => remove(ctx, out, path).await,
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
            // bole-1kz
            let ws = DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, h);
            Some(ws.diff().await?)
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
    // bole-1kz
    let ws = DiskWorkspace::bound(&ctx.repo, &ctx.work_dir, head);
    let d = ws.diff().await?;
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

// bole-hrk
/// Returns the timeline each existing worktree (primary + linked) is bound to.
fn existing_bindings(ctx: &RepoContext) -> Result<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();
    // Primary worktree.
    let primary_state = ctx.repo_dir.join("cli-state.json");
    let primary_path = ctx.repo_dir.parent().unwrap_or(&ctx.repo_dir).display().to_string();
    out.push((primary_path, read_binding(&primary_state)?));
    // Linked worktrees.
    for (id, entry) in worktrees::load(&ctx.repo_dir)?.worktrees {
        let sp = worktrees::meta_dir(&ctx.repo_dir, &id).join("state.json");
        out.push((entry.path, read_binding(&sp)?));
    }
    Ok(out)
}

// bole-hrk
fn read_binding(state_path: &std::path::Path) -> Result<Option<String>> {
    match std::fs::read(state_path) {
        Ok(bytes) => {
            let s: crate::context::CliState = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", state_path.display()))?;
            Ok(s.current_timeline)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", state_path.display())),
    }
}

// bole-hrk
async fn add(
    ctx: &RepoContext,
    out: &Output,
    path: PathBuf,
    timeline: String,
    actor: Option<String>,
) -> Result<()> {
    // The timeline must exist; this also gives us the head to materialise.
    let head = resolve::timeline_head(ctx, &timeline).await?;
    if let Some(name) = &actor {
        crate::actor::get(ctx, name)?; // validate the actor exists
    }

    // Refuse to clobber an existing repo/worktree at the destination.
    let pointer = path.join(crate::context::REPO_DIR);
    if pointer.exists() {
        anyhow::bail!("{} already exists", pointer.display());
    }
    std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    let abs_path = std::fs::canonicalize(&path)?;
    let abs_store = std::fs::canonicalize(&ctx.repo_dir)?;

    // Warn (but allow) if another worktree is already on this timeline.
    if existing_bindings(ctx)?.iter().any(|(_, t)| t.as_deref() == Some(timeline.as_str())) {
        eprintln!("warning: timeline '{timeline}' is already checked out in another worktree");
    }

    // Register and write per-worktree binding + pointer.
    let mut registry = worktrees::load(&ctx.repo_dir)?;
    let id = worktrees::allocate_id(&registry, &abs_path);
    let meta = worktrees::meta_dir(&ctx.repo_dir, &id);
    std::fs::create_dir_all(&meta).with_context(|| format!("creating {}", meta.display()))?;
    let state = crate::context::CliState { current_timeline: Some(timeline.clone()), current_actor: actor };
    std::fs::write(meta.join("state.json"), serde_json::to_vec_pretty(&state)?)?;

    let ptr = Pointer { store: abs_store.display().to_string(), id: id.clone() };
    std::fs::write(&pointer, serde_json::to_vec_pretty(&ptr)?)
        .with_context(|| format!("writing {}", pointer.display()))?;

    registry.worktrees.insert(id.clone(), worktrees::Entry { path: abs_path.display().to_string() });
    worktrees::save(&ctx.repo_dir, &registry)?;

    // Materialise the timeline head into the new worktree directory.
    bole::materialize(&ctx.repo.objects, head, &abs_path)
        .await
        .context("materialising timeline head into new worktree")?;

    out.emit(
        || format!("added worktree {} bound to {timeline} @ {}", abs_path.display(), short(&head)),
        || serde_json::json!({ "id": id, "path": abs_path.display().to_string(), "timeline": timeline, "head": head.to_string() }),
    );
    Ok(())
}

// bole-hrk
async fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    // (path, timeline, head, linked)
    let mut rows: Vec<(String, Option<String>, Option<bole::ObjectId>, bool)> = Vec::new();
    for (i, (path, tl)) in existing_bindings(ctx)?.into_iter().enumerate() {
        let head = match &tl {
            Some(name) => ctx.repo.refs.get_timeline(&resolve::ref_name(name)?)?.map(|t| t.head),
            None => None,
        };
        rows.push((path, tl, head, i != 0)); // first entry is the primary
    }
    out.emit(
        || {
            rows.iter()
                .map(|(p, tl, head, linked)| {
                    format!(
                        "{}  {}  {}{}",
                        p,
                        tl.as_deref().unwrap_or("(none)"),
                        head.map(|h| short(&h)).unwrap_or_else(|| "-".into()),
                        if *linked { "  (linked)" } else { "" },
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            serde_json::json!(rows
                .iter()
                .map(|(p, tl, head, linked)| serde_json::json!({
                    "path": p,
                    "timeline": tl,
                    "head": head.map(|h| h.to_string()),
                    "linked": linked,
                }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

// bole-hrk
async fn remove(ctx: &RepoContext, out: &Output, path: PathBuf) -> Result<()> {
    let abs = std::fs::canonicalize(&path).unwrap_or(path.clone());
    let abs_str = abs.display().to_string();
    let mut registry = worktrees::load(&ctx.repo_dir)?;
    let id = registry
        .worktrees
        .iter()
        .find(|(_, e)| e.path == abs_str)
        .map(|(id, _)| id.clone())
        .ok_or_else(|| anyhow::anyhow!("no linked worktree registered at {}", abs.display()))?;

    // Remove the pointer file and metadata; never touch the user's files.
    let pointer = abs.join(crate::context::REPO_DIR);
    if pointer.is_file() {
        std::fs::remove_file(&pointer).ok();
    }
    let meta = worktrees::meta_dir(&ctx.repo_dir, &id);
    if meta.is_dir() {
        std::fs::remove_dir_all(&meta).ok();
    }
    registry.worktrees.remove(&id);
    worktrees::save(&ctx.repo_dir, &registry)?;

    out.emit(
        || format!("removed worktree {}", abs.display()),
        || serde_json::json!({ "removed": abs_str, "id": id }),
    );
    Ok(())
}
