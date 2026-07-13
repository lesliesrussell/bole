// bole-wphx
//! `bole doctor` — health-check a repo/hub store and report problems.
//!
//! Runs a set of checks and prints a report; exits non-zero if any check is an
//! error (so CI can gate on it). `--strict` also fails on warnings. Designed to
//! catch the leak class that bit the Grove hub: a private account seed committed
//! into a snapshot and published.

use anyhow::Result;
use bole::DiskWorkspace;

use crate::context::RepoContext;
use crate::output::Output;

/// Severity of a diagnostic.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Severity {
    Ok,
    Warn,
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
    fn glyph(self) -> &'static str {
        match self {
            Severity::Ok => "✓",
            Severity::Warn => "⚠",
            Severity::Error => "✗",
        }
    }
}

/// One check's result.
struct Diagnostic {
    check: &'static str,
    severity: Severity,
    message: String,
    hint: Option<String>,
}

impl Diagnostic {
    fn ok(check: &'static str, message: impl Into<String>) -> Self {
        Self { check, severity: Severity::Ok, message: message.into(), hint: None }
    }
    fn warn(check: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { check, severity: Severity::Warn, message: message.into(), hint: Some(hint.into()) }
    }
    fn error(check: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { check, severity: Severity::Error, message: message.into(), hint: Some(hint.into()) }
    }
}

/// Runs `bole doctor`. `strict` promotes warnings to failures.
pub async fn run(ctx: &RepoContext, out: &Output, strict: bool) -> Result<()> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // Check 1 — committed seeds: any timeline head snapshot (incl. a hub's
    // refs/users/**) that contains a file looking like a private account seed.
    let committed = ctx.repo.scan_committed_seeds().await?;
    if committed.is_empty() {
        diags.push(Diagnostic::ok("committed-seed", "no private seeds committed to any timeline"));
    } else {
        for (tl, path) in &committed {
            diags.push(Diagnostic::error(
                "committed-seed",
                format!("a private seed is committed at '{path}' in timeline '{}'", tl.as_str()),
                "this key is compromised — rotate the account, remove the file, and re-snapshot",
            ));
        }
    }

    // Check 2 — working-tree seeds: a seed-like file the next snapshot would
    // capture (not covered by .boleignore).
    let ws = DiskWorkspace::new(&ctx.repo, &ctx.work_dir);
    let wt = ws.scan_seed_files().await?;
    let unignored: Vec<&String> = wt.iter().filter(|(_, ig)| !ig).map(|(p, _)| p).collect();
    if unignored.is_empty() {
        diags.push(Diagnostic::ok("worktree-seed", "no unignored seed files in the working tree"));
    } else {
        for p in &unignored {
            diags.push(Diagnostic::warn(
                "worktree-seed",
                format!("'{p}' looks like a private seed and would be captured by the next snapshot"),
                format!("move it to ~/.bole/keys, or: bole ignore add {p}"),
            ));
        }
    }

    // Check 3 — store integrity: every object decodes.
    let ids = ctx.repo.objects.list().await?;
    let mut bad = Vec::new();
    for id in &ids {
        if ctx.repo.objects.get(id).await.is_err() {
            bad.push(id.to_string());
        }
    }
    if bad.is_empty() {
        diags.push(Diagnostic::ok("store-integrity", format!("{} objects verified", ids.len())));
    } else {
        diags.push(Diagnostic::error(
            "store-integrity",
            format!("{} of {} objects failed to decode", bad.len(), ids.len()),
            "run `bole store fsck` for the list; the store may be corrupt",
        ));
    }

    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let warns = diags.iter().filter(|d| d.severity == Severity::Warn).count();

    out.emit(
        || {
            let mut s = String::new();
            for d in &diags {
                s.push_str(&format!("{} [{}] {}\n", d.severity.glyph(), d.check, d.message));
                if let Some(h) = &d.hint {
                    s.push_str(&format!("    → {h}\n"));
                }
            }
            s.push_str(&format!(
                "\n{} error(s), {} warning(s){}",
                errors,
                warns,
                if errors == 0 && warns == 0 { " — all clear" } else { "" }
            ));
            s
        },
        || {
            serde_json::json!({
                "errors": errors,
                "warnings": warns,
                "checks": diags.iter().map(|d| serde_json::json!({
                    "check": d.check,
                    "severity": d.severity.label(),
                    "message": d.message,
                    "hint": d.hint,
                })).collect::<Vec<_>>(),
            })
        },
    );

    // Non-zero exit gates CI: errors always fail; warnings fail under --strict.
    if errors > 0 || (strict && warns > 0) {
        std::process::exit(1);
    }
    Ok(())
}
