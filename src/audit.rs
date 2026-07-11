// bole-eean
//! Access-control audit trail.
//!
//! bole enforces access decisions at the API boundary; this module lets a
//! deployer *observe* them. A [`Repository`](crate::Repository) can be given an
//! [`AuditSink`]; the repository then emits an [`AuditEvent`] for each
//! agent-initiated timeline advance whenever an access or policy check decides
//! it — allowed after the head moves, denied on an ACL or policy refusal, or
//! approval-required — so security-relevant decisions are attributable after
//! the fact. (An advance that fails on a non-decision — an absent timeline or
//! snapshot, or a lost compare-and-swap race — is not recorded.)
//!
//! The sink is a trait rather than a fixed logger so a deployer chooses the
//! destination (a file, `tracing`, a SIEM); the library takes no logging
//! dependency. Emission is best-effort and side-effect-only: it never changes
//! whether an operation is allowed, and a sink must not panic.

use crate::object::ObjectId;

// bole-eean
/// The outcome of an access-control decision, mirroring the enforcement
/// verdicts a caller sees (`Ok`, `Error::AccessDenied`/`PolicyViolation`,
/// `Error::ApprovalRequired`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditDecision {
    /// The operation was permitted and applied.
    Allowed,
    /// The operation was refused. `reason` is the enforcement message.
    Denied { reason: String },
    /// The operation is gated on `needed` further approvals.
    ApprovalRequired { needed: u32, reason: String },
}

// bole-eean
/// A single audited action. Today the trail covers timeline advancement (the
/// primary agent-initiated, policy-gated write); the enum is non-exhaustive so
/// further event kinds (merges, secret reveals) can be added without breaking
/// sinks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditEvent {
    /// An attempt to advance a timeline's head, and how it was decided.
    TimelineAdvance {
        /// The timeline ref name.
        timeline: String,
        /// The actor label the request was made under (`None` when the
        /// accessor carries no actor identity, e.g. a privileged internal op).
        actor: Option<String>,
        /// The head the timeline was at when the decision was made.
        old_head: ObjectId,
        /// The head the advance targeted.
        new_head: ObjectId,
        /// The decision.
        decision: AuditDecision,
    },
}

// bole-eean
/// A destination for [`AuditEvent`]s. Implementations must be cheap, must not
/// panic, and must not block the calling operation for long — audit emission is
/// on the hot path of every advance.
pub trait AuditSink: Send + Sync {
    /// Record one event. Called synchronously from the audited operation.
    fn record(&self, event: &AuditEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A sink that collects events for assertions.
    #[derive(Default)]
    pub struct CollectingSink {
        pub events: Mutex<Vec<AuditEvent>>,
    }
    impl AuditSink for CollectingSink {
        fn record(&self, event: &AuditEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn collecting_sink_records() {
        let sink = CollectingSink::default();
        let ev = AuditEvent::TimelineAdvance {
            timeline: "main".into(),
            actor: Some("alice".into()),
            old_head: ObjectId::from_content(b"a"),
            new_head: ObjectId::from_content(b"b"),
            decision: AuditDecision::Allowed,
        };
        sink.record(&ev);
        assert_eq!(sink.events.lock().unwrap().as_slice(), std::slice::from_ref(&ev));
    }
}
