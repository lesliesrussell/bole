// bole-eean
//! CLI audit sink: appends one JSON line per audited access decision to the
//! file named by `$BOLE_AUDIT_LOG`, giving a deployer a durable, greppable
//! trail of agent-initiated timeline transitions and how they were decided.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use bole::{AuditDecision, AuditEvent, AuditSink, Repository};

/// Env var naming the append-only audit log file. Unset → no audit sink.
pub const AUDIT_LOG_ENV: &str = "BOLE_AUDIT_LOG";

/// Appends JSON-line audit records to a file. Failures are swallowed (audit is
/// best-effort and must never break the operation being audited), but the
/// handle is opened up front so a bad path surfaces at install time.
struct FileAuditSink {
    file: Mutex<std::fs::File>,
}

impl AuditSink for FileAuditSink {
    fn record(&self, event: &AuditEvent) {
        let line = render(event);
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

// bole-eean
/// Installs the file audit sink on `repo` when `$BOLE_AUDIT_LOG` is set;
/// otherwise returns `repo` unchanged. A set-but-unopenable path is an error so
/// misconfigured auditing fails loudly at startup rather than silently dropping
/// records.
pub fn install(repo: Repository) -> anyhow::Result<Repository> {
    let path = match std::env::var_os(AUDIT_LOG_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return Ok(repo),
    };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| anyhow::anyhow!("opening {} ({}): {e}", AUDIT_LOG_ENV, path.display()))?;
    Ok(repo.with_audit_sink(std::sync::Arc::new(FileAuditSink { file: Mutex::new(file) })))
}

/// Renders an audit event as a single compact JSON object.
fn render(event: &AuditEvent) -> String {
    match event {
        AuditEvent::TimelineAdvance { timeline, actor, old_head, new_head, decision } => {
            let (decision_str, detail) = match decision {
                AuditDecision::Allowed => ("allowed".to_string(), String::new()),
                AuditDecision::Denied { reason } => {
                    ("denied".to_string(), format!(",\"reason\":{}", json_str(reason)))
                }
                AuditDecision::ApprovalRequired { needed, reason } => (
                    "approval_required".to_string(),
                    format!(",\"needed\":{needed},\"reason\":{}", json_str(reason)),
                ),
            };
            let actor_field = match actor {
                Some(a) => json_str(a),
                None => "null".to_string(),
            };
            format!(
                "{{\"event\":\"timeline_advance\",\"timeline\":{},\"actor\":{},\"old_head\":\"{}\",\"new_head\":\"{}\",\"decision\":\"{}\"{}}}",
                json_str(timeline),
                actor_field,
                old_head,
                new_head,
                decision_str,
                detail,
            )
        }
        // AuditEvent is #[non_exhaustive]; a future kind renders as a stub
        // rather than being silently dropped.
        _ => "{\"event\":\"unknown\"}".to_string(),
    }
}

/// Minimal JSON string escaping for the fields we emit.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
