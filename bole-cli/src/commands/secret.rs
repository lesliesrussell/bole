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
    /// List secret names.
    List,
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
        Cmd::List => list(ctx, out),
    }
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
    let key = key::resolve(&key_env, key_file.as_deref())?;
    let plaintext = read_plaintext(from_stdin, from_file)?;
    let id = ctx.repo.objects.put_secret(&plaintext, &key).await.context("encrypting secret")?;
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
    let key = key::resolve(&key_env, key_file.as_deref())?;
    let bytes = ctx
        .repo
        .objects
        .get_secret(&id, &key)
        .await
        .context("decrypting secret")?
        .ok_or_else(|| anyhow!("secret object missing from store: {id}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    out.emit(|| text.clone(), || serde_json::json!({ "name": name, "value": text }));
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
