// bole-fz1
//! Signed merge/advance approvals — the real form of the placeholder approval
//! refs the WS1 `ApprovalHook` counted.
//!
//! An [`Attestation`] is an Ed25519 signature by an authorized approver over
//! `(target, head)` — "I approve advancing/merging `target` to `head`". Only
//! attestations by a key in the [`ApproverRegistry`] whose signature verifies
//! count, so a bare ref can no longer stand in for an approval, and an approval
//! is bound to the exact head it authorises (a later head needs fresh approvals).

use std::collections::BTreeSet;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::acl::glob::glob_matches;
use crate::acl::hook::{PolicyContext, PolicyDecision, PolicyEvent, PolicyHook};
use crate::acl::policy_object::PolicyObject;
use crate::error::Result;
use crate::object::{Object, ObjectId};
use crate::refs::{RefName, RefStore};
use crate::store::ObjectStore;

// bole-6i7
/// Well-known ref pinning the content-addressed [`ApproverRegistry`].
pub const APPROVERS_REF: &str = "refs/policy/approvers";
// bole-6i7
/// Prefix for per-attestation refs (`refs/attestations/<attestation-object-id>`).
pub const ATTESTATIONS_PREFIX: &str = "refs/attestations/";

// bole-6i7
/// Loads the approver registry pinned by [`APPROVERS_REF`], or an empty registry
/// if none is set. Both the persistence helpers and the repo-loading hook use it,
/// so approver state has a single source of truth.
pub async fn load_approvers(refs: &RefStore, objects: &ObjectStore) -> Result<ApproverRegistry> {
    let name = RefName::new(APPROVERS_REF)?;
    let tag = match refs.get_tag(&name)? {
        Some(t) => t,
        None => return Ok(ApproverRegistry::new()),
    };
    match objects.get(&tag.target).await? {
        Some(Object::Policy(PolicyObject::Approvers(reg))) => Ok(reg),
        _ => Ok(ApproverRegistry::new()),
    }
}

// bole-6i7
/// Loads every stored attestation (the refs under [`ATTESTATIONS_PREFIX`]).
pub async fn load_attestations(refs: &RefStore, objects: &ObjectStore) -> Result<Vec<Attestation>> {
    let mut out = Vec::new();
    for name in refs.list(ATTESTATIONS_PREFIX)? {
        if let Some(tag) = refs.get_tag(&name)? {
            if let Some(Object::Policy(PolicyObject::Attestation(att))) =
                objects.get(&tag.target).await?
            {
                out.push(att);
            }
        }
    }
    Ok(out)
}

// bole-fz1
/// An authorized approver's public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approver {
    pub key_id: String,
    pub public_key: [u8; 32],
}

// bole-fz1
/// The set of keys allowed to approve merges/advances for a repo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproverRegistry {
    pub approvers: Vec<Approver>,
}

impl ApproverRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, approver: Approver) {
        self.approvers.push(approver);
    }
    pub fn find(&self, key_id: &str) -> Option<&Approver> {
        self.approvers.iter().find(|a| a.key_id == key_id)
    }
}

// bole-fz1
/// A signed approval of advancing/merging `target` to `head`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    pub target: String,
    pub head: ObjectId,
    pub approver_key_id: String,
    /// Ed25519 signature (64 bytes) over `(target, head)`.
    pub sig: Vec<u8>,
}

// bole-fz1
/// Holds an approver's signing key and issues attestations.
pub struct AttestationSigner {
    key_id: String,
    signing: SigningKey,
}

impl AttestationSigner {
    pub fn from_seed(key_id: impl Into<String>, seed: [u8; 32]) -> Self {
        Self { key_id: key_id.into(), signing: SigningKey::from_bytes(&seed) }
    }
    /// The public [`Approver`] to register so this signer's approvals count.
    pub fn approver(&self) -> Approver {
        Approver { key_id: self.key_id.clone(), public_key: self.signing.verifying_key().to_bytes() }
    }
    /// Signs an approval of advancing `target` to `head`.
    pub fn attest(&self, target: impl Into<String>, head: ObjectId) -> Attestation {
        let target = target.into();
        let sig = self.signing.sign(&attestation_message(&target, head));
        Attestation { target, head, approver_key_id: self.key_id.clone(), sig: sig.to_bytes().to_vec() }
    }
}

// bole-fz1
// bole-m2p
/// Domain-separation tag for approval attestations (see the analogous tag in
/// `acl::authority`). Binds the signature to this scheme so a key reused across
/// bole's other Ed25519 schemes cannot cross-verify.
const ATTESTATION_DOMAIN: &[u8] = b"bole-attestation-v1\0";

/// The canonical signed message: domain tag, target bytes, a separator, head id.
fn attestation_message(target: &str, head: ObjectId) -> Vec<u8> {
    let mut m = Vec::with_capacity(ATTESTATION_DOMAIN.len() + target.len() + 33);
    m.extend_from_slice(ATTESTATION_DOMAIN);
    m.extend_from_slice(target.as_bytes());
    m.push(0);
    m.extend_from_slice(head.as_bytes());
    m
}

// bole-fz1
/// True if `att` is by a registered approver and its signature over
/// `(att.target, att.head)` verifies.
pub fn verify_attestation(att: &Attestation, registry: &ApproverRegistry) -> bool {
    let approver = match registry.find(&att.approver_key_id) {
        Some(a) => a,
        None => return false,
    };
    let vk = match VerifyingKey::from_bytes(&approver.public_key) {
        Ok(vk) => vk,
        Err(_) => return false,
    };
    let sig_bytes: [u8; 64] = match att.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    vk.verify(&attestation_message(&att.target, att.head), &signature).is_ok()
}

// bole-fz1
/// Counts distinct authorized approvers whose valid attestation approves
/// advancing `target` to `head`. Duplicate keys and invalid/wrong-head/unregistered
/// attestations do not count.
pub fn count_valid_approvals(
    attestations: &[Attestation],
    registry: &ApproverRegistry,
    target: &str,
    head: ObjectId,
) -> u32 {
    let mut approvers: BTreeSet<&str> = BTreeSet::new();
    for att in attestations {
        if att.target == target && att.head == head && verify_attestation(att, registry) {
            approvers.insert(att.approver_key_id.as_str());
        }
    }
    approvers.len() as u32
}

// bole-fz1
// bole-6i7
/// "Advances/merges into `<pattern>` need `needed` distinct signed approvals of
/// the exact head." The approver registry and attestations are loaded from the
/// repository at evaluation time ([`load_approvers`] / [`load_attestations`]), so
/// this hook is fully configured by `(pattern, needed)` and resolvable from a
/// `HookSpec` (kind `"signed-approval"`). Replaces the forgeable ref-counting
/// `ApprovalHook`.
pub struct SignedApprovalHook {
    pub pattern: String,
    pub needed: u32,
}

#[async_trait::async_trait]
impl PolicyHook for SignedApprovalHook {
    fn name(&self) -> &str {
        "signed-approval"
    }

    // bole-6i7
    /// **Non-deterministic.** The verdict counts attestations stored in the
    /// repository's mutable `refs/attestations/` namespace, which is not
    /// guaranteed identical across replicas (approvers add attestations over
    /// time), so two nodes can disagree. It therefore gates **local**
    /// advance/merge (via `Repository::advance_timeline` / `check_merge`); a
    /// replicated push into an approval-gated timeline is refused fail-closed by
    /// `apply_push_ops` (bole-7c1). Unlike the removed `ApprovalHook`, the
    /// approvals it counts are Ed25519-signed and head-bound, not forgeable refs.
    /// A deterministic variant would bind approvals into the head's object
    /// closure (an "approval commit"); tracked as future work.
    fn deterministic(&self) -> bool {
        false
    }

    async fn check(&self, ctx: &PolicyContext<'_>) -> PolicyDecision {
        // bole-rdh: gate BOTH merge and advance, bound to the head being moved to.
        let (target, head) = match &ctx.event {
            PolicyEvent::Merge { target, result_head, .. } => (*target, *result_head),
            PolicyEvent::Advance { timeline, new_head, .. } => (*timeline, *new_head),
        };
        if !glob_matches(&self.pattern, target.as_str()) {
            return PolicyDecision::Allow;
        }
        // bole-6i7: load the governing approver set + stored attestations from the
        // repo. Fail closed if either cannot be loaded.
        let approvers = match load_approvers(ctx.refs, ctx.objects).await {
            Ok(a) => a,
            Err(e) => return PolicyDecision::Deny(format!("approver load failed: {e}")),
        };
        let attestations = match load_attestations(ctx.refs, ctx.objects).await {
            Ok(a) => a,
            Err(e) => return PolicyDecision::Deny(format!("attestation load failed: {e}")),
        };
        let have = count_valid_approvals(&attestations, &approvers, target.as_str(), head);
        if have < self.needed {
            return PolicyDecision::RequiresApproval {
                reason: format!(
                    "{} needs {} signed approval(s) of head {}, has {}",
                    target.as_str(),
                    self.needed,
                    head,
                    have
                ),
                needed: self.needed - have,
            };
        }
        PolicyDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::hook::PolicyEvent;
    use crate::acl::Accessor;
    use crate::object::Snapshot;
    use crate::refs::memory::MemoryRefBackend;
    use crate::refs::{RefName, RefStore, TimelinePolicy};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    #[test]
    fn verify_and_count_distinct_valid_approvals() {
        let alice = AttestationSigner::from_seed("alice", [1u8; 32]);
        let bob = AttestationSigner::from_seed("bob", [2u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        reg.add(bob.approver());

        let head = ObjectId::from_content(b"result");
        let a1 = alice.attest("release/1.0", head);
        let b1 = bob.attest("release/1.0", head);

        assert!(verify_attestation(&a1, &reg));
        // Two distinct approvers → 2.
        assert_eq!(count_valid_approvals(&[a1.clone(), b1.clone()], &reg, "release/1.0", head), 2);
        // A duplicate from alice counts once.
        assert_eq!(count_valid_approvals(&[a1.clone(), a1.clone()], &reg, "release/1.0", head), 1);

        // An attestation over a DIFFERENT head does not count for this head.
        let other = ObjectId::from_content(b"other");
        let a_other = alice.attest("release/1.0", other);
        assert_eq!(count_valid_approvals(&[a_other], &reg, "release/1.0", head), 0);

        // bole-zqx: an attestation for a DIFFERENT target does not count for this
        // target (no cross-target replay).
        let a_other_target = alice.attest("release/2.0", head);
        assert_eq!(count_valid_approvals(&[a_other_target], &reg, "release/1.0", head), 0);

        // An unregistered approver does not count.
        let mallory = AttestationSigner::from_seed("mallory", [9u8; 32]);
        let m = mallory.attest("release/1.0", head);
        assert_eq!(count_valid_approvals(&[m], &reg, "release/1.0", head), 0);

        // A tampered signature does not verify.
        let mut forged = a1.clone();
        forged.head = other;
        assert!(!verify_attestation(&forged, &reg));
    }

    #[tokio::test]
    async fn hook_requires_then_allows_with_enough_signed_approvals() {
        let objects = ObjectStore::new(MemoryBackend::new());
        let refs = RefStore::new(MemoryRefBackend::new());
        let tree = objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
            .await
            .unwrap();
        let source = RefName::new("feature/x").unwrap();
        let target = RefName::new("release/1.0").unwrap();
        refs.create_timeline(target.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let alice = AttestationSigner::from_seed("alice", [3u8; 32]);
        let bob = AttestationSigner::from_seed("bob", [4u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        reg.add(bob.approver());

        let accessor = Accessor::privileged();
        let ctx = PolicyContext {
            event: PolicyEvent::Merge { source: &source, target: &target, old_head: base, result_head: base },
            accessor: &accessor,
            objects: &objects,
            refs: &refs,
            now: 0,
        };

        // bole-6i7: the hook loads approvers + attestations from the repo, so we
        // persist them via the same refs/objects scheme load_* reads.
        let reg_id = objects.put(&Object::Policy(PolicyObject::Approvers(reg))).await.unwrap();
        refs.create_tag(RefName::new(APPROVERS_REF).unwrap(), reg_id, None, 0).unwrap();

        let hook = SignedApprovalHook { pattern: "release/**".into(), needed: 2 };

        // No attestations yet → RequiresApproval.
        assert!(matches!(hook.check(&ctx).await, PolicyDecision::RequiresApproval { .. }));

        // Store two valid approvals of the result head.
        for att in [alice.attest("release/1.0", base), bob.attest("release/1.0", base)] {
            let id = objects.put(&Object::Policy(PolicyObject::Attestation(att))).await.unwrap();
            refs.create_tag(
                RefName::new(format!("{ATTESTATIONS_PREFIX}{id}")).unwrap(),
                id,
                None,
                0,
            )
            .unwrap();
        }

        // Now enough distinct signed approvals of the head → Allow.
        assert_eq!(hook.check(&ctx).await, PolicyDecision::Allow);
    }
}
