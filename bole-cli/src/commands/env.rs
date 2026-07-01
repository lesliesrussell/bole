// bole-1q9
//! `bole env` — manage environment overlays (named bundles of variables).
//!
//! Overlays are immutable content-addressed objects, so every edit stores a
//! new overlay and repoints the name at it.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use bole::{EnvOverlay, EnvValue};
use clap::Subcommand;

use crate::commands::secret::SECRETS_FILE;
use crate::context::RepoContext;
use crate::output::Output;
use crate::{key, registry};

/// File holding the env name -> overlay-id map.
pub const ENVS_FILE: &str = "envs.json";

/// Env subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Create a new, empty overlay.
    Create {
        /// Overlay name (e.g. dev).
        name: String,
    },
    /// Set a plaintext variable.
    Set {
        /// Overlay name.
        name: String,
        /// Variable name.
        var: String,
        /// Plaintext value.
        value: String,
    },
    /// Point a variable at a named secret.
    SetSecret {
        /// Overlay name.
        name: String,
        /// Variable name.
        var: String,
        /// Secret name (from `bole secret`).
        secret: String,
    },
    /// Show an overlay (secret-backed values are redacted).
    Show {
        /// Overlay name.
        name: String,
    },
    /// Resolve an overlay to a concrete environment (secrets redacted by default).
    Resolve {
        /// Overlay name.
        name: String,
        /// Decrypt and print real secret values (requires clearance).
        #[arg(long)]
        reveal: bool,
        /// Omit secrets the actor is not cleared for instead of failing.
        #[arg(long)]
        skip_unauthorized: bool,
        /// Env var holding the 64-hex key.
        #[arg(long, default_value = "BOLE_KEY")]
        key_env: String,
        /// File holding the 64-hex key.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// List overlay names.
    List,
}

/// Dispatches an env subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { name } => create(ctx, out, name).await,
        Cmd::Set { name, var, value } => set(ctx, out, name, var, EnvValue::Plain(value)).await,
        Cmd::SetSecret { name, var, secret } => set_secret(ctx, out, name, var, secret).await,
        Cmd::Show { name } => show(ctx, out, name).await,
        Cmd::Resolve { name, reveal, skip_unauthorized, key_env, key_file } => {
            resolve(ctx, out, name, reveal, skip_unauthorized, key_env, key_file).await
        }
        Cmd::List => list(ctx, out),
    }
}

// bole-9mz
async fn resolve(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    reveal: bool,
    skip_unauthorized: bool,
    key_env: String,
    key_file: Option<PathBuf>,
) -> Result<()> {
    let overlay = load_overlay(ctx, &name).await?;

    // Redacted view (default): no decryption, no access check needed.
    if !reveal {
        out.emit(
            || {
                overlay
                    .entries
                    .iter()
                    .map(|(k, v)| match v {
                        EnvValue::Plain(s) => format!("{k}={s}"),
                        EnvValue::Secret(_) => format!("{k}=<redacted>"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            || {
                serde_json::json!({
                    "name": name,
                    "revealed": false,
                    "entries": overlay.entries.iter().map(|(k, v)| match v {
                        EnvValue::Plain(s) => serde_json::json!({ "var": k, "value": s }),
                        EnvValue::Secret(_) => serde_json::json!({ "var": k, "value": null, "secret": true }),
                    }).collect::<Vec<_>>(),
                })
            },
        );
        return Ok(());
    }

    // Reveal: access-checked resolution through the provider chain.
    let reg = registry::load(ctx, ENVS_FILE)?;
    let id_str = reg.get(&name).ok_or_else(|| anyhow!("no such overlay: {name}"))?;
    let overlay_id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;
    let chain = key::build_chain(&key_env, key_file.as_deref())?;
    let accessor = crate::actor::effective_accessor(ctx)?;
    let env = ctx
        .repo
        .resolve_overlay(&overlay_id, &chain, &accessor, skip_unauthorized)
        .await?;
    out.emit(
        || env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"),
        || serde_json::json!({ "name": name, "revealed": true, "env": env }),
    );
    Ok(())
}

async fn load_overlay(ctx: &RepoContext, name: &str) -> Result<EnvOverlay> {
    let reg = registry::load(ctx, ENVS_FILE)?;
    let id_str = reg.get(name).ok_or_else(|| anyhow!("no such overlay: {name}"))?;
    let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;
    ctx.repo
        .objects
        .get_overlay(&id)
        .await?
        .ok_or_else(|| anyhow!("overlay object missing from store: {id}"))
}

async fn store_overlay(ctx: &RepoContext, name: &str, overlay: EnvOverlay) -> Result<()> {
    let id = ctx.repo.objects.put_overlay(overlay).await?;
    let mut reg = registry::load(ctx, ENVS_FILE)?;
    reg.insert(name.to_string(), id.to_string());
    registry::save(ctx, ENVS_FILE, &reg)
}

async fn create(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    if registry::load(ctx, ENVS_FILE)?.contains_key(&name) {
        bail!("overlay already exists: {name}");
    }
    store_overlay(ctx, &name, EnvOverlay { entries: Default::default() }).await?;
    out.emit(|| format!("created overlay {name}"), || serde_json::json!({ "created": name }));
    Ok(())
}

async fn set(ctx: &RepoContext, out: &Output, name: String, var: String, value: EnvValue) -> Result<()> {
    let mut overlay = load_overlay(ctx, &name).await?;
    overlay.entries.insert(var.clone(), value);
    store_overlay(ctx, &name, overlay).await?;
    out.emit(
        || format!("set {name}.{var}"),
        || serde_json::json!({ "overlay": name, "var": var }),
    );
    Ok(())
}

async fn set_secret(ctx: &RepoContext, out: &Output, name: String, var: String, secret: String) -> Result<()> {
    let secrets = registry::load(ctx, SECRETS_FILE)?;
    let id_str = secrets.get(&secret).ok_or_else(|| anyhow!("no such secret: {secret}"))?;
    let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;
    set(ctx, out, name, var, EnvValue::Secret(id)).await
}

async fn show(ctx: &RepoContext, out: &Output, name: String) -> Result<()> {
    let overlay = load_overlay(ctx, &name).await?;
    out.emit(
        || {
            if overlay.entries.is_empty() {
                format!("overlay {name} (empty)")
            } else {
                overlay
                    .entries
                    .iter()
                    .map(|(k, v)| match v {
                        EnvValue::Plain(s) => format!("{k}={s}"),
                        EnvValue::Secret(_) => format!("{k}=<secret>"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        },
        || {
            serde_json::json!({
                "name": name,
                "entries": overlay.entries.iter().map(|(k, v)| match v {
                    EnvValue::Plain(s) => serde_json::json!({ "var": k, "kind": "plain", "value": s }),
                    EnvValue::Secret(id) => serde_json::json!({ "var": k, "kind": "secret", "secret_id": id.to_string() }),
                }).collect::<Vec<_>>(),
            })
        },
    );
    Ok(())
}

fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let reg = registry::load(ctx, ENVS_FILE)?;
    out.emit(
        || {
            if reg.is_empty() {
                "no overlays".to_string()
            } else {
                reg.keys().cloned().collect::<Vec<_>>().join("\n")
            }
        },
        || serde_json::json!(reg.keys().cloned().collect::<Vec<_>>()),
    );
    Ok(())
}
