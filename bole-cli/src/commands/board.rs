// bole-6m6f
//! `bole board` — post to, list, and reply on discussion boards.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use bole::board::BoardSigner;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::key;
use crate::output::Output;

/// Board subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Post a new top-level message to a board.
    Post {
        /// Board name (e.g. `general`).
        board: String,
        /// The message body.
        #[arg(long)]
        body: String,
        /// Env var holding the 64-hex Ed25519 seed (the author).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Reply to an existing post (threads under the same board).
    Reply {
        /// The parent post's object id (64 hex).
        parent: String,
        /// The reply body.
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List a board's posts.
    List {
        /// Board name.
        board: String,
    },
}

/// Dispatches a `board` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Post { board, body, key_env, key_file } => {
            let signer = BoardSigner::from_seed(key::resolve(&key_env, key_file.as_deref())?);
            let now = now_secs();
            let p = signer.sign_post(board.clone(), body.clone(), None, now);
            let id = ctx.repo.publish_post(&p).await?;
            out.emit(
                || format!("posted to {board}: {id}"),
                || serde_json::json!({ "id": id.to_string(), "board": board, "body": body, "author": key::hex32(&p.author) }),
            );
            Ok(())
        }
        Cmd::Reply { parent, body, key_env, key_file } => {
            let pid = parent.parse::<bole::ObjectId>().map_err(|e| anyhow!("invalid post id: {e}"))?;
            // Thread under the parent's board; the parent must exist.
            let parent_post = ctx
                .repo
                .get_post(&pid)
                .await?
                .ok_or_else(|| anyhow!("no such post: {parent}"))?;
            let signer = BoardSigner::from_seed(key::resolve(&key_env, key_file.as_deref())?);
            let now = now_secs();
            let p = signer.sign_post(parent_post.board.clone(), body.clone(), Some(pid), now);
            let id = ctx.repo.publish_post(&p).await?;
            out.emit(
                || format!("replied to {parent} on {}: {id}", parent_post.board),
                || serde_json::json!({ "id": id.to_string(), "board": parent_post.board, "parent": parent, "body": body }),
            );
            Ok(())
        }
        Cmd::List { board } => {
            let posts = ctx.repo.list_posts(&board).await?;
            let rows: Vec<_> = posts.iter().map(|(id, p)| serde_json::json!({
                "id": id.to_string(),
                "body": p.body,
                "parent": p.parent.map(|x| x.to_string()),
                "author": key::hex32(&p.author),
                "created_at": p.created_at,
            })).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        format!("no posts on {board}")
                    } else {
                        posts
                            .iter()
                            .map(|(id, p)| {
                                let re = if p.parent.is_some() { "  ↳ " } else { "" };
                                format!("{re}{id}  {}: {}", key::hex32(&p.author), p.body)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!({ "board": board, "posts": rows }),
            );
            Ok(())
        }
    }
}

// bole-6m6f
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
