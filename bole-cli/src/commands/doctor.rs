// bole-wphx
//! `bole doctor` — health-check a repo/hub store and report problems.
//!
//! Runs a set of checks and prints a report; exits non-zero if any check is an
//! error (so CI can gate on it). `--strict` also fails on warnings. Designed to
//! catch the leak class that bit the Grove hub: a private account seed committed
//! into a snapshot and published.

use anyhow::Result;
use bole::DiskWorkspace;

use crate::commands::env::ENVS_FILE;
use crate::commands::secret::SECRETS_FILE;
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

    // Check 1b — .boleignore coverage: warn if it's missing or doesn't cover
    // the usual secret footguns, so a stray key/env file can't slip in.
    const SECRET_GLOBS: &[&str] = &["*.key", "*.pem", "*.seed", "id_rsa", ".env"];
    let ignore_path = ctx.work_dir.join(bole::IGNORE_FILE);
    match std::fs::read_to_string(&ignore_path) {
        Err(_) => diags.push(Diagnostic::warn(
            "boleignore",
            "no .boleignore in the working tree",
            format!("create one covering secrets: bole ignore add {}", SECRET_GLOBS.join(" ")),
        )),
        Ok(body) => {
            let lines: Vec<&str> = body.lines().map(|l| l.trim()).collect();
            let missing: Vec<&str> = SECRET_GLOBS.iter().copied().filter(|g| !lines.contains(g)).collect();
            if missing.is_empty() {
                diags.push(Diagnostic::ok("boleignore", ".boleignore covers common secret patterns"));
            } else {
                diags.push(Diagnostic::warn(
                    "boleignore",
                    format!(".boleignore does not cover: {}", missing.join(", ")),
                    format!("bole ignore add {}", missing.join(" ")),
                ));
            }
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

    // Check 4 — object-graph health: dangling refs and broken closures (a
    // missing object in a timeline's history breaks fetch/materialize).
    let (dangling, broken) = ctx.repo.scan_object_health().await?;
    if dangling.is_empty() && broken.is_empty() {
        diags.push(Diagnostic::ok("object-closure", "all refs resolve to complete object closures"));
    } else {
        for r in &dangling {
            diags.push(Diagnostic::error(
                "object-closure",
                format!("ref '{}' points at a missing object", r.as_str()),
                "the target object is absent — restore it or delete the ref (bole ref delete)",
            ));
        }
        for (r, id) in &broken {
            diags.push(Diagnostic::error(
                "object-closure",
                format!("timeline '{}' is missing object {id} in its history", r.as_str()),
                "history is incomplete (fetch/materialize will fail) — re-fetch from a complete peer",
            ));
        }
    }

    // Check 5 — collab signatures: every published Profile/RepoRecord/TrustEdge
    // must verify. A failure means a tampered or corrupt signed object.
    let invalid = ctx.repo.scan_invalid_collab_signatures().await?;
    if invalid.is_empty() {
        diags.push(Diagnostic::ok("collab-signatures", "all published profiles, repos, and trust edges verify"));
    } else {
        for (r, kind) in &invalid {
            diags.push(Diagnostic::error(
                "collab-signatures",
                format!("{kind} at '{}' fails signature verification", r.as_str()),
                "a tampered/corrupt signed object — remove it (bole ref delete) and re-publish",
            ));
        }
    }

    // Check 6 — orphan repos: an announced repo with no pushed content, or
    // pushed content with no announced RepoRecord (a hub coherence issue).
    let (announced_empty, unannounced) = ctx.repo.scan_orphan_repos().await?;
    if announced_empty.is_empty() && unannounced.is_empty() {
        diags.push(Diagnostic::ok("orphan-repo", "every announced repo has content and vice versa"));
    } else {
        for (fp, name) in &announced_empty {
            diags.push(Diagnostic::warn(
                "orphan-repo",
                format!("repo '{name}' (owner {}…) is announced but has no pushed content", &fp[..fp.len().min(12)]),
                "push a timeline for it, or unannounce the RepoRecord",
            ));
        }
        for (fp, name) in &unannounced {
            diags.push(Diagnostic::warn(
                "orphan-repo",
                format!("timeline for '{name}' (owner {}…) has content but no announced RepoRecord", &fp[..fp.len().min(12)]),
                "announce it (bole repo announce) so it appears under the profile",
            ));
        }
    }

    // Check 7 — policy pin: if refs/policy/root exists it must load as a valid
    // PolicyObject::Root (policy_root is fail-closed on a malformed pin).
    match ctx.repo.policy_root().await {
        Ok(Some(_)) => diags.push(Diagnostic::ok("policy-pin", "policy root is pinned and intact")),
        Ok(None) => diags.push(Diagnostic::ok("policy-pin", "no policy root pinned")),
        Err(e) => diags.push(Diagnostic::error(
            "policy-pin",
            format!("refs/policy/root is malformed: {e}"),
            "repair or delete the pin (bole ref delete refs/policy/root) and re-adopt a valid policy",
        )),
    }

    // Check 8 — bound state: the CLI's current timeline/actor still exist.
    let state = ctx.load_state()?;
    let mut bound_ok = true;
    if let Some(tl) = &state.current_timeline {
        let missing = crate::resolve::ref_name(tl)
            .ok()
            .map(|rn| ctx.repo.refs.get_timeline(&rn).map(|t| t.is_none()).unwrap_or(true))
            .unwrap_or(true);
        if missing {
            bound_ok = false;
            diags.push(Diagnostic::warn(
                "bound-state",
                format!("current timeline '{tl}' no longer exists"),
                "bind a valid timeline (bole workspace open <timeline>) or clear it",
            ));
        }
    }
    if let Some(actor) = &state.current_actor {
        if !crate::actor::load(ctx)?.actors.contains_key(actor) {
            bound_ok = false;
            diags.push(Diagnostic::warn(
                "bound-state",
                format!("current actor '{actor}' no longer exists"),
                "bind a valid actor (bole actor use <name>) or clear it",
            ));
        }
    }
    if bound_ok {
        diags.push(Diagnostic::ok("bound-state", "bound timeline/actor are valid"));
    }

    // Check 9 — key-file permissions: seeds in ~/.bole/keys should be 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(home) = std::env::var_os("HOME") {
            let keys = std::path::Path::new(&home).join(".bole").join("keys");
            let mut loose = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&keys) {
                for e in rd.flatten() {
                    if e.path().extension().map(|x| x == "key").unwrap_or(false) {
                        if let Ok(meta) = e.metadata() {
                            if meta.permissions().mode() & 0o077 != 0 {
                                loose.push(e.file_name().to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
            if loose.is_empty() {
                diags.push(Diagnostic::ok("key-perms", "account seeds in ~/.bole/keys are owner-only (or none present)"));
            } else {
                for f in &loose {
                    diags.push(Diagnostic::warn(
                        "key-perms",
                        format!("~/.bole/keys/{f} is readable by others"),
                        format!("chmod 600 ~/.bole/keys/{f}"),
                    ));
                }
            }
        }
    }

    // Check 10 — gc opportunity (informational): reclaimable object count.
    let mut extra_roots = Vec::new();
    for file in [SECRETS_FILE, ENVS_FILE] {
        if let Ok(map) = crate::registry::load(ctx, file) {
            for id_str in map.values() {
                if let Ok(id) = id_str.parse::<bole::ObjectId>() {
                    extra_roots.push(id);
                }
            }
        }
    }
    let reclaimable = ctx.repo.unreachable_object_count(&extra_roots).await?;
    if reclaimable == 0 {
        diags.push(Diagnostic::ok("gc-opportunity", "no reclaimable objects"));
    } else {
        diags.push(Diagnostic::ok(
            "gc-opportunity",
            format!("{reclaimable} reclaimable object(s) — run `bole store gc` to reclaim space"),
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
