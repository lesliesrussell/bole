// bole-1q9
//! `bole secret` — store, reveal, and rotate encrypted secrets by name.

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context as _, Result};
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;
use crate::{key, registry};

/// File holding the secret name -> object-id map.
pub const SECRETS_FILE: &str = "secrets.json";

/// Secret subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Store a new secret under a name.
    Put {
        /// Secret name (e.g. prod/db/url).
        name: String,
        /// Read the plaintext from stdin.
        #[arg(long)]
        from_stdin: bool,
        /// Read the plaintext from a file.
        #[arg(long)]
        from_file: Option<PathBuf>,
        /// Environment variable holding the 64-hex key.
        #[arg(long, default_value = "BOLE_KEY")]
        key_env: String,
        /// File holding the 64-hex key.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Decrypt and print a secret.
    Reveal {
        /// Secret name.
        name: String,
        #[arg(long, default_value = "BOLE_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Replace a secret's value (the name keeps pointing at the new ciphertext).
    Rotate {
        /// Secret name.
        name: String,
        #[arg(long)]
        from_stdin: bool,
        #[arg(long)]
        from_file: Option<PathBuf>,
        #[arg(long, default_value = "BOLE_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Rotate the master key: re-wrap secrets under a new key (values untouched).
    Rekey {
        /// Rekey every registered secret.
        #[arg(long)]
        all: bool,
        /// Specific secret names to rekey (when not `--all`).
        names: Vec<String>,
        /// Env var holding the OLD 64-hex key.
        #[arg(long, default_value = "BOLE_KEY")]
        from_key_env: String,
        /// File holding the OLD 64-hex key.
        #[arg(long)]
        from_key_file: Option<PathBuf>,
        /// Env var holding the NEW 64-hex key.
        #[arg(long, default_value = "BOLE_NEW_KEY")]
        to_key_env: String,
        /// File holding the NEW 64-hex key.
        #[arg(long)]
        to_key_file: Option<PathBuf>,
    },
    /// List secret names.
    List,
    /// Grant another actor read access by wrapping the data key for their key.
    ///
    /// A plain (single-recipient) secret is upgraded to multi-recipient on the
    /// first grant, keeping the granter as a reader.
    GrantActor {
        /// Secret name.
        name: String,
        /// Env var holding YOUR 64-hex key (must already be able to read).
        #[arg(long, default_value = "BOLE_KEY")]
        key_env: String,
        /// File holding YOUR 64-hex key.
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Env var holding the RECIPIENT's 64-hex key.
        #[arg(long, default_value = "BOLE_RECIPIENT_KEY")]
        recipient_key_env: String,
        /// File holding the RECIPIENT's 64-hex key.
        #[arg(long)]
        recipient_key_file: Option<PathBuf>,
    },
    /// Revoke an actor's read access (drops their wrapped data key).
    ///
    /// Forward revocation only: a reader who already extracted the value is not
    /// un-taught it — follow with `secret rotate` to defeat that.
    RevokeActor {
        /// Secret name.
        name: String,
        /// Env var holding the RECIPIENT's 64-hex key (to derive their key ref).
        #[arg(long, default_value = "BOLE_RECIPIENT_KEY")]
        recipient_key_env: String,
        /// File holding the RECIPIENT's 64-hex key.
        #[arg(long)]
        recipient_key_file: Option<PathBuf>,
    },
}

// bole-9mz
/// The AAD for a secret named `name`: binds `{version, label}`, where the label
/// is this secret name's effective WS1 label (omitted when public/bottom).
async fn secret_aad(ctx: &RepoContext, name: &str) -> Result<bole::SecretAad> {
    let lattice = ctx.repo.acls.lattice()?;
    let rules = ctx.repo.acls.label_ruleset()?;
    let label = rules.label_for_secret(&lattice, name);
    let label = if label == lattice.bottom() { None } else { Some(label) };
    Ok(bole::SecretAad::v2(label))
}

fn read_plaintext(from_stdin: bool, from_file: Option<PathBuf>) -> Result<Vec<u8>> {
    match (from_stdin, from_file) {
        (true, None) => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).context("reading stdin")?;
            Ok(buf)
        }
        (false, Some(p)) => std::fs::read(&p).with_context(|| format!("reading {}", p.display())),
        (false, None) => bail!("provide --from-stdin or --from-file"),
        (true, Some(_)) => bail!("use only one of --from-stdin or --from-file"),
    }
}

/// Dispatches a secret subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Put { name, from_stdin, from_file, key_env, key_file } => {
            store(ctx, out, name, from_stdin, from_file, key_env, key_file, false).await
        }
        Cmd::Rotate { name, from_stdin, from_file, key_env, key_file } => {
            store(ctx, out, name, from_stdin, from_file, key_env, key_file, true).await
        }
        Cmd::Reveal { name, key_env, key_file } => reveal(ctx, out, name, key_env, key_file).await,
        Cmd::Rekey { all, names, from_key_env, from_key_file, to_key_env, to_key_file } => {
            rekey(ctx, out, all, names, from_key_env, from_key_file, to_key_env, to_key_file).await
        }
        Cmd::List => list(ctx, out),
        Cmd::GrantActor { name, key_env, key_file, recipient_key_env, recipient_key_file } => {
            grant_actor(ctx, out, name, key_env, key_file, recipient_key_env, recipient_key_file)
                .await
        }
        Cmd::RevokeActor { name, recipient_key_env, recipient_key_file } => {
            revoke_actor(ctx, out, name, recipient_key_env, recipient_key_file).await
        }
    }
}

// bole-amy
/// Grants an actor read access by wrapping the secret's data key under their key.
#[allow(clippy::too_many_arguments)]
async fn grant_actor(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    key_env: String,
    key_file: Option<PathBuf>,
    recipient_key_env: String,
    recipient_key_file: Option<PathBuf>,
) -> Result<()> {
    let mut reg = registry::load(ctx, SECRETS_FILE)?;
    let id_str = reg.get(&name).ok_or_else(|| anyhow!("no such secret: {name}"))?;
    let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;

    let granter = key::build_chain(&key_env, key_file.as_deref())?;
    let recipient = key::build_chain(&recipient_key_env, recipient_key_file.as_deref())?;
    let recipient_provider = recipient.active()?;
    let recipient_ref = recipient_provider.active_key_ref().to_string();

    let new_id = ctx
        .repo
        .objects
        .grant_secret_recipient(&id, &granter, recipient_provider)
        .await
        .context("granting recipient")?;
    reg.insert(name.clone(), new_id.to_string());
    registry::save(ctx, SECRETS_FILE, &reg)?;
    out.emit(
        || format!("granted {recipient_ref} read access to secret {name}"),
        || serde_json::json!({ "action": "grant-actor", "name": name, "recipient": recipient_ref, "id": new_id.to_string() }),
    );
    Ok(())
}

// bole-amy
/// Revokes an actor's read access by dropping their wrapped data key.
async fn revoke_actor(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    recipient_key_env: String,
    recipient_key_file: Option<PathBuf>,
) -> Result<()> {
    let mut reg = registry::load(ctx, SECRETS_FILE)?;
    let id_str = reg.get(&name).ok_or_else(|| anyhow!("no such secret: {name}"))?;
    let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;

    let recipient = key::build_chain(&recipient_key_env, recipient_key_file.as_deref())?;
    let recipient_ref = recipient.active()?.active_key_ref().to_string();

    let new_id = ctx
        .repo
        .objects
        .revoke_secret_recipient(&id, &recipient_ref)
        .await
        .context("revoking recipient")?;
    reg.insert(name.clone(), new_id.to_string());
    registry::save(ctx, SECRETS_FILE, &reg)?;
    out.emit(
        || format!("revoked {recipient_ref} from secret {name}"),
        || serde_json::json!({ "action": "revoke-actor", "name": name, "recipient": recipient_ref, "id": new_id.to_string() }),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn store(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    from_stdin: bool,
    from_file: Option<PathBuf>,
    key_env: String,
    key_file: Option<PathBuf>,
    must_exist: bool,
) -> Result<()> {
    let mut reg = registry::load(ctx, SECRETS_FILE)?;
    if must_exist && !reg.contains_key(&name) {
        bail!("no such secret: {name}");
    }
    if !must_exist && reg.contains_key(&name) {
        bail!("secret already exists: {name} (use `secret rotate`)");
    }
    // bole-9mz: write v2 envelope secrets under the resolved master key.
    let chain = key::build_chain(&key_env, key_file.as_deref())?;
    let aad = secret_aad(ctx, &name).await?;
    let plaintext = read_plaintext(from_stdin, from_file)?;
    let id = ctx
        .repo
        .objects
        .put_secret_enveloped(&plaintext, chain.active()?, aad)
        .await
        .context("encrypting secret")?;
    reg.insert(name.clone(), id.to_string());
    registry::save(ctx, SECRETS_FILE, &reg)?;
    let verb = if must_exist { "rotated" } else { "stored" };
    out.emit(
        || format!("{verb} secret {name}"),
        || serde_json::json!({ "action": verb, "name": name, "id": id.to_string() }),
    );
    Ok(())
}

async fn reveal(
    ctx: &RepoContext,
    out: &Output,
    name: String,
    key_env: String,
    key_file: Option<PathBuf>,
) -> Result<()> {
    let reg = registry::load(ctx, SECRETS_FILE)?;
    let id_str = reg.get(&name).ok_or_else(|| anyhow!("no such secret: {name}"))?;
    let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;
    // bole-9mz: resolve v2 (envelope) or legacy v1 secrets via the chain.
    let chain = key::build_chain(&key_env, key_file.as_deref())?;
    let bytes = ctx
        .repo
        .objects
        .get_secret_resolved(&id, &chain)
        .await
        .context("decrypting secret")?
        .ok_or_else(|| anyhow!("secret object missing from store: {id}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    out.emit(|| text.clone(), || serde_json::json!({ "name": name, "value": text }));
    Ok(())
}

// bole-9mz
/// Master-key rotation: re-wrap the targeted secrets' data keys from the old key
/// to the new key (values untouched; v1 secrets are upgraded to v2), then repoint
/// the secrets registry and any overlays referencing the rotated objects.
#[allow(clippy::too_many_arguments)]
async fn rekey(
    ctx: &RepoContext,
    out: &Output,
    all: bool,
    names: Vec<String>,
    from_key_env: String,
    from_key_file: Option<PathBuf>,
    to_key_env: String,
    to_key_file: Option<PathBuf>,
) -> Result<()> {
    use std::collections::BTreeMap;

    let mut reg = registry::load(ctx, SECRETS_FILE)?;
    let targets: Vec<String> = if all {
        reg.keys().cloned().collect()
    } else if names.is_empty() {
        bail!("give secret names or --all");
    } else {
        names
    };

    let old = key::build_chain(&from_key_env, from_key_file.as_deref())?;
    let new = key::build_chain(&to_key_env, to_key_file.as_deref())?;
    let new_provider = new.active()?;

    // Map each target name to its current object id.
    let mut name_ids: Vec<(String, bole::ObjectId)> = Vec::new();
    for name in &targets {
        let id_str = reg.get(name).ok_or_else(|| anyhow!("no such secret: {name}"))?;
        let id = id_str.parse::<bole::ObjectId>().map_err(|e| anyhow!("corrupt registry id: {e}"))?;
        name_ids.push((name.clone(), id));
    }
    let ids: Vec<bole::ObjectId> = name_ids.iter().map(|(_, id)| *id).collect();

    let mapping = ctx.repo.rekey(&ids, &old, new_provider).await.context("rekeying secrets")?;
    // old id -> new id
    let remap: BTreeMap<bole::ObjectId, bole::ObjectId> = mapping.into_iter().collect();

    // Repoint the secrets registry.
    for (name, old_id) in &name_ids {
        if let Some(new_id) = remap.get(old_id) {
            reg.insert(name.clone(), new_id.to_string());
        }
    }
    registry::save(ctx, SECRETS_FILE, &reg)?;

    // Repoint overlays that reference any rotated object (O4 auto-repoint).
    let mut envs = registry::load(ctx, crate::commands::env::ENVS_FILE)?;
    let mut env_updates: Vec<(String, String)> = Vec::new();
    for (env_name, oid_str) in envs.iter() {
        let oid = match oid_str.parse::<bole::ObjectId>() {
            Ok(o) => o,
            Err(_) => continue,
        };
        let overlay = match ctx.repo.objects.get_overlay(&oid).await? {
            Some(o) => o,
            None => continue,
        };
        let mut changed = false;
        let mut entries = overlay.entries.clone();
        for value in entries.values_mut() {
            if let bole::EnvValue::Secret(id) = value {
                if let Some(new_id) = remap.get(id) {
                    *value = bole::EnvValue::Secret(*new_id);
                    changed = true;
                }
            }
        }
        if changed {
            let new_oid = ctx.repo.objects.put_overlay(bole::EnvOverlay { entries }).await?;
            env_updates.push((env_name.clone(), new_oid.to_string()));
        }
    }
    for (env_name, new_oid) in env_updates {
        envs.insert(env_name, new_oid);
    }
    registry::save(ctx, crate::commands::env::ENVS_FILE, &envs)?;

    let count = remap.len();
    out.emit(
        || format!("rekeyed {count} secret(s)"),
        || serde_json::json!({ "rekeyed": count, "secrets": targets }),
    );
    Ok(())
}

fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let reg = registry::load(ctx, SECRETS_FILE)?;
    out.emit(
        || {
            if reg.is_empty() {
                "no secrets".to_string()
            } else {
                reg.keys().cloned().collect::<Vec<_>>().join("\n")
            }
        },
        || serde_json::json!(reg.keys().cloned().collect::<Vec<_>>()),
    );
    Ok(())
}
