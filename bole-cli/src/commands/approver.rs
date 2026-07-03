// bole-ehx
//! `bole approver` — manage the signed-approval approver registry, and
//! `bole approve` — sign a head-bound attestation as a bound approver.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use bole::{Approver, AttestationSigner};
use clap::Subcommand;

use crate::context::RepoContext;
use crate::key;
use crate::output::Output;
use crate::resolve;

/// Approver subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Register an approver's public key under `key-id`.
    Add {
        /// Stable id the approver signs under (must match its attestations).
        key_id: String,
        /// Raw 64-hex Ed25519 public key.
        #[arg(long, conflicts_with = "seed")]
        public_key: Option<String>,
        /// Derive the public key from a 64-hex Ed25519 seed (convenience).
        #[arg(long)]
        seed: Option<String>,
    },
    /// List registered approvers.
    List,
}

/// Dispatches an `approver` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Add { key_id, public_key, seed } => {
            let approver = match (public_key, seed) {
                (Some(pk), None) => Approver { key_id: key_id.clone(), public_key: key::parse_hex_32(&pk)? },
                (None, Some(sd)) => {
                    AttestationSigner::from_seed(key_id.clone(), key::parse_hex_32(&sd)?).approver()
                }
                (None, None) => bail!("provide --public-key <64hex> or --seed <64hex>"),
                (Some(_), Some(_)) => bail!("use only one of --public-key or --seed"),
            };
            // Load, replace-or-add by key_id, persist.
            let mut reg = ctx.repo.approvers().await?;
            reg.approvers.retain(|a| a.key_id != approver.key_id);
            reg.add(approver.clone());
            ctx.repo.set_approvers(&reg).await?;
            let pk_hex = key::hex32(&approver.public_key);
            out.emit(
                || format!("registered approver {} ({})", approver.key_id, pk_hex),
                || serde_json::json!({ "action": "approver-add", "key_id": approver.key_id, "public_key": pk_hex }),
            );
            Ok(())
        }
        Cmd::List => {
            let reg = ctx.repo.approvers().await?;
            out.emit(
                || {
                    if reg.approvers.is_empty() {
                        "no approvers".to_string()
                    } else {
                        reg.approvers
                            .iter()
                            .map(|a| format!("{} {}", a.key_id, key::hex32(&a.public_key)))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || {
                    serde_json::json!(reg
                        .approvers
                        .iter()
                        .map(|a| serde_json::json!({ "key_id": a.key_id, "public_key": key::hex32(&a.public_key) }))
                        .collect::<Vec<_>>())
                },
            );
            Ok(())
        }
    }
}

// bole-ehx
/// `bole approve <timeline> <snapshot>` — sign a head-bound attestation approving
/// advancing/merging `timeline` to the resolved snapshot, as approver `key_id`.
#[allow(clippy::too_many_arguments)]
pub async fn approve(
    ctx: &RepoContext,
    out: &Output,
    timeline: String,
    snapshot: String,
    key_id: String,
    key_env: String,
    key_file: Option<PathBuf>,
) -> Result<()> {
    let state = ctx.load_state()?;
    let head = resolve::snapshot(ctx, &state, &snapshot).await?;
    // The signing seed comes from env/file, never argv (kept out of shell history).
    let seed = key::resolve(&key_env, key_file.as_deref())?;
    let signer = AttestationSigner::from_seed(key_id.clone(), seed);

    // Refuse to sign as a key that is not registered — a no-op attestation would
    // never count, and this catches key_id/seed mismatches early.
    let reg = ctx.repo.approvers().await?;
    match reg.find(&key_id) {
        Some(a) if a.public_key == signer.approver().public_key => {}
        Some(_) => bail!("key_id '{key_id}' is registered with a different public key"),
        None => return Err(anyhow!("approver '{key_id}' is not registered (run `approver add`)")),
    }

    let att = signer.attest(timeline.clone(), head);
    let id = ctx.repo.add_attestation(&att).await?;
    out.emit(
        || format!("approved {timeline} -> {head} as {key_id}"),
        || serde_json::json!({
            "action": "approve",
            "timeline": timeline,
            "head": head.to_string(),
            "key_id": key_id,
            "attestation": id.to_string(),
        }),
    );
    Ok(())
}

