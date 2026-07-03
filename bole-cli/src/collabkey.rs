// bole-6i1
//! Resolves the node's collaboration signing key (a 32-byte Ed25519 seed) from
//! `$BOLE_COLLAB_KEY` or `--key-file`. The seed never appears on argv.

use std::path::Path;

use anyhow::Result;
use bole::CollabSigner;

/// Builds a `CollabSigner` from the seed in `$key_env` or `--key-file`.
pub fn signer_from(key_env: &str, key_file: Option<&Path>) -> Result<CollabSigner> {
    let seed = crate::key::resolve(key_env, key_file)?;
    Ok(CollabSigner::from_seed(seed))
}
