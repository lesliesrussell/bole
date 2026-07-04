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
    // bole-lxkm
    /// Search trusted relays for strangers with a verifiable trust path (transient).
    Relay {
        /// Substring to match against profile name/bio/aliases/key.
        term: String,
        /// Ad-hoc: query a single unpinned endpoint (host:port) instead of the pinned set.
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long, default_value_t = 4)]
        max_hops: u8,
        /// Env var holding the 64-hex Ed25519 seed (used to derive own key).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
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
        // bole-lxkm
        Cmd::Relay { term, endpoint, max_hops, key_env, key_file } => {
            // bole-9vhh
            if term.len() < bole::MIN_SEARCH_TERM_LEN {
                anyhow::bail!("search term must be at least {} characters", bole::MIN_SEARCH_TERM_LEN);
            }
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            // Gather the querier's own verified edges (own published + tracked cache).
            let mut own_edges = ctx.repo.public_edges().await?;
            for o in ctx.repo.tracked_collab().await? {
                if let bole::CollabObject::TrustEdge(e) = o {
                    own_edges.push(e);
                }
            }
            let hits = match endpoint {
                // bole-mbz6
                // Ad-hoc one-off: use server-side search (transparent optimization; fallback
                // to whole-aggregate is handled inside collab_search itself).
                Some(addr) => {
                    let stream = tokio::net::TcpStream::connect(&addr).await?;
                    let mut conn = TcpConn::new(stream);
                    let corpus = bole::collab_search(&mut conn, &term, max_hops).await?;
                    bole::rank_strangers(&self_key, &own_edges, &corpus, &term, max_hops)
                }
                // Query the pinned set: authenticate each, merge, attribute.
                None => {
                    let relays = ctx.repo.relays().await?;
                    bole::query_relay_set(&self_key, &own_edges, &relays, &term, max_hops).await
                }
            };
            let rows: Vec<_> = hits.iter().map(|h| {
                let trust_path = h.trust_path.as_ref().map(|path| {
                    path.iter().map(|hop| serde_json::json!({
                        "key": key::hex32(&hop.key),
                        "via": match hop.via {
                            bole::TrustKind::Vouch => "vouch",
                            bole::TrustKind::Follow => "follow",
                            bole::TrustKind::Review => "review",
                        },
                    })).collect::<Vec<_>>()
                });
                serde_json::json!({
                    "key": key::hex32(&h.key),
                    "display_name": h.display_name,
                    "reach": "stranger",
                    "trust_path": trust_path,
                    "hops": h.hops,
                    "relays": h.relays.iter().map(key::hex32).collect::<Vec<_>>(),
                })
            }).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no strangers matched".to_string()
                    } else {
                        rows.iter().map(|r| {
                            let hops = if r["hops"].is_null() {
                                "no path".to_string()
                            } else {
                                format!("{} hops", r["hops"])
                            };
                            let nrelays = r["relays"].as_array().map(|a| a.len()).unwrap_or(0);
                            format!("{} [stranger, {}, via {} relays] {}", r["key"], hops, nrelays, r["display_name"])
                        }).collect::<Vec<_>>().join("\n")
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
