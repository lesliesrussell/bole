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
// bole-ef8
mod actor;
// bole-1q9
mod key;
mod registry;
// bole-hrk
mod worktrees;

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
    // bole-ef8
    /// Manage named actors (reusable access credentials).
    Actor {
        #[command(subcommand)]
        cmd: commands::actor::Cmd,
    },
    /// Manage path/timeline protection and test access.
    Acl {
        #[command(subcommand)]
        cmd: commands::acl::Cmd,
    },
    // bole-tme
    /// Check and perform timeline merges.
    Merge {
        #[command(subcommand)]
        cmd: commands::merge::Cmd,
    },
    /// Export to a bare Git repository.
    Git {
        #[command(subcommand)]
        cmd: commands::git::Cmd,
    },
    // bole-1q9
    /// Store and reveal encrypted secrets by name.
    Secret {
        #[command(subcommand)]
        cmd: commands::secret::Cmd,
    },
    /// Manage environment overlays.
    Env {
        #[command(subcommand)]
        cmd: commands::env::Cmd,
    },
    // bole-ehx
    /// Configure content-gating policy (signed approvals).
    Policy {
        #[command(subcommand)]
        cmd: commands::policy::Cmd,
    },
    /// Manage the signed-approval approver registry.
    Approver {
        #[command(subcommand)]
        cmd: commands::approver::Cmd,
    },
    /// Sign a head-bound attestation approving a timeline advance/merge.
    Approve {
        /// Timeline being approved (e.g. `release/1.0`).
        timeline: String,
        /// Snapshot ref being approved (`@`, `@tag:x`, an id, or a timeline name).
        snapshot: String,
        /// Approver id you are signing as (must be registered).
        #[arg(long)]
        key_id: String,
        /// Env var holding your 64-hex Ed25519 seed.
        #[arg(long, default_value = "BOLE_APPROVER_KEY")]
        key_env: String,
        /// File holding your 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
    // bole-9mz
    /// Resolve an overlay and run a command with its variables injected.
    Run(commands::run::RunArgs),
    // bole-0hg
    /// Repository-level information.
    Repo {
        #[command(subcommand)]
        cmd: commands::repo::Cmd,
    },
    /// Plumbing: content-addressed object store.
    Object {
        #[command(subcommand)]
        cmd: commands::object::Cmd,
    },
    /// Plumbing: reference store.
    Ref {
        #[command(subcommand)]
        cmd: commands::refs::Cmd,
    },
    /// Plumbing: object-store administration.
    Store {
        #[command(subcommand)]
        cmd: commands::store::Cmd,
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
        // bole-ef8
        Command::Actor { cmd } => {
            let ctx = open().await?;
            commands::actor::run(&ctx, &out, cmd).await
        }
        Command::Acl { cmd } => {
            let ctx = open().await?;
            commands::acl::run(&ctx, &out, cmd).await
        }
        // bole-tme
        Command::Merge { cmd } => {
            let ctx = open().await?;
            commands::merge::run(&ctx, &out, cmd).await
        }
        Command::Git { cmd } => {
            let ctx = open().await?;
            commands::git::run(&ctx, &out, cmd).await
        }
        // bole-1q9
        Command::Secret { cmd } => {
            let ctx = open().await?;
            commands::secret::run(&ctx, &out, cmd).await
        }
        Command::Env { cmd } => {
            let ctx = open().await?;
            commands::env::run(&ctx, &out, cmd).await
        }
        // bole-ehx
        Command::Policy { cmd } => {
            let ctx = open().await?;
            commands::policy::run(&ctx, &out, cmd).await
        }
        Command::Approver { cmd } => {
            let ctx = open().await?;
            commands::approver::run(&ctx, &out, cmd).await
        }
        Command::Approve { timeline, snapshot, key_id, key_env, key_file } => {
            let ctx = open().await?;
            commands::approver::approve(&ctx, &out, timeline, snapshot, key_id, key_env, key_file).await
        }
        // bole-9mz
        Command::Run(args) => {
            let ctx = open().await?;
            commands::run::run(&ctx, &out, args).await
        }
        // bole-0hg
        Command::Repo { cmd } => {
            let ctx = open().await?;
            commands::repo::run(&ctx, &out, cmd).await
        }
        Command::Object { cmd } => {
            let ctx = open().await?;
            commands::object::run(&ctx, &out, cmd).await
        }
        Command::Ref { cmd } => {
            let ctx = open().await?;
            commands::refs::run(&ctx, &out, cmd).await
        }
        Command::Store { cmd } => {
            let ctx = open().await?;
            commands::store::run(&ctx, &out, cmd).await
        }
    }
}

/// Discovers and opens the repository from the current directory.
async fn open() -> Result<RepoContext> {
    let cwd = std::env::current_dir()?;
    RepoContext::discover(&cwd).await
}
