// bole-ef8
//! Named-actor registry.
//!
//! The library's `Accessor` is an in-memory credential built from path and
//! timeline roles. To make actors reusable from the command line, the CLI
//! persists their grants in `.bole/actors.json` and rebuilds an `Accessor`
//! on demand. The actor bound in CLI state becomes the identity used for
//! access-controlled operations; with no actor bound the CLI uses full access.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context as _, Result};
use bole::{Accessor, PathRole, TimelineRole};
use serde::{Deserialize, Serialize};

use crate::context::RepoContext;
use crate::resolve;

/// The grants held by one named actor.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ActorDef {
    #[serde(default)]
    pub path_roles: Vec<PathRole>,
    #[serde(default)]
    pub timeline_roles: Vec<TimelineRole>,
}

impl ActorDef {
    /// Builds a runtime [`Accessor`] from these grants.
    pub fn to_accessor(&self) -> Accessor {
        let mut acc = Accessor::new();
        for r in &self.path_roles {
            acc = acc.with_path_role(r.clone());
        }
        for r in &self.timeline_roles {
            acc = acc.with_timeline_role(r.clone());
        }
        acc
    }
}

/// The full actor registry: name -> grants.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub actors: BTreeMap<String, ActorDef>,
}

fn registry_path(ctx: &RepoContext) -> PathBuf {
    ctx.repo_dir.join("actors.json")
}

/// Loads the registry, returning an empty one if the file is absent.
pub fn load(ctx: &RepoContext) -> Result<Registry> {
    let path = registry_path(ctx);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Persists the registry.
pub fn save(ctx: &RepoContext, registry: &Registry) -> Result<()> {
    let path = registry_path(ctx);
    let bytes = serde_json::to_vec_pretty(registry)?;
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// Looks up one actor by name.
pub fn get(ctx: &RepoContext, name: &str) -> Result<ActorDef> {
    load(ctx)?
        .actors
        .remove(name)
        .ok_or_else(|| anyhow!("no such actor: {name}"))
}

/// Returns the accessor for the actor bound in CLI state, or full access when
/// none is bound.
pub fn effective_accessor(ctx: &RepoContext) -> Result<Accessor> {
    let state = ctx.load_state()?;
    match state.current_actor {
        Some(name) => Ok(get(ctx, &name)
            .with_context(|| format!("bound actor '{name}' is not in the registry"))?
            .to_accessor()),
        None => Ok(resolve::full_access()),
    }
}

/// Marks `name` as the current actor in CLI state, erroring if unknown.
pub fn bind(ctx: &RepoContext, name: &str) -> Result<()> {
    if !load(ctx)?.actors.contains_key(name) {
        bail!("no such actor: {name}");
    }
    let mut state = ctx.load_state()?;
    state.current_actor = Some(name.to_string());
    ctx.save_state(&state)?;
    Ok(())
}
