// bole-ehx
//! `bole policy` — configure content-gating policy hooks (signed approvals).
//!
//! Hook bindings are persisted as a JSON array of `HookSpec` in
//! `<store>/policy-hooks.json`. `RepoContext::discover` loads and registers them
//! on every invocation, so `advance`/`merge` enforce them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use bole::HookSpec;
use clap::Subcommand;

use crate::context::RepoContext;
use crate::output::Output;

/// File under the store holding the persisted `HookSpec` bindings.
pub const HOOKS_FILE: &str = "policy-hooks.json";

fn hooks_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join(HOOKS_FILE)
}

/// Loads the persisted hook bindings (empty if the file is absent).
pub fn load_hooks(repo_dir: &Path) -> Result<Vec<HookSpec>> {
    let p = hooks_path(repo_dir);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

fn save_hooks(repo_dir: &Path, hooks: &[HookSpec]) -> Result<()> {
    let p = hooks_path(repo_dir);
    let bytes = serde_json::to_vec_pretty(hooks)?;
    std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))
}

/// Policy subcommands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Require N distinct signed approvals to advance/merge into `pattern`.
    RequireApproval {
        /// Timeline glob pattern (e.g. `release/**`).
        pattern: String,
        /// Number of distinct approvals required.
        #[arg(long, default_value_t = 1)]
        needed: u64,
    },
    /// Remove the signed-approval requirement for `pattern`.
    Unrequire {
        pattern: String,
    },
    /// List configured policy hooks.
    List,
}

/// Dispatches a policy subcommand.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::RequireApproval { pattern, needed } => {
            let mut hooks = load_hooks(&ctx.repo_dir)?;
            // Replace any existing signed-approval binding for this exact pattern.
            hooks.retain(|h| !(h.kind == "signed-approval" && h.pattern == pattern));
            hooks.push(HookSpec {
                kind: "signed-approval".into(),
                pattern: pattern.clone(),
                params: BTreeMap::from([("needed".to_string(), needed)]),
            });
            save_hooks(&ctx.repo_dir, &hooks)?;
            out.emit(
                || format!("require {needed} signed approval(s) for {pattern}"),
                || serde_json::json!({ "action": "require-approval", "pattern": pattern, "needed": needed }),
            );
            Ok(())
        }
        Cmd::Unrequire { pattern } => {
            let mut hooks = load_hooks(&ctx.repo_dir)?;
            let before = hooks.len();
            hooks.retain(|h| !(h.kind == "signed-approval" && h.pattern == pattern));
            save_hooks(&ctx.repo_dir, &hooks)?;
            let removed = before - hooks.len();
            out.emit(
                || format!("removed {removed} approval requirement(s) for {pattern}"),
                || serde_json::json!({ "action": "unrequire", "pattern": pattern, "removed": removed }),
            );
            Ok(())
        }
        Cmd::List => {
            let hooks = load_hooks(&ctx.repo_dir)?;
            out.emit(
                || {
                    if hooks.is_empty() {
                        "no policy hooks".to_string()
                    } else {
                        hooks
                            .iter()
                            .map(|h| {
                                let needed = h.params.get("needed").copied().unwrap_or(0);
                                format!("{} {} (needed={})", h.kind, h.pattern, needed)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || {
                    serde_json::json!(hooks
                        .iter()
                        .map(|h| serde_json::json!({
                            "kind": h.kind,
                            "pattern": h.pattern,
                            "params": h.params,
                        }))
                        .collect::<Vec<_>>())
                },
            );
            Ok(())
        }
    }
}
