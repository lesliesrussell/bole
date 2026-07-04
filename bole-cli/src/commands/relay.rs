// bole-lxkm
//! `bole relay` — manage the trusted-relay set (local, per-repo).

use anyhow::Result;
use bole::RelayPin;
use clap::Subcommand;

use crate::key;
use crate::output::Output;
use crate::context::RepoContext;

/// Relay subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Pin a trusted relay by its raw 64-hex key + endpoint (upsert).
    Add {
        /// Raw 64-hex Ed25519 public key of the relay.
        key_hex: String,
        /// Network endpoint of the relay (e.g. `127.0.0.1:47900`).
        endpoint: String,
    },
    /// Remove a pinned relay by its raw 64-hex key.
    Remove {
        /// Raw 64-hex Ed25519 public key of the relay to remove.
        key_hex: String,
    },
    /// List all pinned relays.
    List,
}

/// Dispatches a `relay` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        // bole-lxkm
        Cmd::Add { key_hex, endpoint } => {
            let key = key::parse_hex_32(&key_hex)?;
            ctx.repo.add_relay(RelayPin { key, endpoint }).await?;
            out.emit(
                || "relay pinned".to_string(),
                || serde_json::json!({ "ok": true }),
            );
            Ok(())
        }
        // bole-lxkm
        Cmd::Remove { key_hex } => {
            let key = key::parse_hex_32(&key_hex)?;
            let removed = ctx.repo.remove_relay(&key).await?;
            out.emit(
                || if removed { "relay removed".into() } else { "no such relay".into() },
                || serde_json::json!({ "removed": removed }),
            );
            Ok(())
        }
        // bole-lxkm
        Cmd::List => {
            let pins = ctx.repo.relays().await?;
            let rows: Vec<_> = pins
                .iter()
                .map(|p| serde_json::json!({ "key": key::hex32(&p.key), "endpoint": p.endpoint }))
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no relays pinned".to_string()
                    } else {
                        rows.iter()
                            .map(|r| format!("{} {}", r["key"].as_str().unwrap_or(""), r["endpoint"].as_str().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
    }
}
