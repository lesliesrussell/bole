// bole-1n7
//! `bole node` — run the read-only collaboration-serve daemon.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::serve_collab_tcp_once;
use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::output::Output;

/// Node subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Run the read-only collaboration-serve daemon until killed.
    // bole-vrf
    Serve {
        /// Address to bind (e.g. `127.0.0.1:47653`).
        #[arg(long)]
        listen: String,
        /// Run as a relay: serve the whole aggregate (all cached authors), not
        /// just directly-followed ones. See WS8d.
        #[arg(long)]
        relay: bool,
        // bole-lxkm
        /// Env var holding the 64-hex Ed25519 seed (required when --relay).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed (required when --relay).
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

/// Dispatches a `node` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        // bole-vrf
        // bole-lxkm
        Cmd::Serve { listen, relay, key_env, key_file } => {
            // bole-lxkm
            let relay_signer = if relay {
                Some(signer_from(&key_env, key_file.as_deref())?)
            } else {
                None
            };
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            out.emit(
                || format!("serving collab on {listen}"),
                || serde_json::json!({ "serving": listen }),
            );
            loop {
                // bole-g87: Wrap per-connection serve in a 30-second timeout so a peer
                // that connects and never sends data cannot wedge the accept loop forever.
                // Fully-concurrent (spawned-per-connection) serving is deferred to WS8c;
                // this timeout only prevents a permanent wedge.
                // bole-nbug
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    serve_collab_tcp_once(&listener, &ctx.repo, relay, relay_signer.as_ref()),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => out.emit(
                        || format!("connection error: {e}"),
                        || serde_json::json!({ "error": e.to_string() }),
                    ),
                    Err(_) => out.emit(
                        || "connection timed out".to_string(),
                        || serde_json::json!({ "error": "timeout" }),
                    ),
                }
            }
        }
    }
}
