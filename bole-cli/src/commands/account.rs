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
    Create {
        /// Where to write the 64-hex seed — your private key. Keep it secret.
        #[arg(long, default_value = "account.key")]
        out: PathBuf,
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

/// Dispatches an `account` subcommand. Needs no repository.
pub async fn run(out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Create { out: path, force } => {
            if path.exists() && !force {
                bail!("{} already exists (use --force to overwrite)", path.display());
            }
            let seed = bole::generate_seed();
            std::fs::write(&path, format!("{}\n", key::hex32(&seed)))
                .with_context(|| format!("writing key file {}", path.display()))?;
            // Lock the seed to the owner (0600) so it isn't world-readable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            let account = key::hex32(&RepoSigner::from_seed(seed).public_key());
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
