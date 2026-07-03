// bole-6i1
//! `bole profile` — author and inspect this node's collaboration profile.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use crate::collabkey::signer_from;
use crate::context::RepoContext;
use crate::key;
use crate::output::Output;

/// Profile subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Author and publish (monotonically) this node's profile.
    Set {
        /// Display name to publish.
        #[arg(long)]
        display_name: String,
        /// Short bio.
        #[arg(long, default_value = "")]
        bio: String,
        /// Publicly reachable endpoint URLs.
        #[arg(long = "endpoint")]
        endpoints: Vec<String>,
        /// Env var holding the 64-hex Ed25519 seed.
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Show own profile (default) or a peer's by 64-hex key.
    Show {
        /// 64-hex public key of the peer to look up (omit for own key).
        key: Option<String>,
        /// Env var holding the 64-hex Ed25519 seed (used to derive own key).
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        /// File holding the 64-hex Ed25519 seed.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
}

/// Dispatches a `profile` subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Set { display_name, bio, endpoints, key_env, key_file } => {
            let signer = signer_from(&key_env, key_file.as_deref())?;
            // Read current seq to ensure strict monotonicity on re-publish.
            let cur = ctx.repo.profile(&signer.public_key()).await?;
            let seq = cur.map(|p| p.seq + 1).unwrap_or(1);
            let profile = signer.sign_profile(display_name, bio, endpoints, vec![], seq);
            ctx.repo.publish_profile(&profile).await?;
            let fp = bole::fingerprint(&signer.public_key());
            out.emit(
                || format!("profile published (seq={seq}, key={fp})"),
                || serde_json::json!({ "seq": seq, "key": fp }),
            );
            Ok(())
        }
        Cmd::Show { key, key_env, key_file } => {
            let k = match key {
                Some(h) => key::parse_hex_32(&h)?,
                None => signer_from(&key_env, key_file.as_deref())?.public_key(),
            };
            match ctx.repo.profile(&k).await? {
                Some(p) => out.emit(
                    || format!("display_name={} seq={}", p.display_name, p.seq),
                    || serde_json::json!({
                        "display_name": p.display_name,
                        "bio": p.bio,
                        "endpoints": p.endpoints,
                        "seq": p.seq,
                        "key": key::hex32(&p.key),
                    }),
                ),
                None => out.emit(
                    || "no profile found".to_string(),
                    || serde_json::json!({ "profile": null }),
                ),
            }
            Ok(())
        }
    }
}
