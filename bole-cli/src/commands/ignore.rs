// bole-phxz
//! `bole ignore` — manage the workspace `.boleignore` file.
//!
//! Patterns use gitignore semantics and are enforced by the snapshot disk-walk
//! (`DiskWorkspace::collect`). The file is a normal tracked file at the work
//! tree root; this command is just a convenient editor and dry-run tester.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use bole::IGNORE_FILE;
use clap::{Args, Subcommand};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::context::RepoContext;
use crate::output::Output;

/// `bole ignore` — add/list/remove/check ignore patterns.
///
/// Bare patterns (`bole ignore "*.log" target/`) are sugar for `add`.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: Option<Sub>,
    /// Pattern(s) to add when no subcommand is given (gitignore syntax).
    #[arg(value_name = "PATTERN")]
    patterns: Vec<String>,
}

#[derive(Subcommand)]
enum Sub {
    /// Add pattern(s) to `.boleignore` (created if absent; duplicates skipped).
    Add {
        #[arg(required = true, value_name = "PATTERN")]
        patterns: Vec<String>,
    },
    /// List the active ignore patterns.
    List,
    /// Remove pattern(s) from `.boleignore` (exact line match).
    Remove {
        #[arg(required = true, value_name = "PATTERN")]
        patterns: Vec<String>,
    },
    /// Test whether path(s) would be ignored, and by which pattern.
    Check {
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<String>,
    },
}

/// Dispatches an `ignore` invocation.
pub async fn run(ctx: &RepoContext, out: &Output, cmd: Cmd) -> Result<()> {
    match cmd.sub {
        Some(Sub::Add { patterns }) => add(ctx, out, patterns),
        Some(Sub::List) => list(ctx, out),
        Some(Sub::Remove { patterns }) => remove(ctx, out, patterns),
        Some(Sub::Check { paths }) => check(ctx, out, paths),
        None => {
            if cmd.patterns.is_empty() {
                bail!("no patterns given; try `bole ignore <pattern>...` or `bole ignore list`");
            }
            add(ctx, out, cmd.patterns)
        }
    }
}

/// Path to the work-tree `.boleignore`.
fn ignore_path(ctx: &RepoContext) -> PathBuf {
    ctx.work_dir.join(IGNORE_FILE)
}

/// Reads the file's lines (empty if absent).
fn read_lines(path: &Path) -> Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The active patterns: non-blank, non-comment trimmed lines.
fn active_patterns(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Validates a single pattern by feeding it to a throwaway builder.
fn validate(pattern: &str) -> Result<()> {
    let mut b = GitignoreBuilder::new("");
    b.add_line(None, pattern)
        .with_context(|| format!("invalid ignore pattern: {pattern}"))?;
    Ok(())
}

fn add(ctx: &RepoContext, out: &Output, patterns: Vec<String>) -> Result<()> {
    let path = ignore_path(ctx);
    let mut lines = read_lines(&path)?;
    let existing = active_patterns(&lines);

    let mut added = Vec::new();
    let mut skipped = Vec::new();
    for pat in patterns {
        let pat = pat.trim().to_string();
        if pat.is_empty() {
            continue;
        }
        validate(&pat)?;
        if existing.contains(&pat) || added.contains(&pat) {
            skipped.push(pat);
        } else {
            lines.push(pat.clone());
            added.push(pat);
        }
    }

    if !added.is_empty() {
        // Preserve one-per-line with a trailing newline.
        let mut body = lines.join("\n");
        body.push('\n');
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    }

    out.emit(
        || {
            let mut msg = String::new();
            if added.is_empty() {
                msg.push_str("nothing added");
            } else {
                msg.push_str(&format!("added {}: {}", added.len(), added.join(", ")));
            }
            if !skipped.is_empty() {
                msg.push_str(&format!("\nskipped {} already present: {}", skipped.len(), skipped.join(", ")));
            }
            msg
        },
        || serde_json::json!({ "added": added, "skipped": skipped }),
    );
    Ok(())
}

fn list(ctx: &RepoContext, out: &Output) -> Result<()> {
    let path = ignore_path(ctx);
    let patterns = active_patterns(&read_lines(&path)?);
    out.emit(
        || {
            if patterns.is_empty() {
                "no ignore patterns".to_string()
            } else {
                patterns.join("\n")
            }
        },
        || serde_json::json!({ "patterns": patterns }),
    );
    Ok(())
}

fn remove(ctx: &RepoContext, out: &Output, patterns: Vec<String>) -> Result<()> {
    let path = ignore_path(ctx);
    let lines = read_lines(&path)?;
    let targets: Vec<String> = patterns.iter().map(|p| p.trim().to_string()).collect();

    let mut removed = Vec::new();
    let kept: Vec<String> = lines
        .into_iter()
        .filter(|line| {
            let trimmed = line.trim();
            if targets.iter().any(|t| t == trimmed) {
                removed.push(trimmed.to_string());
                false
            } else {
                true
            }
        })
        .collect();

    if !removed.is_empty() {
        if kept.iter().all(|l| l.trim().is_empty()) {
            // Nothing meaningful left; drop the file entirely.
            let _ = std::fs::remove_file(&path);
        } else {
            let mut body = kept.join("\n");
            body.push('\n');
            std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        }
    }

    let not_found: Vec<String> = targets.iter().filter(|t| !removed.contains(t)).cloned().collect();
    out.emit(
        || {
            let mut msg = if removed.is_empty() {
                "nothing removed".to_string()
            } else {
                format!("removed {}: {}", removed.len(), removed.join(", "))
            };
            if !not_found.is_empty() {
                msg.push_str(&format!("\nnot present: {}", not_found.join(", ")));
            }
            msg
        },
        || serde_json::json!({ "removed": removed, "not_found": not_found }),
    );
    Ok(())
}

fn check(ctx: &RepoContext, out: &Output, paths: Vec<String>) -> Result<()> {
    let matcher = build_matcher(ctx)?;
    let root = &ctx.work_dir;

    let mut results = Vec::new();
    for rel in paths {
        // bole-0tou: a trailing slash denotes a directory query but breaks path
        // joining/matching — normalize it away and force directory semantics.
        let trimmed = rel.trim_end_matches('/');
        let abs = root.join(trimmed);
        let is_dir = rel.ends_with('/') || abs.is_dir();
        // bole-0tou: model the walk's parent-directory pruning — a path is
        // ignored if it *or any ancestor directory* matches. Plain `matched`
        // only tests the exact path, so files under an ignored dir (whose
        // subtree the walk never descends into) looked "not ignored".
        let m = matcher.matched_path_or_any_parents(&abs, is_dir);
        let (ignored, pattern) = if m.is_ignore() {
            (true, m.inner().map(|g| g.original().to_string()))
        } else {
            (false, None)
        };
        results.push((rel, ignored, pattern));
    }

    out.emit(
        || {
            results
                .iter()
                .map(|(rel, ignored, pattern)| {
                    if *ignored {
                        let by = pattern.as_deref().unwrap_or("?");
                        format!("{rel}: ignored (by `{by}`)")
                    } else {
                        format!("{rel}: not ignored")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        || {
            serde_json::json!({
                "results": results
                    .iter()
                    .map(|(rel, ignored, pattern)| serde_json::json!({
                        "path": rel,
                        "ignored": ignored,
                        "pattern": pattern,
                    }))
                    .collect::<Vec<_>>()
            })
        },
    );
    Ok(())
}

/// Builds the same matcher the snapshot walk uses, rooted at the work tree.
fn build_matcher(ctx: &RepoContext) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(&ctx.work_dir);
    let _ = builder.add(ignore_path(ctx));
    builder.build().context("building ignore matcher")
}
