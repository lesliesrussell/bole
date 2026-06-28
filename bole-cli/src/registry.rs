// bole-1q9
//! Generic name -> object-id registries.
//!
//! The library addresses secrets and env overlays by content hash, with no
//! notion of a human name. The CLI keeps small JSON maps under `.bole/` so
//! users can refer to them by name (`prod/db/url`, `dev`, ...).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result};

use crate::context::RepoContext;

fn path(ctx: &RepoContext, file: &str) -> PathBuf {
    ctx.repo_dir.join(file)
}

/// Loads a name -> id map, returning an empty map if the file is absent.
pub fn load(ctx: &RepoContext, file: &str) -> Result<BTreeMap<String, String>> {
    let p = path(ctx, file);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

/// Persists a name -> id map.
pub fn save(ctx: &RepoContext, file: &str, map: &BTreeMap<String, String>) -> Result<()> {
    let p = path(ctx, file);
    let bytes = serde_json::to_vec_pretty(map)?;
    std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))
}
