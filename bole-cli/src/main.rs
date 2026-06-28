// bole-aqk
//! `bole` — command-line interface over the bole version-control library.
//!
//! The CLI is a thin wrapper: every subcommand maps onto the library's
//! `Repository`, `ObjectStore`, `RefStore`, and `AclStore` APIs. The only
//! state the CLI owns itself is the working-tree binding in
//! [`context::CliState`].

mod commands;
mod context;
mod output;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use context::RepoContext;
use output::Output;

/// Top-level CLI definition.
#[derive(Parser)]
#[command(name = "bole", version, about = "Content-addressed version control")]
struct Cli {
    /// Emit machine-readable JSON instead of human text.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress non-error output.
    #[arg(long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

/// Top-level subcommands.
#[derive(Subcommand)]
enum Command {
    /// Create a new repository.
    Init {
        /// Directory to initialise (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show the current repository, binding, and ref count.
    Status,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let out = Output::new(cli.json, cli.quiet);

    match cli.command {
        Command::Init { path } => commands::init::run(path, &out).await,
        Command::Status => {
            let cwd = std::env::current_dir()?;
            let ctx = RepoContext::discover(&cwd).await?;
            commands::status::run(&ctx, &out).await
        }
    }
}
