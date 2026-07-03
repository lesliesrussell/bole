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
                // Serve one accepted connection, then loop for the next. A
                // per-connection failure is logged and never stops the daemon.
                if let Err(e) = serve_collab_tcp_once(&listener, &ctx.repo).await {
                    out.emit(
                        || format!("connection error: {e}"),
                        || serde_json::json!({ "error": e.to_string() }),
                    );
                }
            }
        }
    }
}
