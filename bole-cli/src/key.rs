// bole-1q9
//! Symmetric-key resolution for secrets.
//!
//! `put_secret`/`get_secret` take a raw 32-byte key. The CLI resolves that key
//! from a 64-character hex string supplied either through an environment
//! variable (default `BOLE_KEY`) or a key file. No key material is ever stored
//! in the repository.

use std::path::Path;

use anyhow::{anyhow, bail, Context as _, Result};
// bole-9mz
use bole::{LocalKeyProvider, ProviderChain};

// bole-9mz
/// Builds a `ProviderChain` from the same flags `resolve` reads. The resolved
/// 32-byte key serves as BOTH the active v2 master key (`LocalKeyProvider`) and
/// a legacy v1 raw key (so `env resolve` / `run` decrypt old single-key secrets
/// too). `ref_prefix` is `file` when `--key-file` is used, else `env`.
pub fn build_chain(key_env: &str, key_file: Option<&Path>) -> Result<ProviderChain> {
    let mk = resolve(key_env, key_file)?;
    let ref_prefix = if key_file.is_some() { "file" } else { "env" };
    let mut chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new(mk, ref_prefix)));
    chain.push_legacy_key(mk);
    Ok(chain)
}

/// Resolves a 32-byte key from `--key-file` if given, otherwise from the
/// environment variable named by `key_env`.
pub fn resolve(key_env: &str, key_file: Option<&Path>) -> Result<[u8; 32]> {
    let hex = match key_file {
        Some(p) => std::fs::read_to_string(p)
            .with_context(|| format!("reading key file {}", p.display()))?
            .trim()
            .to_string(),
        None => std::env::var(key_env).map_err(|_| {
            anyhow!("no key: set ${key_env} to a 64-hex key, or pass --key-file")
        })?,
    };
    parse_hex_32(hex.trim())
}

// bole-ehx: also used for approver Ed25519 seeds / public keys.
pub fn parse_hex_32(s: &str) -> Result<[u8; 32]> {
    if s.len() != 64 {
        bail!("key must be 64 hex characters (32 bytes), got {}", s.len());
    }
    let bytes = s.as_bytes();
    let mut key = [0u8; 32];
    for (i, slot) in key.iter_mut().enumerate() {
        let hi = nibble(bytes[i * 2]).ok_or_else(|| anyhow!("invalid hex in key"))?;
        let lo = nibble(bytes[i * 2 + 1]).ok_or_else(|| anyhow!("invalid hex in key"))?;
        *slot = (hi << 4) | lo;
    }
    Ok(key)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
