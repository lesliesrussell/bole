// bole-1n7
//! `bole discover` — pull peers' public collab objects and search the local
//! discovery index built over own + trust-graph-reachable objects.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::collab_pull;
use bole::sync::transport::TcpConn;
use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::key;
use crate::output::Output;

/// Discover subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Pull a peer's public collab objects from a network address.
    Pull {
        /// Peer address (e.g. `127.0.0.1:47653`).
        addr: String,
    },
    /// Search the local discovery index (own + tracked peers).
    Query {
        /// Term matched against peer display names.
        term: String,
        /// Trust-graph hop limit (friend-of-friend = 2).
        #[arg(long, default_value_t = 2)]
        hops: u8,
        /// Env var holding the 64-hex Ed25519 seed (used to derive own key).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

/// Dispatches a `discover` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Pull { addr } => {
            let stream = tokio::net::TcpStream::connect(&addr).await?;
            let mut conn = TcpConn::new(stream);
            let peer = collab_pull(&mut conn, &ctx.repo).await?;
            let fp = key::hex32(&peer);
            out.emit(
                || format!("pulled {fp}"),
                || serde_json::json!({ "pulled": fp }),
            );
            Ok(())
        }
        Cmd::Query { term, hops, key_env, key_file } => {
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            let idx = ctx.repo.local_discovery_index(&self_key, hops).await?;
            let rows: Vec<_> = idx
                .query(&term)
                .into_iter()
                .map(|r| {
                    let name = match &r.object {
                        bole::CollabObject::Profile(p) => p.display_name.clone(),
                        bole::CollabObject::TrustEdge(_) => String::new(),
                    };
                    serde_json::json!({
                        "key": key::hex32(&r.key),
                        "name": name,
                        "distance": r.distance,
                    })
                })
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no matches".to_string()
                    } else {
                        rows.iter()
                            .map(|r| format!("{} {}", r["key"], r["name"]))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
    }
}
