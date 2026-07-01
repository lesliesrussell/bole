// bole-gvy
//! `bole workspace` — bind the work tree to a timeline and materialise files.

use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
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
    List {
        // bole-3hj
        /// Exit 1 if any linked worktree is stale.
        #[arg(long)]
        check: bool,
    },
    /// Remove a linked worktree's registration (leaves your files untouched).
    Remove {
        /// Worktree directory.
        path: PathBuf,
    },
    // bole-3hj
    /// Drop registry entries whose linked worktree can no longer be verified.
    Prune {
        /// Print what would be pruned; modify nothing.
        #[arg(long)]
        dry_run: bool,
        /// Also prune recoverable (store-moved) entries.
        #[arg(long)]
        include_recoverable: bool,
    },
    // bole-3hj
    /// Reconcile pointer/registry inconsistencies (moved store/dir, orphans).
    Repair {
        /// Print what would change; modify nothing.
        #[arg(long)]
        dry_run: bool,
        /// R2: update a moved worktree's registered path to this new location.
        #[arg(long = "moved-to")]
        moved_to: Option<PathBuf>,
        /// R3: adopt an orphaned pointer directory into the registry.
        #[arg(long)]
        adopt: Option<PathBuf>,
        /// Worktree id (required with --moved-to).
        id: Option<String>,
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
        Cmd::List { check } => list(ctx, out, check).await,
        Cmd::Remove { path } => remove(ctx, out, path).await,
        // bole-3hj
        Cmd::Prune { dry_run, include_recoverable } => prune(ctx, out, dry_run, include_recoverable),
        Cmd::Repair { dry_run, moved_to, adopt, id } => repair(ctx, out, dry_run, moved_to, adopt, id),
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
// bole-3hj: (path, timeline, head, linked, status)
type ListRow = (String, Option<String>, Option<bole::ObjectId>, bool, String);

async fn list(ctx: &RepoContext, out: &Output, check: bool) -> Result<()> {
    let mut rows: Vec<ListRow> = Vec::new();

    // Primary worktree (always present, always "ok").
    let primary_state = ctx.repo_dir.join("cli-state.json");
    let primary_path = ctx.repo_dir.parent().unwrap_or(&ctx.repo_dir).display().to_string();
    let primary_tl = read_binding(&primary_state)?;
    let primary_head = head_of(ctx, &primary_tl)?;
    rows.push((primary_path, primary_tl, primary_head, false, "ok".to_string()));

    // Linked worktrees, each with a consistency status (bole-3hj).
    let mut any_stale = false;
    for (id, entry) in &worktrees::load(&ctx.repo_dir)?.worktrees {
        let status = worktrees::classify(&ctx.repo_dir, id, entry);
        if !status.is_ok() {
            any_stale = true;
        }
        let sp = worktrees::meta_dir(&ctx.repo_dir, id).join("state.json");
        let tl = read_binding(&sp).unwrap_or(None);
        let head = head_of(ctx, &tl)?;
        rows.push((entry.path.clone(), tl, head, true, status.as_str().to_string()));
    }

    out.emit(
        || {
            rows.iter()
                .map(|(p, tl, head, linked, status)| {
                    let kind = if *linked { "  (linked)" } else { "  (primary)" };
                    let stale = if *linked && status != "ok" {
                        format!("  [STALE: {status}]")
                    } else {
                        String::new()
                    };
                    format!(
                        "{}  {}  {}{}{}",
                        p,
                        tl.as_deref().unwrap_or("(none)"),
                        head.map(|h| short(&h)).unwrap_or_else(|| "-".into()),
                        kind,
                        stale,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            serde_json::json!(rows
                .iter()
                .map(|(p, tl, head, linked, status)| serde_json::json!({
                    "path": p,
                    "timeline": tl,
                    "head": head.map(|h| h.to_string()),
                    "linked": linked,
                    "status": status,
                }))
                .collect::<Vec<_>>())
        },
    );

    // bole-3hj: --check turns a stale linked worktree into a non-zero exit.
    if check && any_stale {
        std::process::exit(1);
    }
    Ok(())
}

// bole-3hj
/// Resolves a timeline binding to its current head, if the timeline exists.
fn head_of(ctx: &RepoContext, tl: &Option<String>) -> Result<Option<bole::ObjectId>> {
    match tl {
        Some(name) => Ok(ctx.repo.refs.get_timeline(&resolve::ref_name(name)?)?.map(|t| t.head)),
        None => Ok(None),
    }
}

// bole-3hj
/// `workspace prune` — drop entries whose linked worktree cannot be verified.
fn prune(
    ctx: &RepoContext,
    out: &Output,
    dry_run: bool,
    include_recoverable: bool,
) -> Result<()> {
    use crate::context::REPO_DIR;
    let mut registry = worktrees::load(&ctx.repo_dir)?;
    let ids: Vec<String> = registry.worktrees.keys().cloned().collect();
    let mut pruned: Vec<(String, String, String)> = Vec::new();
    let mut clean = 0usize;

    for id in ids {
        let entry = registry.worktrees[&id].clone();
        let status = worktrees::classify(&ctx.repo_dir, &id, &entry);
        let prunable = status.is_prunable() || (include_recoverable && status.is_recoverable());
        if !prunable {
            if status.is_ok() {
                clean += 1;
            }
            continue;
        }
        if !dry_run {
            // Remove the metadata dir (idempotent).
            let _ = std::fs::remove_dir_all(worktrees::meta_dir(&ctx.repo_dir, &id));
            // For a bad/mismatched pointer with a surviving directory, remove ONLY
            // the `.bole` pointer file; never touch the user's other files.
            if matches!(status, worktrees::Status::BadPointer(_) | worktrees::Status::WrongId { .. }) {
                let ptr = std::path::Path::new(&entry.path).join(REPO_DIR);
                if ptr.is_file() {
                    let _ = std::fs::remove_file(ptr);
                }
            }
            registry.worktrees.remove(&id);
        }
        pruned.push((id, entry.path, status.as_str().to_string()));
    }

    if !dry_run {
        worktrees::save(&ctx.repo_dir, &registry)?;
    }

    let pruned_for_json = pruned.clone();
    let n = pruned.len();
    out.emit(
        || {
            if pruned.is_empty() {
                format!("nothing to prune; {clean} entr{} clean.", if clean == 1 { "y" } else { "ies" })
            } else {
                let mut lines: Vec<String> = pruned
                    .iter()
                    .map(|(id, path, status)| {
                        let verb = if dry_run { "would prune" } else { "pruned" };
                        format!("{verb} worktree {id}  (path: {path}) — {status}")
                    })
                    .collect();
                lines.push(format!("{n} entr{} {}, {clean} clean.", if n == 1 { "y" } else { "ies" }, if dry_run { "would be pruned" } else { "pruned" }));
                lines.join("\n")
            }
        },
        || {
            serde_json::json!(pruned_for_json
                .iter()
                .map(|(id, path, status)| serde_json::json!({
                    "id": id, "path": path, "status": status, "pruned": !dry_run,
                }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

// bole-3hj
/// `workspace repair` — reconcile recoverable pointer/registry inconsistencies.
fn repair(
    ctx: &RepoContext,
    out: &Output,
    dry_run: bool,
    moved_to: Option<PathBuf>,
    adopt: Option<PathBuf>,
    id: Option<String>,
) -> Result<()> {
    if let Some(path) = adopt {
        return repair_adopt(ctx, out, dry_run, path);
    }
    if let Some(new_path) = moved_to {
        let id = id.ok_or_else(|| anyhow::anyhow!("--moved-to requires a worktree id"))?;
        return repair_moved(ctx, out, dry_run, new_path, id);
    }
    repair_store_moved(ctx, out, dry_run)
}

// bole-3hj — R1: the primary store was moved; rewrite each pointer's `store`.
fn repair_store_moved(ctx: &RepoContext, out: &Output, dry_run: bool) -> Result<()> {
    use crate::context::{Pointer, REPO_DIR};
    let registry = worktrees::load(&ctx.repo_dir)?;
    let store = ctx.repo_dir.display().to_string();
    let mut repaired: Vec<(String, String)> = Vec::new();
    for (id, entry) in &registry.worktrees {
        if worktrees::classify(&ctx.repo_dir, id, entry).is_recoverable() {
            if !dry_run {
                let ptr = Pointer { store: store.clone(), id: id.clone() };
                let bytes = serde_json::to_vec_pretty(&ptr)?;
                std::fs::write(std::path::Path::new(&entry.path).join(REPO_DIR), bytes)
                    .with_context(|| format!("rewriting pointer for {id}"))?;
            }
            repaired.push((id.clone(), entry.path.clone()));
        }
    }
    let repaired_json = repaired.clone();
    out.emit(
        || {
            if repaired.is_empty() {
                "no store-moved worktrees to repair.".to_string()
            } else {
                let verb = if dry_run { "would repair" } else { "repaired" };
                repaired
                    .iter()
                    .map(|(id, path)| format!("{verb} worktree {id}  (pointer store updated)  path: {path}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        },
        || {
            serde_json::json!(repaired_json
                .iter()
                .map(|(id, path)| serde_json::json!({ "id": id, "path": path, "repaired": !dry_run }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

// bole-3hj — R2: a worktree directory moved; update its registered path.
fn repair_moved(ctx: &RepoContext, out: &Output, dry_run: bool, new_path: PathBuf, id: String) -> Result<()> {
    use crate::context::{Pointer, REPO_DIR};
    let mut registry = worktrees::load(&ctx.repo_dir)?;
    if !registry.worktrees.contains_key(&id) {
        bail!("no such worktree id '{id}'; did you mean --adopt?");
    }
    let canon = std::fs::canonicalize(&new_path)
        .with_context(|| format!("resolving {}", new_path.display()))?;
    let ptr_bytes = std::fs::read(canon.join(REPO_DIR))
        .with_context(|| format!("reading pointer at {}", canon.display()))?;
    let ptr: Pointer = serde_json::from_slice(&ptr_bytes).context("parsing pointer")?;
    if !worktrees::same_path(&ptr.store, &ctx.repo_dir) {
        bail!("pointer store '{}' does not match this store", ptr.store);
    }
    if ptr.id != id {
        bail!("pointer id '{}' does not match '{id}'", ptr.id);
    }
    let old = registry.worktrees[&id].path.clone();
    let new_str = canon.display().to_string();
    if !dry_run {
        registry.worktrees.get_mut(&id).unwrap().path = new_str.clone();
        worktrees::save(&ctx.repo_dir, &registry)?;
    }
    out.emit(
        || format!("{} worktree {id}: path {old} -> {new_str}", if dry_run { "would repair" } else { "repaired" }),
        || serde_json::json!({ "id": id, "old_path": old, "new_path": new_str, "repaired": !dry_run }),
    );
    Ok(())
}

// bole-3hj — R3: adopt an orphaned pointer directory into the registry.
fn repair_adopt(ctx: &RepoContext, out: &Output, dry_run: bool, path: PathBuf) -> Result<()> {
    use crate::context::{CliState, Pointer, REPO_DIR};
    let canon = std::fs::canonicalize(&path)
        .with_context(|| format!("resolving {}", path.display()))?;
    let ptr_bytes = std::fs::read(canon.join(REPO_DIR))
        .with_context(|| format!("reading pointer at {}", canon.display()))?;
    let ptr: Pointer = serde_json::from_slice(&ptr_bytes).context("parsing pointer")?;
    if !worktrees::same_path(&ptr.store, &ctx.repo_dir) {
        bail!("pointer store '{}' does not match this store", ptr.store);
    }
    let id = ptr.id.clone();
    let canon_str = canon.display().to_string();
    let mut registry = worktrees::load(&ctx.repo_dir)?;
    if let Some(existing) = registry.worktrees.get(&id) {
        if worktrees::same_path(&existing.path, &canon) {
            out.emit(
                || format!("worktree {id} already consistent at {canon_str}"),
                || serde_json::json!({ "id": id, "path": canon_str, "adopted": false }),
            );
            return Ok(());
        }
        bail!("id '{id}' is already registered at '{}'; use --moved-to to update", existing.path);
    }
    if !dry_run {
        registry.worktrees.insert(id.clone(), worktrees::Entry { path: canon_str.clone() });
        worktrees::save(&ctx.repo_dir, &registry)?;
        // Ensure the metadata dir exists with a default (unbound) state.
        let meta = worktrees::meta_dir(&ctx.repo_dir, &id);
        std::fs::create_dir_all(&meta)?;
        let state_path = meta.join("state.json");
        if !state_path.exists() {
            std::fs::write(&state_path, serde_json::to_vec_pretty(&CliState::default())?)?;
        }
    }
    out.emit(
        || format!("{} worktree {id} at {canon_str}", if dry_run { "would adopt" } else { "adopted" }),
        || serde_json::json!({ "id": id, "path": canon_str, "adopted": !dry_run }),
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
