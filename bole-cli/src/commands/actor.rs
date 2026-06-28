// bole-ef8
//! `bole actor` — manage named actors (reusable access credentials).

use anyhow::{bail, Result};
use bole::{PathRole, Permission, TimelineRole};
use clap::{Subcommand, ValueEnum};

use crate::actor::{self, ActorDef};
use crate::context::RepoContext;
use crate::output::Output;

/// Actor subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Create a new (empty) actor.
    Create {
        /// Actor name.
        name: String,
    },
    /// List all actors.
    List,
    /// Show one actor's grants.
    Show {
        /// Actor name.
        name: String,
    },
    /// Grant a path role to an actor.
    GrantPath {
        /// Actor name.
        name: String,
        /// Glob pattern (e.g. "src/**").
        glob: String,
        /// Permission level.
        #[arg(value_enum)]
        permission: Perm,
    },
    /// Grant a timeline role to an actor.
    GrantTimeline {
        /// Actor name.
        name: String,
        /// Timeline pattern (e.g. "agent/**").
        pattern: String,
        /// Permission level.
        #[arg(value_enum)]
        permission: Perm,
    },
    /// Bind the CLI to act as an actor.
    Use {
        /// Actor name.
        name: String,
    },
    /// Show the currently-bound actor.
    Current,
}

/// CLI mirror of [`Permission`].
#[derive(Copy, Clone, ValueEnum)]
pub enum Perm {
    Read,
    Write,
}

impl From<Perm> for Permission {
    fn from(p: Perm) -> Self {
        match p {
            Perm::Read => Permission::Read,
            Perm::Write => Permission::Write,
        }
    }
}

fn perm_str(p: &Permission) -> &'static str {
    match p {
        Permission::Read => "read",
        Permission::Write => "write",
    }
}

/// Dispatches an actor subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { name } => create(ctx, out, name),
        Cmd::List => list(ctx, out),
        Cmd::Show { name } => show(ctx, out, name),
        Cmd::GrantPath { name, glob, permission } => grant_path(ctx, out, name, glob, permission),
        Cmd::GrantTimeline { name, pattern, permission } => {
            grant_timeline(ctx, out, name, pattern, permission)
        }
        Cmd::Use { name } => use_actor(ctx, out, name),
        Cmd::Current => current(ctx, out),
    }
}

fn create(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let mut reg = actor::load(ctx)?;
    if reg.actors.contains_key(&name) {
        bail!("actor already exists: {name}");
    }
    reg.actors.insert(name.clone(), ActorDef::default());
    actor::save(ctx, &reg)?;
    out.emit(|| format!("created actor {name}"), || serde_json::json!({ "created": name }));
    Ok(())
}

fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let reg = actor::load(ctx)?;
    out.emit(
        || {
            if reg.actors.is_empty() {
                "no actors".to_string()
            } else {
                reg.actors.keys().cloned().collect::<Vec<_>>().join("\n")
            }
        },
        || serde_json::json!(reg.actors.keys().cloned().collect::<Vec<_>>()),
    );
    Ok(())
}

fn show(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let def = actor::get(ctx, &name)?;
    out.emit(
        || {
            let mut lines = vec![format!("actor: {name}")];
            for r in &def.path_roles {
                lines.push(format!("  path     {} {}", perm_str(&r.permission), r.glob));
            }
            for r in &def.timeline_roles {
                lines.push(format!("  timeline {} {}", perm_str(&r.permission), r.pattern));
            }
            if def.path_roles.is_empty() && def.timeline_roles.is_empty() {
                lines.push("  (no grants)".to_string());
            }
            lines.join("\n")
        },
        || {
            serde_json::json!({
                "name": name,
                "path_roles": def.path_roles.iter().map(|r| serde_json::json!({
                    "glob": r.glob, "permission": perm_str(&r.permission),
                })).collect::<Vec<_>>(),
                "timeline_roles": def.timeline_roles.iter().map(|r| serde_json::json!({
                    "pattern": r.pattern, "permission": perm_str(&r.permission),
                })).collect::<Vec<_>>(),
            })
        },
    );
    Ok(())
}

fn grant_path(ctx: &RepoContext, out: &Output, name: String, glob: String, permission: Perm) -> Result<()> {
    let mut reg = actor::load(ctx)?;
    let def = reg.actors.get_mut(&name).ok_or_else(|| anyhow::anyhow!("no such actor: {name}"))?;
    def.path_roles.push(PathRole { glob: glob.clone(), permission: permission.into() });
    actor::save(ctx, &reg)?;
    out.emit(
        || format!("granted {name} path {glob}"),
        || serde_json::json!({ "actor": name, "glob": glob }),
    );
    Ok(())
}

fn grant_timeline(ctx: &RepoContext, out: &Output, name: String, pattern: String, permission: Perm) -> Result<()> {
    let mut reg = actor::load(ctx)?;
    let def = reg.actors.get_mut(&name).ok_or_else(|| anyhow::anyhow!("no such actor: {name}"))?;
    def.timeline_roles.push(TimelineRole { pattern: pattern.clone(), permission: permission.into() });
    actor::save(ctx, &reg)?;
    out.emit(
        || format!("granted {name} timeline {pattern}"),
        || serde_json::json!({ "actor": name, "pattern": pattern }),
    );
    Ok(())
}

fn use_actor(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    actor::bind(ctx, &name)?;
    out.emit(|| format!("now acting as {name}"), || serde_json::json!({ "actor": name }));
    Ok(())
}

fn current(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let actor = state.current_actor;
    out.emit(
        || actor.clone().unwrap_or_else(|| "(none)".to_string()),
        || serde_json::json!({ "actor": actor }),
    );
    Ok(())
}
