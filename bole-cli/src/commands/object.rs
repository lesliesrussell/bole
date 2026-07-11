// bole-0hg
//! `bole object` — low-level access to the content-addressed object store.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use bole::{Object, ObjectId};
use bytes::Bytes;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;

/// Object subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// List all object ids.
    List,
    /// Show an object's decoded structure.
    Get {
        /// Object id (64 hex).
        id: String,
    },
    /// Print an object's kind.
    Type {
        /// Object id.
        id: String,
    },
    /// Write a blob's raw bytes to stdout.
    Cat {
        /// Object id (must be a blob).
        id: String,
    },
    /// Store a file as a blob and print its id.
    PutBlob {
        /// File to store.
        file: PathBuf,
    },
}

fn parse_id(s: &str) -> Result<ObjectId> {
    s.parse::<ObjectId>().map_err(|e| anyhow!("invalid object id '{s}': {e}"))
}

fn kind(obj: &Object) -> &'static str {
    match obj {
        Object::Blob(_) => "blob",
        Object::Tree(_) => "tree",
        Object::Snapshot(_) => "snapshot",
        Object::Secret(_) => "secret",
        Object::EnvOverlay(_) => "env-overlay",
        // bole-fo2
        Object::Policy(_) => "policy",
        // bole-9mz
        Object::SecretV2(_) => "secret",
        // bole-amy
        Object::MultiRecipientSecret(_) => "secret",
        // bole-6i1
        Object::Collab(_) => "collab",
        // bole-060a
        Object::ChangeProposal(_) => "change-proposal",
    }
}

/// Dispatches an object subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::List => list(ctx, out).await,
        Cmd::Get { id } => get(ctx, out, id).await,
        Cmd::Type { id } => type_of(ctx, out, id).await,
        Cmd::Cat { id } => cat(ctx, id).await,
        Cmd::PutBlob { file } => put_blob(ctx, out, file).await,
    }
}

async fn load(ctx: &RepoContext, id: ObjectId) -> Result<Object> {
    ctx.repo.objects.get(&id).await?.ok_or_else(|| anyhow!("object not found: {id}"))
}

async fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let ids = ctx.repo.objects.list().await?;
    let strs: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    out.emit(|| strs.join("\n"), || serde_json::json!(strs));
    Ok(())
}

async fn get(ctx: &RepoContext, out: &Output, id: String) -> Result<()> {
    let oid = parse_id(&id)?;
    let obj = load(ctx, oid).await?;
    let k = kind(&obj);
    out.emit(
        || format!("{id}\nkind: {k}\n{obj:#?}"),
        || serde_json::json!({ "id": id, "kind": k, "debug": format!("{obj:?}") }),
    );
    Ok(())
}

async fn type_of(ctx: &RepoContext, out: &Output, id: String) -> Result<()> {
    let obj = load(ctx, parse_id(&id)?).await?;
    let k = kind(&obj);
    out.emit(|| k.to_string(), || serde_json::json!({ "id": id, "kind": k }));
    Ok(())
}

async fn cat(ctx: &RepoContext, id: String) -> Result<()> {
    match load(ctx, parse_id(&id)?).await? {
        Object::Blob(b) => {
            std::io::stdout().write_all(&b.data)?;
            Ok(())
        }
        other => bail!("{id} is a {}, not a blob", kind(&other)),
    }
}

async fn put_blob(ctx: &RepoContext, out: &Output, file: PathBuf) -> Result<()> {
    let bytes = std::fs::read(&file)?;
    let id = ctx.repo.objects.put_blob(Bytes::from(bytes)).await?;
    out.emit(|| id.to_string(), || serde_json::json!({ "id": id.to_string() }));
    Ok(())
}
