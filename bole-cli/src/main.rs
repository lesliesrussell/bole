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
// bole-w3a
mod resolve;
// bole-gvy
mod worktree;

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
    // bole-w3a
    /// Manage timelines (movable named heads).
    Timeline {
        #[command(subcommand)]
        cmd: commands::timeline::Cmd,
    },
    /// Alias for `timeline`.
    Branch {
        #[command(subcommand)]
        cmd: commands::timeline::Cmd,
    },
    /// Alias for `timeline list`.
    Branches,
    /// Manage tags (immutable named pointers).
    Tag {
        #[command(subcommand)]
        cmd: commands::tag::Cmd,
    },
    // bole-gvy
    /// Create and inspect snapshots.
    Snapshot {
        #[command(subcommand)]
        cmd: commands::snapshot::Cmd,
    },
    /// Bind the work tree to a timeline and materialise files.
    Workspace {
        #[command(subcommand)]
        cmd: commands::workspace::Cmd,
    },
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
            let ctx = open().await?;
            commands::status::run(&ctx, &out).await
        }
        // bole-w3a
        Command::Timeline { cmd } | Command::Branch { cmd } => {
            let ctx = open().await?;
            commands::timeline::run(&ctx, &out, cmd).await
        }
        Command::Branches => {
            let ctx = open().await?;
            commands::timeline::run(&ctx, &out, commands::timeline::Cmd::List).await
        }
        Command::Tag { cmd } => {
            let ctx = open().await?;
            commands::tag::run(&ctx, &out, cmd).await
        }
        // bole-gvy
        Command::Snapshot { cmd } => {
            let ctx = open().await?;
            commands::snapshot::run(&ctx, &out, cmd).await
        }
        Command::Workspace { cmd } => {
            let ctx = open().await?;
            commands::workspace::run(&ctx, &out, cmd).await
        }
    }
}

/// Discovers and opens the repository from the current directory.
async fn open() -> Result<RepoContext> {
    let cwd = std::env::current_dir()?;
    RepoContext::discover(&cwd).await
}
