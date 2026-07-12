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
    // bole-k93a
    /// Aggregated hub bundle for a dev: profile + own trust out-edges + (own) timelines.
    Bundle {
        /// 64-hex public key to bundle (omit for own key).
        key: Option<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
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
            // bole-g87
            let hex = key::hex32(&signer.public_key());
            out.emit(
                || format!("profile published (seq={seq}, key={hex})"),
                || serde_json::json!({ "seq": seq, "key": hex }),
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
        // bole-k93a
        Cmd::Bundle { key, key_env, key_file } => {
            let k = match key {
                Some(h) => key::parse_hex_32(&h)?,
                None => signer_from(&key_env, key_file.as_deref())?.public_key(),
            };
            // bole-k93a: the owner's own hub view — a read-all accessor. A
            // served bundle (Grove, later) would pass the caller's accessor so
            // ACL-protected timelines are filtered per bole-e78l.
            let b = ctx.repo.profile_bundle(&k, &bole::Accessor::privileged()).await?;
            let profile_json = match &b.profile {
                Some(p) => serde_json::json!({
                    "key": key::hex32(&p.key),
                    "display_name": p.display_name,
                    "bio": p.bio,
                    "endpoints": p.endpoints,
                    "dns_aliases": p.dns_aliases,
                    "seq": p.seq,
                }),
                None => serde_json::Value::Null,
            };
            let edges_json: Vec<_> = b.edges.iter().map(|e| serde_json::json!({
                "to": key::hex32(&e.to_key),
                "kind": match e.kind {
                    bole::TrustKind::Follow => "follow",
                    bole::TrustKind::Vouch => "vouch",
                    bole::TrustKind::Review => "review",
                },
                "petname": e.petname,
                "seq": e.seq,
            })).collect();
            let timelines_json: Vec<_> = b.timelines.iter().map(|t| serde_json::json!({
                "name": t.name,
                "head": t.head.to_string(),
                "author": t.author,
                "created_at": t.created_at,
            })).collect();
            // bole-x23l
            let repos_json: Vec<_> = b.repos.iter().map(|r| serde_json::json!({
                "name": r.name, "description": r.description, "seq": r.seq,
            })).collect();
            let bundle_key = key::hex32(&b.key);
            let is_local = b.is_local;
            out.emit(
                || format!(
                    "{} [{}] {} repos, {} edges, {} timelines",
                    bundle_key,
                    if is_local { "local" } else { "peer" },
                    repos_json.len(),
                    edges_json.len(),
                    timelines_json.len(),
                ),
                || serde_json::json!({
                    "key": bundle_key,
                    "is_local": is_local,
                    "profile": profile_json,
                    "repos": repos_json,
                    "trust": { "edges": edges_json },
                    "timelines": timelines_json,
                }),
            );
            Ok(())
        }
    }
}
