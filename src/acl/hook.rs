// bole-fo2
use crate::acl::Accessor;
// bole-fo2
use crate::error::{Error, Result};
// bole-fo2
use crate::acl::glob::glob_matches;
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
    pub now: u64,
}

// bole-fo2
/// A predicate evaluated at a write decision point, for rules the label lattice
/// cannot express. Hooks run after the label check passes and may only deny.
#[async_trait::async_trait]
pub trait PolicyHook: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision;
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

// bole-fo2
/// The ref-namespace prefix under which approval attestations for `target` live.
/// Placeholder storage (O4): real signed attestations are WS5's job.
pub fn approval_ref_prefix(target: &str) -> String {
    format!("refs/approval/{}/", target)
}

// bole-fo2
/// Counts recorded approval refs for `target`.
pub fn count_approvals(refs: &RefStore, target: &str) -> Result<u32> {
    let prefix = approval_ref_prefix(target);
    let n = refs.list(&prefix)?.len();
    Ok(n as u32)
}

// bole-fo2
/// "Merges into `<pattern>` need `needed` approvals." Checks `Merge` events whose
/// target matches `pattern`; blocks until enough approval refs exist.
pub struct ApprovalHook {
    pub pattern: String,
    pub needed: u32,
}

#[async_trait::async_trait]
impl PolicyHook for ApprovalHook {
    fn name(&self) -> &str {
        "approval"
    }

    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        if let PolicyEvent::Merge { target, .. } = &ctx.event {
            if glob_matches(&self.pattern, target.as_str()) {
                let approvals = match count_approvals(ctx.refs, target.as_str()) {
                    Ok(n) => n,
                    Err(e) => return PolicyDecision::Deny(format!("approval lookup failed: {e}")),
                };
                if approvals < self.needed {
                    return PolicyDecision::RequiresApproval {
                        reason: format!(
                            "{} needs {} approvals, has {}",
                            target.as_str(),
                            self.needed,
                            approvals
                        ),
                        needed: self.needed - approvals,
                    };
                }
            }
        }
        PolicyDecision::Allow
    }
}

// bole-fo2
/// Resolves a declarative `HookSpec` into a hook instance. Fail-closed: an
/// unknown `kind` is rejected rather than silently skipped (decision O5).
pub fn resolve_hook(spec: &HookSpec) -> Result<Box<dyn PolicyHook>> {
    match spec.kind.as_str() {
        "approval" => {
            let needed = *spec.params.get("needed").unwrap_or(&1) as u32;
            Ok(Box::new(ApprovalHook {
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
    #[tokio::test]
    async fn approval_hook_blocks_then_allows() {
        use super::{approval_ref_prefix, ApprovalHook};
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let source = RefName::new("feature/x").unwrap();
        let target = RefName::new("release/1.0").unwrap();
        let accessor = Accessor::privileged();
        let head = objects.put_tree(BTreeMap::new()).await.unwrap();

        let hook = ApprovalHook { pattern: "release/**".into(), needed: 1 };
        let ctx = PolicyContext {
            event: PolicyEvent::Merge { source: &source, target: &target, old_head: head, result_head: head },
            accessor: &accessor, objects: &objects, refs: &refs, now: 0,
        };
        assert!(matches!(hook.check(&ctx).await, PolicyDecision::RequiresApproval { .. }));

        let rn = RefName::new(format!("{}alice", approval_ref_prefix(target.as_str()))).unwrap();
        refs.create_tag(rn, head, None, 0).unwrap();
        assert_eq!(hook.check(&ctx).await, PolicyDecision::Allow);
    }
}
