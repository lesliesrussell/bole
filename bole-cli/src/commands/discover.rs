// bole-1n7
//! `bole discover` — pull peers' public collab objects and search the local
//! discovery index built over own + trust-graph-reachable objects.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::{collab_pull, collab_fetch_transient};
use bole::collab::fingerprint;
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
    // bole-vrf
    /// Search a relay for strangers (transient; mutates no local state).
    Relay {
        /// Relay network endpoint (host:port).
        endpoint: String,
        /// Substring to match against profile name/bio/aliases/key.
        term: String,
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
        // bole-vrf
        Cmd::Relay { endpoint, term } => {
            let stream = tokio::net::TcpStream::connect(&endpoint).await?;
            let mut conn = TcpConn::new(stream);
            let objs = collab_fetch_transient(&mut conn).await?;
            let mut hits: Vec<&bole::Profile> = objs
                .iter()
                .filter_map(|o| match o {
                    bole::CollabObject::Profile(p) => {
                        let t = term.as_str();
                        let matches = p.display_name.contains(t)
                            || p.bio.contains(t)
                            || p.dns_aliases.iter().any(|a| a.contains(t))
                            || key::hex32(&p.key).contains(t);
                        if matches { Some(p) } else { None }
                    }
                    _ => None,
                })
                .collect();
            // Deterministic, honest ranking: match already applied; tiebreak name then key fp.
            hits.sort_by(|a, b| {
                a.display_name.cmp(&b.display_name).then_with(|| fingerprint(&a.key).cmp(&fingerprint(&b.key)))
            });
            let rows: Vec<_> = hits
                .iter()
                .map(|p| serde_json::json!({
                    "key": key::hex32(&p.key),
                    "display_name": p.display_name,
                    "reach": "stranger",
                }))
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no strangers matched".to_string()
                    } else {
                        rows.iter().map(|r| format!("{} [stranger] {}", r["key"], r["display_name"]))
                            .collect::<Vec<_>>().join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
        // bole-cyw
        Cmd::Query { term, hops, key_env, key_file } => {
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            let hits = ctx.repo.query_discovery(&self_key, hops, &term).await?;
            let rows: Vec<_> = hits
                .iter()
                .map(|h| {
                    let reach = match h.reach {
                        0 => "self",
                        1 => "direct",
                        _ => "transitive",
                    };
                    serde_json::json!({
                        "key": key::hex32(&h.key),
                        "display_name": h.display_name,
                        "petname": h.petname,
                        "reach": reach,
                        "trust_path": h.trust_path.iter().map(key::hex32).collect::<Vec<_>>(),
                    })
                })
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no matches".to_string()
                    } else {
                        rows.iter()
                            .map(|r| format!("{} [{}] {}", r["key"], r["reach"], r["display_name"]))
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
