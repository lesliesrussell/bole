// bole-9mz
//! `bole run` — resolve an env overlay (access-checked) and exec a command with
//! its variables injected. Secrets exist only in the child's environment block;
//! nothing is written to disk and bole's own output never prints values.

use std::path::PathBuf;
use std::process::Command as ProcCommand;

use anyhow::{anyhow, Context as _, Result};
use clap::Args;

use crate::commands::env::ENVS_FILE;
use crate::context::RepoContext;
use crate::output::Output;
use crate::{key, registry};

/// Arguments for `bole run`.
#[derive(Args)]
pub struct RunArgs {
    /// Overlay name to resolve and inject.
    #[arg(long)]
    env: String,
    /// Start from an empty environment (default: inherit the parent env).
    #[arg(long)]
    clean: bool,
    /// Omit secrets the actor is not cleared for instead of failing.
    #[arg(long)]
    skip_unauthorized: bool,
    /// Env var holding the 64-hex key.
    #[arg(long, default_value = "BOLE_KEY")]
    key_env: String,
    /// File holding the 64-hex key.
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// The command and its arguments, after `--`.
    #[arg(last = true, required = true)]
    cmd: Vec<String>,
}

/// Resolves the overlay and execs the command, propagating its exit status.
pub async fn run(ctx: &RepoContext, _out: &Output, args: RunArgs) -> Result<()> {
    let reg = registry::load(ctx, ENVS_FILE)?;
    let id_str = reg.get(&args.env).ok_or_else(|| anyhow!("no such overlay: {}", args.env))?;
    let overlay_id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;

    let chain = key::build_chain(&args.key_env, args.key_file.as_deref())?;
    let accessor = crate::actor::effective_accessor(ctx)?;
    let resolved = ctx
        .repo
        .resolve_overlay(&overlay_id, &chain, &accessor, args.skip_unauthorized)
        .await
        .context("resolving overlay")?;

    let (prog, rest) = args.cmd.split_first().ok_or_else(|| anyhow!("no command given"))?;
    let mut child = ProcCommand::new(prog);
    child.args(rest);
    if args.clean {
        child.env_clear();
    }
    for (k, v) in &resolved {
        child.env(k, v);
    }
    let status = child.status().with_context(|| format!("spawning {prog}"))?;
    std::process::exit(status.code().unwrap_or(1));
}
