// bole-1n7
//! `bole node` — run the read-only collaboration-serve daemon.

use anyhow::Result;
use clap::Subcommand;

use bole::sync::collab::serve_collab_tcp_once;
use crate::context::RepoContext;
use crate::output::Output;

/// Node subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Run the read-only collaboration-serve daemon until killed.
    Serve {
        /// Address to bind (e.g. `127.0.0.1:47653`).
        #[arg(long)]
        listen: String,
    },
}

/// Dispatches a `node` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Serve { listen } => {
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
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    // bole-jdo
                    serve_collab_tcp_once(&listener, &ctx.repo, false),
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
