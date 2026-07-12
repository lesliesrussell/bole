// bole-xwqv
//! `bole pr` — create, list, and show change proposals (the PR system).

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use bole::pr::ProposalSigner;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::key;
use crate::output::Output;

/// Change-proposal subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Open a proposal to merge one timeline (`--from`) into another (`--into`).
    Create {
        /// Source timeline ref name (the branch being proposed).
        #[arg(long)]
        from: String,
        /// Target timeline ref name (where it would merge).
        #[arg(long)]
        into: String,
        /// A short title for the proposal.
        #[arg(long)]
        title: String,
        /// Env var holding the 64-hex Ed25519 seed (the author).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List open proposals.
    List,
    /// Show one proposal by id.
    Show {
        /// The proposal's object id (64 hex).
        id: String,
    },
    // bole-t290
    /// Add a comment to a proposal's review thread.
    Comment {
        /// The proposal's object id (64 hex).
        id: String,
        /// The comment body.
        #[arg(long)]
        body: String,
        /// Mark this comment as resolving the thread.
        #[arg(long)]
        resolve: bool,
        /// Env var holding the 64-hex Ed25519 seed (the author).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    // bole-t290
    /// List a proposal's review comments.
    Comments {
        /// The proposal's object id (64 hex).
        id: String,
    },
    // bole-ooxm
    /// Merge a proposal's source into its target (approval-gated).
    Merge {
        /// The proposal's object id (64 hex).
        id: String,
        /// The merge commit message.
        #[arg(long, default_value = "merge proposal")]
        message: String,
    },
}

/// Dispatches a `pr` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { from, into, title, key_env, key_file } => {
            let seed = key::resolve(&key_env, key_file.as_deref())?;
            let signer = ProposalSigner::from_seed(seed);
            // Wall-clock seconds; the field is signed so it can't be altered.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let p = signer.sign_proposal(from.clone(), into.clone(), title.clone(), now);
            let id = ctx.repo.publish_proposal(&p).await?;
            out.emit(
                || format!("opened proposal {id}: {from} -> {into} ({title})"),
                || serde_json::json!({
                    "id": id.to_string(),
                    "from": from,
                    "into": into,
                    "title": title,
                    "author": key::hex32(&p.author),
                }),
            );
            Ok(())
        }
        Cmd::List => {
            let proposals = ctx.repo.list_proposals().await?;
            let rows: Vec<_> = proposals.iter().map(|(id, p)| serde_json::json!({
                "id": id.to_string(),
                "from": p.source,
                "into": p.target,
                "title": p.title,
                "author": key::hex32(&p.author),
                "created_at": p.created_at,
            })).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no open proposals".to_string()
                    } else {
                        proposals
                            .iter()
                            .map(|(id, p)| format!("{id}  {} -> {}  {}", p.source, p.target, p.title))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!({ "proposals": rows }),
            );
            Ok(())
        }
        Cmd::Show { id } => {
            let oid = id.parse::<bole::ObjectId>().map_err(|e| anyhow!("invalid proposal id: {e}"))?;
            let p = ctx
                .repo
                .get_proposal(&oid)
                .await?
                .ok_or_else(|| anyhow!("no such proposal: {id}"))?;
            out.emit(
                || format!("{} -> {}  {}  (author {})", p.source, p.target, p.title, key::hex32(&p.author)),
                || serde_json::json!({
                    "id": oid.to_string(),
                    "from": p.source,
                    "into": p.target,
                    "title": p.title,
                    "author": key::hex32(&p.author),
                    "created_at": p.created_at,
                }),
            );
            Ok(())
        }
        // bole-t290
        Cmd::Comment { id, body, resolve, key_env, key_file } => {
            let oid = id.parse::<bole::ObjectId>().map_err(|e| anyhow!("invalid proposal id: {e}"))?;
            let seed = key::resolve(&key_env, key_file.as_deref())?;
            let signer = ProposalSigner::from_seed(seed);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let c = signer.sign_comment(oid, body.clone(), resolve, now);
            let cid = ctx.repo.add_comment(&c).await?;
            out.emit(
                || format!("commented on {oid}{}: {body}", if resolve { " (resolved)" } else { "" }),
                || serde_json::json!({
                    "id": cid.to_string(),
                    "proposal": oid.to_string(),
                    "body": body,
                    "resolves": resolve,
                    "author": key::hex32(&c.author),
                }),
            );
            Ok(())
        }
        // bole-t290
        Cmd::Comments { id } => {
            let oid = id.parse::<bole::ObjectId>().map_err(|e| anyhow!("invalid proposal id: {e}"))?;
            let comments = ctx.repo.list_comments(&oid).await?;
            let rows: Vec<_> = comments.iter().map(|(cid, c)| serde_json::json!({
                "id": cid.to_string(),
                "body": c.body,
                "resolves": c.resolves,
                "author": key::hex32(&c.author),
                "created_at": c.created_at,
            })).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no comments".to_string()
                    } else {
                        comments
                            .iter()
                            .map(|(_, c)| format!("{}{}: {}", key::hex32(&c.author), if c.resolves { " (resolved)" } else { "" }, c.body))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!({ "proposal": oid.to_string(), "comments": rows }),
            );
            Ok(())
        }
        // bole-ooxm
        Cmd::Merge { id, message } => {
            let oid = id.parse::<bole::ObjectId>().map_err(|e| anyhow!("invalid proposal id: {e}"))?;
            let accessor = crate::actor::effective_accessor(ctx)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            match ctx.repo.merge_proposal(&oid, default_author(), now, message, &accessor).await? {
                bole::ProposalMerge::Merged(snap) => {
                    out.emit(
                        || format!("merged proposal {oid} -> {snap}"),
                        || serde_json::json!({ "merged": true, "snapshot": snap.to_string() }),
                    );
                    Ok(())
                }
                bole::ProposalMerge::Conflicts(conflicts) => {
                    let paths: Vec<String> = conflicts.iter().map(|c| c.path.clone()).collect();
                    let paths2 = paths.clone();
                    out.emit(
                        || format!("merge has conflicts ({} paths):\n{}", paths.len(), paths.join("\n")),
                        || serde_json::json!({ "merged": false, "conflicts": paths2 }),
                    );
                    anyhow::bail!("proposal not merged: {} conflicting paths", conflicts.len())
                }
            }
        }
    }
}

// bole-ooxm
/// The author string stamped on a proposal merge snapshot.
fn default_author() -> String {
    "bole-pr".to_string()
}
