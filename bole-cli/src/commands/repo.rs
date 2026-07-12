// bole-0hg
//! `bole repo` — repository-level information, and (bole-x23l) announcing the
//! named repos a developer owns for the hub/profile listing.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use bole::reporecord::RepoSigner;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::key;

/// Repo subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Print repository paths, counts, and current binding.
    Info,
    // bole-x23l
    /// Announce a named repo you own (publishes a signed RepoRecord).
    Announce {
        /// Repo name (e.g. `dotfiles`).
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        /// Env var holding the 64-hex Ed25519 seed (the owner).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    // bole-x23l
    /// List a developer's announced repos (omit key for your own).
    List {
        /// 64-hex owner key (omit for your own key).
        key: Option<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    // bole-x23l
    /// Show one repo record by owner + name.
    Show {
        /// Repo name.
        name: String,
        /// 64-hex owner key (omit for your own key).
        #[arg(long)]
        owner: Option<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

/// Dispatches a repo subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Info => info(ctx, out).await,
        // bole-x23l
        Cmd::Announce { name, description, key_env, key_file } => {
            let signer = RepoSigner::from_seed(key::resolve(&key_env, key_file.as_deref())?);
            let cur = ctx.repo.get_repo(&signer.public_key(), &name).await?;
            let seq = cur.map(|r| r.seq + 1).unwrap_or(1);
            let rec = signer.sign_repo(name.clone(), description.clone(), seq);
            let id = ctx.repo.publish_repo(&rec).await?;
            out.emit(
                || format!("announced repo {name} (seq={seq})"),
                || serde_json::json!({ "id": id.to_string(), "name": name, "description": description, "owner": key::hex32(&rec.owner), "seq": seq }),
            );
            Ok(())
        }
        Cmd::List { key, key_env, key_file } => {
            let owner = match key {
                Some(h) => key::parse_hex_32(&h)?,
                None => RepoSigner::from_seed(key::resolve(&key_env, key_file.as_deref())?).public_key(),
            };
            let repos = ctx.repo.list_repos(&owner).await?;
            let rows: Vec<_> = repos.iter().map(|r| serde_json::json!({
                "name": r.name, "description": r.description, "seq": r.seq,
            })).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no repos".to_string()
                    } else {
                        repos.iter().map(|r| format!("{}  {}", r.name, r.description)).collect::<Vec<_>>().join("\n")
                    }
                },
                || serde_json::json!({ "owner": key::hex32(&owner), "repos": rows }),
            );
            Ok(())
        }
        Cmd::Show { name, owner, key_env, key_file } => {
            let owner_key = match owner {
                Some(h) => key::parse_hex_32(&h)?,
                None => RepoSigner::from_seed(key::resolve(&key_env, key_file.as_deref())?).public_key(),
            };
            let r = ctx
                .repo
                .get_repo(&owner_key, &name)
                .await?
                .ok_or_else(|| anyhow!("no such repo: {name}"))?;
            out.emit(
                || format!("{}  {}  (owner {})", r.name, r.description, key::hex32(&r.owner)),
                || serde_json::json!({ "name": r.name, "description": r.description, "owner": key::hex32(&r.owner), "seq": r.seq }),
            );
            Ok(())
        }
    }
}

async fn info(ctx: &RepoContext, out: &Output) -> Result<()> {
    let state = ctx.load_state()?;
    let objects = ctx.repo.objects.list().await?.len();
    let refs = ctx.repo.refs.list("")?.len();
    let timeline = state.current_timeline.clone();
    let actor = state.current_actor.clone();
    out.emit(
        || {
            format!(
                "work tree:   {}\nrepository:  {}\nbackend:     disk\nobjects:     {}\nrefs:        {}\ntimeline:    {}\nactor:       {}",
                ctx.work_dir.display(),
                ctx.repo_dir.display(),
                objects,
                refs,
                timeline.as_deref().unwrap_or("(none)"),
                actor.as_deref().unwrap_or("(none)"),
            )
        },
        || serde_json::json!({
            "work_dir": ctx.work_dir.display().to_string(),
            "repo_dir": ctx.repo_dir.display().to_string(),
            "backend": "disk",
            "objects": objects,
            "refs": refs,
            "timeline": timeline,
            "actor": actor,
        }),
    );
    Ok(())
}
