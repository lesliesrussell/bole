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
use crate::{actor, key, resolve};

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
        // bole-1x2v
        /// Run as a multi-user hub: accept owner-authenticated pushes into
        /// per-owner namespaces (refs/users/<owner-fp>/…) instead of the plain
        /// accessor-gated serve.
        #[arg(long)]
        hub: bool,
    },
    /// Push local timelines to a peer's `bole serve`.
    Push {
        /// Peer address, e.g. `127.0.0.1:9000` (an optional `tcp://` is stripped).
        addr: String,
        /// What to push. Plain: timeline names. With `--as`: `<repo>[:<timeline>]`
        /// specs (timeline defaults to `main`), pushed to your hub namespace.
        timelines: Vec<String>,
        /// Name for the peer's remote-tracking refs.
        #[arg(long, default_value = "origin")]
        remote: String,
        // bole-1x2v
        /// Owner-authenticated hub push: key file holding your 64-hex owner
        /// seed. The hub verifies it and files each repo under
        /// refs/users/<your-fp>/<repo>/<timeline>.
        #[arg(long = "as")]
        as_keyfile: Option<PathBuf>,
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
        Cmd::Serve { listen, once, addr_file, hub } => {
            // The bound actor's access gates what is advertised and which pushes
            // are authorized; with no actor bound this is full access. (Ignored
            // in hub mode, where the accessor is derived per-connection from the
            // authenticated owner.)
            let accessor = actor::effective_accessor(ctx)?;
            let listener = TcpListener::bind(&listen)
                .await
                .with_context(|| format!("binding {listen}"))?;
            let bound = listener.local_addr().context("reading bound address")?;
            if let Some(p) = &addr_file {
                std::fs::write(p, bound.to_string())
                    .with_context(|| format!("writing addr file {}", p.display()))?;
            }
            let mode = if hub { "hub" } else { "repository" };
            out.emit(
                || format!("serving {mode} on {bound} (no TLS — trusted networks only)"),
                || serde_json::json!({ "listen": bound.to_string(), "mode": mode }),
            );
            loop {
                let result = if hub {
                    // bole-1x2v: owner-authenticated push into per-owner namespaces.
                    match listener.accept().await {
                        Ok((stream, _)) => {
                            let mut conn = bole::sync::transport::TcpConn::new(stream);
                            bole::sync::hub::serve_hub_push(&mut conn, &ctx.repo).await
                        }
                        Err(e) => Err(bole::Error::Io(e)),
                    }
                } else {
                    serve_tcp_once(&listener, &ctx.repo, &accessor).await
                };
                if let Err(e) = result {
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
        Cmd::Push { addr, timelines, remote, as_keyfile } => {
            if timelines.is_empty() {
                anyhow::bail!("give at least one timeline (or, with --as, a <repo>[:<timeline>]) to push");
            }
            let dialed = dial_addr(&addr).to_string();

            // bole-1x2v: owner-authenticated hub push into refs/users/<fp>/…
            if let Some(keyfile) = as_keyfile {
                let seed = key::resolve("", Some(keyfile.as_path()))?;
                let owner = bole::RepoSigner::from_seed(seed).public_key();
                let ns = bole::sync::hub::user_namespace(&owner); // refs/users/<fp>/
                let mut pushes: Vec<(bole::RefName, bole::RefName)> = Vec::new();
                for spec in &timelines {
                    let (repo_name, tl) = spec.split_once(':').unwrap_or((spec.as_str(), "main"));
                    let local = resolve::ref_name(tl)?;
                    let remote_name = bole::RefName::new(format!("{ns}{repo_name}/{tl}"))
                        .map_err(|e| anyhow::anyhow!("bad repo/timeline name: {e}"))?;
                    pushes.push((local, remote_name));
                }
                let mut conn = TcpConn::connect(&dialed)
                    .await
                    .with_context(|| format!("connecting to {dialed}"))?;
                let results = bole::sync::hub::hub_push(&mut conn, &ctx.repo, &seed, &remote, &pushes).await?;
                let rows: Vec<_> = results
                    .iter()
                    .map(|r| serde_json::json!({ "name": r.name.as_str(), "status": format!("{:?}", r.status) }))
                    .collect();
                out.emit(
                    || results.iter().map(|r| format!("{}  {:?}", r.name.as_str(), r.status)).collect::<Vec<_>>().join("\n"),
                    || serde_json::json!({ "pushed_to": dialed, "owner": bole::key_hex(&owner), "results": rows }),
                );
                return Ok(());
            }

            let names: Vec<bole::RefName> = timelines
                .iter()
                .map(|t| resolve::ref_name(t))
                .collect::<Result<_>>()?;
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
