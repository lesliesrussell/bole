// bole-q5rm
//! `bole account` — create and inspect an account: an ed25519 keypair whose
//! public half is your identity on a hub and whose seed lets you push repos.

use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use bole::RepoSigner;
use clap::Subcommand;

use crate::key;
use crate::output::Output;

/// Account subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Create a new account: generate a keypair, write its seed to a key file,
    /// and print the account id (public key). No repository needed.
    ///
    /// By default the seed is written OUTSIDE any repo (to ~/.bole/keys/) so a
    /// snapshot can never accidentally publish it. Pass --out to override.
    Create {
        /// Where to write the 64-hex seed — your private key. Keep it secret.
        /// Default: ~/.bole/keys/<account>.key (outside any working tree).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite the key file if it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Print the account id (public key) for a key file or env seed.
    Show {
        /// Env var holding the 64-hex seed.
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

// bole-ohi0
/// The central keyring directory (`$HOME/.bole/keys`) where account seeds live
/// by default — deliberately outside any repository working tree.
fn keyring_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; pass --out to choose where to write the seed"))?;
    Ok(home.join(".bole").join("keys"))
}

/// Dispatches an `account` subcommand. Needs no repository.
pub async fn run(out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { out: path, force } => {
            let seed = bole::generate_seed();
            let account = key::hex32(&RepoSigner::from_seed(seed).public_key());
            // bole-ohi0: default the seed OUTSIDE any repo (~/.bole/keys/<id>.key)
            // so a snapshot can never scoop it up; --out overrides.
            let path = match path {
                Some(p) => p,
                None => {
                    let dir = keyring_dir()?;
                    std::fs::create_dir_all(&dir)
                        .with_context(|| format!("creating keyring dir {}", dir.display()))?;
                    dir.join(format!("{account}.key"))
                }
            };
            if path.exists() && !force {
                bail!("{} already exists (use --force to overwrite)", path.display());
            }
            std::fs::write(&path, format!("{}\n", key::hex32(&seed)))
                .with_context(|| format!("writing key file {}", path.display()))?;
            // Lock the seed to the owner (0600) so it isn't world-readable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            out.emit(
                || format!("created account {account}\nseed written to {} — keep it secret", path.display()),
                || serde_json::json!({ "account": account, "key_file": path.display().to_string() }),
            );
            Ok(())
        }
        Cmd::Show { key_env, key_file } => {
            let seed = key::resolve(&key_env, key_file.as_deref())?;
            let account = key::hex32(&RepoSigner::from_seed(seed).public_key());
            out.emit(
                || format!("account {account}"),
                || serde_json::json!({ "account": account }),
            );
            Ok(())
        }
    }
}
