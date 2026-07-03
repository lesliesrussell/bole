// bole-6i1
//! `bole trust` — author and inspect this node's trust edges.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use bole::{Key, TrustKind};
use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::key;
use crate::output::Output;

/// Trust subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Publish a Follow edge to a 64-hex peer key.
    Follow {
        /// Target peer's 64-hex public key.
        key: String,
        /// Env var holding the 64-hex Ed25519 seed.
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Publish a Vouch edge (identity suggestion) with a petname.
    Vouch {
        /// Target peer's 64-hex public key.
        key: String,
        /// Local petname to vouch for this key under.
        #[arg(long)]
        name: String,
        /// Env var holding the 64-hex Ed25519 seed.
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List this node's public trust edges.
    List {
        /// Env var holding the 64-hex Ed25519 seed.
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

/// Dispatches a `trust` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Follow { key: key_hex, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            let to = key::parse_hex_32(&key_hex)?;
            let seq = next_seq_for(ctx, &signer.public_key(), TrustKind::Follow, &to).await?;
            let edge = signer.sign_edge(to, TrustKind::Follow, None, seq);
            ctx.repo.publish_edge(&edge).await?;
            out.emit(
                || format!("followed {} (seq={seq})", &key_hex[..8]),
                || serde_json::json!({ "followed": key_hex, "seq": seq }),
            );
            Ok(())
        }
        Cmd::Vouch { key: key_hex, name, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            let to = key::parse_hex_32(&key_hex)?;
            let seq = next_seq_for(ctx, &signer.public_key(), TrustKind::Vouch, &to).await?;
            let edge = signer.sign_edge(to, TrustKind::Vouch, Some(name.clone()), seq);
            ctx.repo.publish_edge(&edge).await?;
            out.emit(
                || format!("vouched for {} as {:?} (seq={seq})", &key_hex[..8], name),
                || serde_json::json!({ "vouched": key_hex, "name": name, "seq": seq }),
            );
            Ok(())
        }
        Cmd::List { key_env, key_file } => {
            let me = signer_from(&key_env, key_file.as_deref())?.public_key();
            let edges = ctx.repo.public_edges().await?;
            let my_edges: Vec<_> = edges.iter().filter(|e| e.from_key == me).collect();
            let rows: Vec<_> = my_edges.iter().map(|e| serde_json::json!({
                "to": key::hex32(&e.to_key),
                "kind": format!("{:?}", e.kind),
                "petname": e.petname,
                "seq": e.seq,
            })).collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no trust edges".to_string()
                    } else {
                        my_edges.iter().map(|e| format!("{} {:?}", key::hex32(&e.to_key), e.kind))
                            .collect::<Vec<_>>().join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
    }
}

/// Returns the next valid seq for `(from, kind, to)` by reading the current
/// public edge and adding one. Returns 1 if no edge exists yet.
/// WS8a rejects a seq not strictly greater than the current edge's seq, so a
/// constant would break re-follows of the same key.
async fn next_seq_for(
    ctx: &RepoContext,
    from: &Key,
    kind: TrustKind,
    to: &Key,
) -> anyhow::Result<u64> {
    let edges = ctx.repo.public_edges().await?;
    let cur = edges.iter().find(|e| &e.from_key == from && e.kind == kind && &e.to_key == to);
    Ok(cur.map(|e| e.seq + 1).unwrap_or(1))
}

