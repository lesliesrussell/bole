// bole-fo2
use crate::acl::Accessor;
// bole-fo2
use crate::error::{Error, Result};
// bole-fo2
use crate::acl::policy_object::HookSpec;
use crate::object::{Object, ObjectId};
use crate::refs::{RefName, RefStore, TimelinePolicy};
use crate::store::ObjectStore;
use std::collections::BTreeSet;

// bole-fo2
/// The outcome of a hook check. Hooks may only further restrict access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequiresApproval { reason: String, needed: u32 },
}

// bole-fo2
/// The decision point being evaluated.
pub enum PolicyEvent<'a> {
    Advance {
        timeline: &'a RefName,
        old_head: ObjectId,
        new_head: ObjectId,
    },
    Merge {
        source: &'a RefName,
        target: &'a RefName,
        old_head: ObjectId,
        result_head: ObjectId,
    },
}

// bole-fo2
/// The read-only context handed to a hook.
pub struct PolicyContext<'a> {
    pub event: PolicyEvent<'a>,
    pub accessor: &'a Accessor,
    pub objects: &'a ObjectStore,
    pub refs: &'a RefStore,
    // bole-7c1
    /// Wall-clock seconds. **Non-deterministic input**: a hook that branches on
    /// `now` is not replayable and must report [`PolicyHook::deterministic`] as
    /// `false`. See the determinism contract on [`PolicyHook`].
    pub now: u64,
}

// bole-fo2
/// A predicate evaluated at a write decision point, for rules the label lattice
/// cannot express. Hooks run after the label check passes and may only deny.
///
/// # Determinism contract (bole-7c1)
///
/// bole's distributed sync accepts a ref advance via compare-and-swap on heads.
/// If two replicas evaluate the *same* decision point but reach *different*
/// verdicts, they can accept divergent histories — the policy stops being a
/// global invariant. To prevent this, a hook's [`check`](PolicyHook::check)
/// **should be a pure function of**:
///
/// - the [`PolicyEvent`] (timeline name plus the old/new/result head ids), and
/// - the content-addressed object graph reachable from those ids
///   ([`PolicyContext::objects`] — identical on every replica by construction), and
/// - the hook's own configuration and any *replicated* data it carries.
///
/// It **must not** depend on wall-clock time ([`PolicyContext::now`]), the live
/// mutable ref store ([`PolicyContext::refs`], which races across replicas),
/// randomness, or process environment.
///
/// A hook that cannot honour this overrides [`deterministic`](PolicyHook::deterministic)
/// to return `false`. The replication path
/// ([`PolicyRegistry::evaluate_replayable`]) then refuses it fail-closed rather
/// than risk divergence; interactive local evaluation ([`PolicyRegistry::evaluate`])
/// still runs it.
#[async_trait::async_trait]
pub trait PolicyHook: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision;

    // bole-7c1
    /// Whether this hook's verdict is a pure, replayable function of the event
    /// and the content-addressed object graph (see the determinism contract on
    /// the trait). Defaults to `true`; hooks that consult wall-clock, live ref
    /// state, randomness, or environment must override this to `false`.
    fn deterministic(&self) -> bool {
        true
    }
}

// bole-fo2
/// Returns the more restrictive of two decisions (Deny > RequiresApproval > Allow).
fn more_restrictive(a: PolicyDecision, b: PolicyDecision) -> PolicyDecision {
    fn rank(d: &PolicyDecision) -> u8 {
        match d {
            PolicyDecision::Allow => 0,
            PolicyDecision::RequiresApproval { .. } => 1,
            PolicyDecision::Deny(_) => 2,
        }
    }
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

// bole-fo2
/// True if `ancestor` is `descendant` or an ancestor of it in the snapshot DAG.
async fn is_ancestor(
    objects: &ObjectStore,
    ancestor: ObjectId,
    descendant: ObjectId,
) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }
    let mut stack = vec![descendant];
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    while let Some(cur) = stack.pop() {
        if let Some(Object::Snapshot(s)) = objects.get(&cur).await? {
            for p in s.parents {
                if p == ancestor {
                    return Ok(true);
                }
                if seen.insert(p) {
                    stack.push(p);
                }
            }
        }
    }
    Ok(false)
}

// bole-fo2
/// Holds the bound hooks; the effective decision is the most restrictive across
/// all of them. Always preloads the built-in `TimelinePolicyHook`.
pub struct PolicyRegistry {
    hooks: Vec<Box<dyn PolicyHook>>,
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self { hooks: vec![Box::new(TimelinePolicyHook)] }
    }

    pub fn push(&mut self, hook: Box<dyn PolicyHook>) {
        self.hooks.push(hook);
    }

    pub async fn evaluate(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        let mut decision = PolicyDecision::Allow;
        for hook in &self.hooks {
            decision = more_restrictive(decision, hook.check(ctx).await);
        }
        decision
    }

    // bole-7c1
    /// True iff every bound hook honours the determinism contract (see
    /// [`PolicyHook`]). Only a deterministic registry can safely gate a
    /// replicated (CAS-on-heads) advance.
    pub fn deterministic(&self) -> bool {
        self.hooks.iter().all(|h| h.deterministic())
    }

    // bole-7c1
    /// The names of the bound hooks that report themselves non-deterministic.
    pub fn non_deterministic(&self) -> Vec<String> {
        self.hooks
            .iter()
            .filter(|h| !h.deterministic())
            .map(|h| h.name().to_string())
            .collect()
    }

    // bole-7c1
    /// Evaluate for a *replicated* decision point (e.g. accepting a pushed
    /// advance). Fail-closed: if any bound hook is non-deterministic, refuse the
    /// whole decision with a [`PolicyDecision::Deny`] naming the offenders,
    /// because a non-replayable verdict cannot be a global invariant. Otherwise
    /// this is exactly [`evaluate`](Self::evaluate).
    pub async fn evaluate_replayable(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        let offenders = self.non_deterministic();
        if !offenders.is_empty() {
            return PolicyDecision::Deny(format!(
                "non-deterministic policy hook(s) [{}] cannot gate a replicated advance (fail-closed); \
                 use replayable hooks (e.g. signed attestations) instead",
                offenders.join(", ")
            ));
        }
        self.evaluate(ctx).await
    }
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// bole-fo2
/// The built-in hook reproducing today's `TimelinePolicy` enforcement: on an
/// `Advance`, `FastForwardOnly`/`Append` require the new head to descend from the
/// old head; `Unrestricted` always allows. Non-advance events are allowed.
pub struct TimelinePolicyHook;

#[async_trait::async_trait]
impl PolicyHook for TimelinePolicyHook {
    fn name(&self) -> &str {
        "timeline-policy"
    }

    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        if let PolicyEvent::Advance { timeline, old_head, new_head } = &ctx.event {
            let tl = match ctx.refs.get_timeline(timeline) {
                Ok(Some(tl)) => tl,
                Ok(None) => {
                    return PolicyDecision::Deny(format!(
                        "timeline '{}' not found",
                        timeline.as_str()
                    ))
                }
                Err(e) => {
                    return PolicyDecision::Deny(format!("timeline lookup failed: {e}"))
                }
            };
            match tl.policy {
                TimelinePolicy::Unrestricted => PolicyDecision::Allow,
                TimelinePolicy::FastForwardOnly | TimelinePolicy::Append => {
                    match is_ancestor(ctx.objects, *old_head, *new_head).await {
                        Ok(true) => PolicyDecision::Allow,
                        Ok(false) => PolicyDecision::Deny(format!(
                            "timeline '{}' has policy {:?}; new head {} is not a descendant of current head {}",
                            timeline.as_str(),
                            tl.policy,
                            new_head,
                            old_head
                        )),
                        Err(e) => {
                            PolicyDecision::Deny(format!("ancestry check failed: {e}"))
                        }
                    }
                }
            }
        } else {
            PolicyDecision::Allow
        }
    }
}

// bole-6i7: the forgeable ref-counting `ApprovalHook` (and its
// `approval_ref_prefix` / `count_approvals` helpers) is removed. The only
// resolvable approval hook is now the signed, head-bound
// `crate::acl::attestation::SignedApprovalHook`.

// bole-fo2
/// Resolves a declarative `HookSpec` into a hook instance. Fail-closed: an
/// unknown `kind` is rejected rather than silently skipped (decision O5).
pub fn resolve_hook(spec: &HookSpec) -> Result<Box<dyn PolicyHook>> {
    match spec.kind.as_str() {
        // bole-6i7: signed, head-bound approvals loaded from the repo.
        "signed-approval" => {
            let needed = *spec.params.get("needed").unwrap_or(&1) as u32;
            Ok(Box::new(crate::acl::attestation::SignedApprovalHook {
                pattern: spec.pattern.clone(),
                needed,
            }))
        }
        "timeline-policy" => Ok(Box::new(TimelinePolicyHook)),
        other => Err(Error::PolicyViolation(format!(
            "unknown policy hook kind '{}' (fail-closed)",
            other
        ))),
    }
}

// bole-fo2
#[cfg(test)]
mod tests {
    use super::{PolicyContext, PolicyDecision, PolicyEvent, PolicyHook, PolicyRegistry, TimelinePolicyHook};
    use crate::acl::Accessor;
    use crate::object::Snapshot;
    use crate::refs::{RefName, RefStore, TimelinePolicy};
    use crate::refs::memory::MemoryRefBackend;
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    struct DenyAll;
    #[async_trait::async_trait]
    impl PolicyHook for DenyAll {
        fn name(&self) -> &str { "deny-all" }
        async fn check(&self, _ctx: &PolicyContext<'_>) -> PolicyDecision {
            PolicyDecision::Deny("nope".into())
        }
    }

    #[tokio::test]
    async fn most_restrictive_composition() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let accessor = Accessor::privileged();
        let ctx = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: base, new_head: base },
            accessor: &accessor,
            objects: &objects,
            refs: &refs,
            now: 0,
        };

        // Registry with only the built-in TimelinePolicyHook on an Unrestricted
        // timeline -> Allow.
        let reg = PolicyRegistry::new();
        assert_eq!(reg.evaluate(&ctx).await, PolicyDecision::Allow);

        // Add a DenyAll hook -> most restrictive wins -> Deny.
        let mut reg2 = PolicyRegistry::new();
        reg2.push(Box::new(DenyAll));
        assert!(matches!(reg2.evaluate(&ctx).await, PolicyDecision::Deny(_)));
    }

    #[tokio::test]
    async fn timeline_policy_hook_fast_forward() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let child = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into(),
        }).await.unwrap();
        let sibling = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 2, message: "s".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        refs.create_timeline(name.clone(), base, TimelinePolicy::FastForwardOnly, 0, "persistent".into(), None).unwrap();

        let accessor = Accessor::privileged();
        let hook = TimelinePolicyHook;

        // base -> child is a fast-forward -> Allow.
        let ctx_ff = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: base, new_head: child },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert_eq!(hook.check(&ctx_ff).await, PolicyDecision::Allow);

        // child -> sibling is NOT a fast-forward (sibling is not a descendant of child) -> Deny.
        // (timeline head is base here, but the hook tests old_head -> new_head ancestry.)
        let ctx_non = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: child, new_head: sibling },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert!(matches!(hook.check(&ctx_non).await, PolicyDecision::Deny(_)));
    }

    // bole-fo2
    // bole-7c1
    /// A hook that consults wall-clock `now` — the canonical non-deterministic
    /// hook. It declares itself non-deterministic so the replication guard can
    /// refuse it, even though its verdict here is Allow.
    struct NowGatedHook;
    #[async_trait::async_trait]
    impl PolicyHook for NowGatedHook {
        fn name(&self) -> &str { "now-gated" }
        fn deterministic(&self) -> bool { false }
        async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
            // Pretend "business hours only": depends on ambient time.
            if ctx.now.is_multiple_of(2) { PolicyDecision::Allow } else { PolicyDecision::Deny("after hours".into()) }
        }
    }

    // bole-7c1
    #[tokio::test]
    async fn builtins_have_expected_determinism() {
        use crate::acl::attestation::SignedApprovalHook;
        // The built-in FF hook is a pure function of the object DAG + config.
        assert!(TimelinePolicyHook.deterministic());
        // A fresh registry (only TimelinePolicyHook) is deterministic.
        assert!(PolicyRegistry::new().deterministic());
        // bole-6i7: the signed-approval hook loads attestations from the repo's
        // mutable ref namespace — non-deterministic across replicas.
        assert!(!SignedApprovalHook { pattern: "release/**".into(), needed: 1 }.deterministic());
    }

    // bole-7c1
    #[tokio::test]
    async fn registry_reports_non_deterministic_hooks() {
        let mut reg = PolicyRegistry::new();
        reg.push(Box::new(NowGatedHook));
        assert!(!reg.deterministic());
        assert_eq!(reg.non_deterministic(), vec!["now-gated".to_string()]);
    }

    // bole-7c1
    #[tokio::test]
    async fn evaluate_replayable_is_fail_closed() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let accessor = Accessor::privileged();
        // now is even, so NowGatedHook::check would return Allow — but the
        // replication path must refuse it anyway because it is non-deterministic.
        let ctx = PolicyContext {
            event: PolicyEvent::Advance { timeline: &name, old_head: base, new_head: base },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };

        // Deterministic registry: evaluate_replayable agrees with evaluate.
        let det = PolicyRegistry::new();
        assert_eq!(det.evaluate_replayable(&ctx).await, PolicyDecision::Allow);

        // Non-deterministic registry: fail-closed Deny naming the offending hook,
        // regardless of what the hook's own verdict would be.
        let mut nondet = PolicyRegistry::new();
        nondet.push(Box::new(NowGatedHook));
        match nondet.evaluate_replayable(&ctx).await {
            PolicyDecision::Deny(reason) => assert!(reason.contains("now-gated"), "reason: {reason}"),
            other => panic!("expected fail-closed Deny, got {other:?}"),
        }
    }

    // bole-6i7: the forgeable ApprovalHook was removed; signed-approval gating is
    // exercised in `acl::attestation` tests and the repo integration tests.
}
