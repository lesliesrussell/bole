// bole-w3a
//! `bole timeline` — manage timelines (movable named heads over the snapshot DAG).

use anyhow::{Context as _, Result};
use bole::{Ref, TimelinePolicy};
use clap::{Subcommand, ValueEnum};

use crate::context::RepoContext;
use crate::output::Output;
use crate::resolve;

/// Timeline subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// List all timelines.
    List,
    /// Create a timeline pointing at a snapshot.
    Create {
        /// Timeline name.
        name: String,
        /// Snapshot to point at (ref, @shortcut, or object id).
        #[arg(long)]
        from: String,
        /// Advancement policy.
        #[arg(long, value_enum, default_value_t = Policy::Unrestricted)]
        policy: Policy,
        /// Lifecycle category (e.g. persistent, ephemeral).
        #[arg(long, default_value = "persistent")]
        kind: String,
        /// Optional Unix timestamp after which the timeline may be pruned.
        #[arg(long)]
        expires_at: Option<u64>,
    },
    /// Show a timeline's head, policy, and metadata.
    Show {
        /// Timeline name.
        name: String,
    },
    /// Move a timeline's head to another snapshot.
    Advance {
        /// Timeline name.
        name: String,
        /// Snapshot to move to (ref, @shortcut, or object id).
        #[arg(long)]
        to: String,
    },
    /// Delete a timeline.
    Delete {
        /// Timeline name.
        name: String,
    },
}

/// CLI mirror of [`TimelinePolicy`].
#[derive(Copy, Clone, ValueEnum)]
pub enum Policy {
    /// New head must descend from the current head.
    Ff,
    /// Snapshots may only be appended.
    Append,
    /// Head may be set to any snapshot.
    Unrestricted,
}

impl From<Policy> for TimelinePolicy {
    fn from(p: Policy) -> Self {
        match p {
            Policy::Ff => TimelinePolicy::FastForwardOnly,
            Policy::Append => TimelinePolicy::Append,
            Policy::Unrestricted => TimelinePolicy::Unrestricted,
        }
    }
}

fn policy_str(p: &TimelinePolicy) -> &'static str {
    match p {
        TimelinePolicy::FastForwardOnly => "ff",
        TimelinePolicy::Append => "append",
        TimelinePolicy::Unrestricted => "unrestricted",
    }
}

fn short(id: &bole::ObjectId) -> String {
    id.to_string()[..12].to_string()
}

/// Dispatches a timeline subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::List => list(ctx, out).await,
        Cmd::Create { name, from, policy, kind, expires_at } => {
            create(ctx, out, name, from, policy, kind, expires_at).await
        }
        Cmd::Show { name } => show(ctx, out, name).await,
        Cmd::Advance { name, to } => advance(ctx, out, name, to).await,
        Cmd::Delete { name } => delete(ctx, out, name).await,
    }
}

async fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let mut rows = Vec::new();
    for name in ctx.repo.refs.list("").context("listing refs")? {
        if let Some(Ref::Timeline(t)) = ctx.repo.refs.get(&name)? {
            rows.push((name.as_str().to_string(), t));
        }
    }
    out.emit(
        || {
            if rows.is_empty() {
                "no timelines".to_string()
            } else {
                rows.iter()
                    .map(|(n, t)| format!("{}  {}  {}", n, short(&t.head), policy_str(&t.policy)))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        },
        || {
            serde_json::json!(rows
                .iter()
                .map(|(n, t)| serde_json::json!({
                    "name": n,
                    "head": t.head.to_string(),
                    "policy": policy_str(&t.policy),
                    "kind": t.kind,
                    "expires_at": t.expires_at,
                }))
                .collect::<Vec<_>>())
        },
    );
    Ok(())
}

async fn create(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    from: String,
    policy: Policy,
    kind: String,
    expires_at: Option<u64>,
) -> Result<()> {
    let state = ctx.load_state()?;
    let head = resolve::snapshot(ctx, &state, &from).await?;
    let rn = resolve::ref_name(&name)?;
    ctx.repo
        .refs
        .create_timeline(rn, head, policy.into(), resolve::now(), kind, expires_at)
        .with_context(|| format!("creating timeline '{name}'"))?;
    out.emit(
        || format!("created timeline {name} -> {}", short(&head)),
        || serde_json::json!({ "created": name, "head": head.to_string() }),
    );
    Ok(())
}

async fn show(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    let t = ctx
        .repo
        .refs
        .get_timeline(&rn)?
        .ok_or_else(|| anyhow::anyhow!("no such timeline: {name}"))?;
    out.emit(
        || {
            format!(
                "timeline:   {name}\nhead:       {}\npolicy:     {}\nkind:       {}\ncreated_at: {}\nexpires_at: {}",
                t.head,
                policy_str(&t.policy),
                t.kind,
                t.created_at,
                t.expires_at.map(|e| e.to_string()).unwrap_or_else(|| "-".into()),
            )
        },
        || {
            serde_json::json!({
                "name": name,
                "head": t.head.to_string(),
                "policy": policy_str(&t.policy),
                "kind": t.kind,
                "created_at": t.created_at,
                "expires_at": t.expires_at,
            })
        },
    );
    Ok(())
}

async fn advance(ctx: &RepoContext, out: &Output, name: String, to: String) -> Result<()> {
    let state = ctx.load_state()?;
    let target = resolve::snapshot(ctx, &state, &to).await?;
    let rn = resolve::ref_name(&name)?;
    // bole-ef8: advance as the bound actor (full access when none is bound).
    let accessor = crate::actor::effective_accessor(ctx)?;
    ctx.repo
        .advance_timeline(&rn, target, &accessor)
        .await
        .with_context(|| format!("advancing timeline '{name}'"))?;
    out.emit(
        || format!("advanced {name} -> {}", short(&target)),
        || serde_json::json!({ "advanced": name, "head": target.to_string() }),
    );
    Ok(())
}

async fn delete(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let rn = resolve::ref_name(&name)?;
    // Confirm it is a timeline (not a tag) before deleting.
    ctx.repo
        .refs
        .get_timeline(&rn)?
        .ok_or_else(|| anyhow::anyhow!("no such timeline: {name}"))?;
    ctx.repo.refs.delete_ref(&rn).with_context(|| format!("deleting '{name}'"))?;
    out.emit(
        || format!("deleted timeline {name}"),
        || serde_json::json!({ "deleted": name }),
    );
    Ok(())
}
