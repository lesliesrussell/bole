// bole-ef8
//! `bole acl` — manage path/timeline protection rules and test access.

use anyhow::Result;
use bole::{PathAcl, TimelineAcl};
use clap::Subcommand;

use crate::actor;
use crate::context::RepoContext;
use crate::output::Output;

/// ACL subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Manage path-protection rules.
    Path {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Manage timeline-protection rules.
    Timeline {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Test whether an actor can read a path.
    CanReadPath {
        #[arg(long)]
        actor: String,
        path: String,
    },
    /// Test whether an actor can write a path.
    CanWritePath {
        #[arg(long)]
        actor: String,
        path: String,
    },
    /// Test whether an actor can read a timeline.
    CanReadTimeline {
        #[arg(long)]
        actor: String,
        timeline: String,
    },
    /// Test whether an actor can write a timeline.
    CanWriteTimeline {
        #[arg(long)]
        actor: String,
        timeline: String,
    },
}

/// Protect/unprotect/list operations shared by path and timeline rules.
#[derive(Subcommand)]
pub enum RuleCmd {
    /// Mark a glob/pattern as protected.
    Protect {
        /// Glob (paths) or pattern (timelines).
        value: String,
    },
    /// Remove a protection rule.
    Unprotect {
        /// Glob (paths) or pattern (timelines).
        value: String,
    },
    /// List protection rules.
    List,
}

/// Whether a [`RuleCmd`] applies to paths or timelines.
enum Domain {
    Path,
    Timeline,
}

/// Dispatches an acl subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Path { cmd } => rule(ctx, out, Domain::Path, cmd),
        Cmd::Timeline { cmd } => rule(ctx, out, Domain::Timeline, cmd),
        Cmd::CanReadPath { actor, path } => can(ctx, out, &actor, "read_path", &path),
        Cmd::CanWritePath { actor, path } => can(ctx, out, &actor, "write_path", &path),
        Cmd::CanReadTimeline { actor, timeline } => can(ctx, out, &actor, "read_timeline", &timeline),
        Cmd::CanWriteTimeline { actor, timeline } => can(ctx, out, &actor, "write_timeline", &timeline),
    }
}

fn rule(ctx: &RepoContext, out: &Output, domain: Domain, cmd: RuleCmd) -> Result<()> {
    match (domain, cmd) {
        (Domain::Path, RuleCmd::Protect { value }) => {
            ctx.repo.acls.set_path_acl(PathAcl { glob: value.clone() })?;
            emit_change(out, "protected path", &value);
        }
        (Domain::Path, RuleCmd::Unprotect { value }) => {
            ctx.repo.acls.remove_path_acl(&value)?;
            emit_change(out, "unprotected path", &value);
        }
        (Domain::Path, RuleCmd::List) => {
            let rules: Vec<String> = ctx.repo.acls.list_path_acls()?.into_iter().map(|a| a.glob).collect();
            emit_list(out, rules);
        }
        (Domain::Timeline, RuleCmd::Protect { value }) => {
            ctx.repo.acls.set_timeline_acl(TimelineAcl { pattern: value.clone() })?;
            emit_change(out, "protected timeline", &value);
        }
        (Domain::Timeline, RuleCmd::Unprotect { value }) => {
            ctx.repo.acls.remove_timeline_acl(&value)?;
            emit_change(out, "unprotected timeline", &value);
        }
        (Domain::Timeline, RuleCmd::List) => {
            let rules: Vec<String> =
                ctx.repo.acls.list_timeline_acls()?.into_iter().map(|a| a.pattern).collect();
            emit_list(out, rules);
        }
    }
    Ok(())
}

fn emit_change(out: &Output, action: &str, value: &str) {
    let action = action.to_string();
    let value = value.to_string();
    out.emit(
        || format!("{action}: {value}"),
        || serde_json::json!({ "action": action, "value": value }),
    );
}

fn emit_list(out: &Output, rules: Vec<String>) {
    out.emit(
        || {
            if rules.is_empty() {
                "no rules".to_string()
            } else {
                rules.join("\n")
            }
        },
        || serde_json::json!(rules),
    );
}

fn can(ctx: &RepoContext, out: &Output, actor_name: &str, kind: &str, target: &str) -> Result<()> {
    let acc = actor::get(ctx, actor_name)?.to_accessor();
    let allowed = match kind {
        "read_path" => acc.can_read_path(target),
        "write_path" => acc.can_write_path(target),
        "read_timeline" => acc.can_read_timeline(target),
        "write_timeline" => acc.can_write_timeline(target),
        _ => unreachable!(),
    };
    let actor_name = actor_name.to_string();
    let target = target.to_string();
    out.emit(
        || format!("{}: {actor_name} {} {target}", if allowed { "allowed" } else { "denied" }, kind),
        || serde_json::json!({ "actor": actor_name, "check": kind, "target": target, "allowed": allowed }),
    );
    Ok(())
}
