// bole-ef8
//! `bole acl` — manage path/timeline protection rules and test access.

use anyhow::Result;
use bole::{PathAcl, TimelineAcl};
use clap::Subcommand;

use crate::actor;
use crate::context::RepoContext;
use crate::output::Output;
use crate::resolve;

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
    /// Explain an actor's read/write access to a path at a snapshot: the
    /// effective label, the rules that set it, and the deciding clearance.
    ExplainPath {
        #[arg(long)]
        actor: String,
        /// Snapshot to evaluate against (default: bound timeline head).
        #[arg(long, default_value = "@")]
        snapshot: String,
        path: String,
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
        Cmd::ExplainPath { actor, snapshot, path } => {
            explain_path(ctx, out, &actor, &snapshot, &path).await
        }
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

// bole-7rn
/// Emits a full read/write decision trace for an actor against a path.
async fn explain_path(
    ctx: &RepoContext,
    out: &Output,
    actor_name: &str,
    snapshot_spec: &str,
    path: &str,
) -> Result<()> {
    let acc = actor::get(ctx, actor_name)?.to_accessor();
    let state = ctx.load_state()?;
    let snap = resolve::snapshot(ctx, &state, snapshot_spec).await?;
    let exp = ctx.repo.explain_path(&acc, snap, path).await?;

    let actor_name = actor_name.to_string();
    out.emit(
        || {
            let mut s = format!("path:    {}\n", exp.path);
            s.push_str(&format!("present: {}\n", exp.present));
            s.push_str(&format!("label:   {}\n", exp.label.0));
            s.push_str(&format!(
                "rules:   {}\n",
                if exp.matched_rules.is_empty() {
                    "(none — public)".to_string()
                } else {
                    exp.matched_rules.join(", ")
                }
            ));
            s.push_str(&format!(
                "read:    {} — {}\n",
                if exp.read.allowed { "ALLOW" } else { "DENY" },
                exp.read.reason
            ));
            s.push_str(&format!(
                "write:   {} — {}",
                if exp.write.allowed { "ALLOW" } else { "DENY" },
                exp.write.reason
            ));
            s
        },
        || {
            let decision_json = |d: &bole::Decision| {
                serde_json::json!({
                    "allowed": d.allowed,
                    "reason": d.reason,
                    "confined_write_down_block": d.confined_write_down_block,
                    "clearances": d.clearances.iter().map(|c| serde_json::json!({
                        "ceiling": c.ceiling.0,
                        "scope": c.scope,
                        "scope_applies": c.scope_applies,
                        "grants_capability": c.grants_capability,
                        "dominates": c.dominates,
                        "strictly_dominates": c.strictly_dominates,
                        "decisive": c.decisive,
                    })).collect::<Vec<_>>(),
                })
            };
            serde_json::json!({
                "actor": actor_name,
                "path": exp.path,
                "present": exp.present,
                "label": exp.label.0,
                "matched_rules": exp.matched_rules,
                "read": decision_json(&exp.read),
                "write": decision_json(&exp.write),
            })
        },
    );
    Ok(())
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
