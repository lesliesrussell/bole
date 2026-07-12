// bole-cg06
//! `bole serve` / `bole push` / `bole fetch` — native repository sync over TCP.
//!
//! Wraps the library's tested wire protocol (`sync::session` +
//! `sync::transport`). No TLS or peer authentication: run only over a trusted
//! network (localhost, a VPN, an SSH tunnel) — see the threat model.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use bole::sync::session::{client_fetch, client_push};
use bole::sync::transport::{serve_tcp_once, TcpConn};
use clap::Subcommand;
use tokio::net::TcpListener;

use crate::context::RepoContext;
use crate::output::Output;
use crate::{actor, resolve};

/// Sync subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Serve this repository for fetch/push over TCP until killed.
    Serve {
        /// Address to bind, e.g. `127.0.0.1:9000` (`:0` picks a free port).
        #[arg(long, default_value = "127.0.0.1:9000")]
        listen: String,
        /// Serve exactly one connection, then exit (handy for scripts/tests).
        #[arg(long)]
        once: bool,
        /// Write the actually-bound address to this file once listening.
        #[arg(long)]
        addr_file: Option<PathBuf>,
    },
    /// Push local timelines to a peer's `bole serve`.
    Push {
        /// Peer address, e.g. `127.0.0.1:9000` (an optional `tcp://` is stripped).
        addr: String,
        /// Timelines to push.
        timelines: Vec<String>,
        /// Name for the peer's remote-tracking refs.
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Fetch a peer's refs into remote-tracking refs (never touches local timelines).
    Fetch {
        /// Peer address, e.g. `127.0.0.1:9000`.
        addr: String,
        #[arg(long, default_value = "origin")]
        remote: String,
    },
}

fn dial_addr(addr: &str) -> &str {
    addr.strip_prefix("tcp://").unwrap_or(addr)
}

/// Dispatches a `sync` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Serve { listen, once, addr_file } => {
            // The bound actor's access gates what is advertised and which pushes
            // are authorized; with no actor bound this is full access.
            let accessor = actor::effective_accessor(ctx)?;
            let listener = TcpListener::bind(&listen)
                .await
                .with_context(|| format!("binding {listen}"))?;
            let bound = listener.local_addr().context("reading bound address")?;
            if let Some(p) = &addr_file {
                std::fs::write(p, bound.to_string())
                    .with_context(|| format!("writing addr file {}", p.display()))?;
            }
            out.emit(
                || format!("serving repository on {bound} (no TLS — trusted networks only)"),
                || serde_json::json!({ "listen": bound.to_string() }),
            );
            loop {
                if let Err(e) = serve_tcp_once(&listener, &ctx.repo, &accessor).await {
                    if once {
                        return Err(e.into());
                    }
                    eprintln!("bole serve: connection error: {e}");
                }
                if once {
                    break;
                }
            }
            Ok(())
        }
        Cmd::Push { addr, timelines, remote } => {
            if timelines.is_empty() {
                anyhow::bail!("give at least one timeline to push");
            }
            let names: Vec<bole::RefName> = timelines
                .iter()
                .map(|t| resolve::ref_name(t))
                .collect::<Result<_>>()?;
            let dialed = dial_addr(&addr).to_string();
            let mut conn = TcpConn::connect(&dialed)
                .await
                .with_context(|| format!("connecting to {dialed}"))?;
            let results = client_push(&mut conn, &ctx.repo, &remote, &names).await?;
            let rows: Vec<_> = results
                .iter()
                .map(|r| serde_json::json!({ "name": r.name.as_str(), "status": format!("{:?}", r.status) }))
                .collect();
            out.emit(
                || {
                    results
                        .iter()
                        .map(|r| format!("{}  {:?}", r.name.as_str(), r.status))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
                || serde_json::json!({ "pushed_to": dialed, "results": rows }),
            );
            Ok(())
        }
        Cmd::Fetch { addr, remote } => {
            let dialed = dial_addr(&addr).to_string();
            let mut conn = TcpConn::connect(&dialed)
                .await
                .with_context(|| format!("connecting to {dialed}"))?;
            let tracked = client_fetch(&mut conn, &ctx.repo, &remote).await?;
            let rows: Vec<_> = tracked
                .iter()
                .map(|(name, id)| serde_json::json!({ "ref": name.as_str(), "target": id.to_string() }))
                .collect();
            out.emit(
                || format!("fetched {} ref(s) from {dialed}", tracked.len()),
                || serde_json::json!({ "fetched_from": dialed, "tracked": rows }),
            );
            Ok(())
        }
    }
}
