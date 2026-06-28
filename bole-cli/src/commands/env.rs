// bole-1q9
//! `bole env` — manage environment overlays (named bundles of variables).
//!
//! Overlays are immutable content-addressed objects, so every edit stores a
//! new overlay and repoints the name at it.

use anyhow::{anyhow, bail, Result};
use bole::{EnvOverlay, EnvValue};
use clap::Subcommand;

use crate::commands::secret::SECRETS_FILE;
use crate::context::RepoContext;
use crate::output::Output;
use crate::registry;

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
        Cmd::List => list(ctx, out),
    }
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
