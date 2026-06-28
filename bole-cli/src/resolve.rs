// bole-w3a
//! Shared resolution of user-supplied references to `ObjectId`s, plus a clock.
//!
//! Accepted reference syntax:
//!
//! | Form          | Meaning                                          |
//! |---------------|--------------------------------------------------|
//! | `@`           | head of the currently-bound timeline             |
//! | `@<name>`     | head of timeline `<name>`                         |
//! | `@tag:<name>` | target of tag `<name>`                            |
//! | 64 hex chars  | that object id verbatim                           |
//! | `<name>`      | head of timeline `<name>` (or target if a tag)   |

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context as _, Result};
use bole::{Accessor, ObjectId, PathRole, Permission, Ref, RefName, TimelineRole};

use crate::context::{CliState, RepoContext};

/// A read+write accessor over all paths and timelines.
///
/// `Accessor::privileged()` is read-only by design, so the CLI uses this as
/// the default "root" identity for an unbound session. The actor command
/// group narrows this to a named actor's grants.
pub fn full_access() -> Accessor {
    Accessor::new()
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read })
        .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write })
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Read })
        .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
}

/// Current wall-clock time as Unix seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parses a string into a validated [`RefName`].
pub fn ref_name(s: &str) -> Result<RefName> {
    RefName::new(s).map_err(|e| anyhow!("invalid ref name '{s}': {e}"))
}

/// Resolves a snapshot reference to the `ObjectId` of a snapshot.
pub async fn snapshot(ctx: &RepoContext, state: &CliState, spec: &str) -> Result<ObjectId> {
    if spec == "@" {
        let name = state
            .current_timeline
            .as_deref()
            .ok_or_else(|| anyhow!("no timeline is bound; use `bole workspace open <timeline>` or name one explicitly"))?;
        return timeline_head(ctx, name).await;
    }
    if let Some(tag) = spec.strip_prefix("@tag:") {
        return tag_target(ctx, tag);
    }
    if let Some(name) = spec.strip_prefix('@') {
        return timeline_head(ctx, name).await;
    }
    if spec.len() == 64 && spec.bytes().all(|b| b.is_ascii_hexdigit()) {
        return spec
            .parse::<ObjectId>()
            .map_err(|e| anyhow!("invalid object id '{spec}': {e}"));
    }
    // Bare name: resolve as a ref (timeline head or tag target).
    let name = ref_name(spec)?;
    match ctx.repo.refs.get(&name).context("reading ref")? {
        Some(Ref::Timeline(t)) => Ok(t.head),
        Some(Ref::Tag(t)) => Ok(t.target),
        None => bail!("no such ref: {spec}"),
    }
}

/// Returns the head snapshot of a timeline by name.
pub async fn timeline_head(ctx: &RepoContext, name: &str) -> Result<ObjectId> {
    let rn = ref_name(name)?;
    ctx.repo
        .refs
        .get_timeline(&rn)
        .with_context(|| format!("reading timeline '{name}'"))?
        .map(|t| t.head)
        .ok_or_else(|| anyhow!("no such timeline: {name}"))
}

/// Returns the target snapshot of a tag by name.
pub fn tag_target(ctx: &RepoContext, name: &str) -> Result<ObjectId> {
    let rn = ref_name(name)?;
    ctx.repo
        .refs
        .get_tag(&rn)
        .with_context(|| format!("reading tag '{name}'"))?
        .map(|t| t.target)
        .ok_or_else(|| anyhow!("no such tag: {name}"))
}
