// bole-fo2
use crate::acl::clearance::ClearanceSet;
use crate::acl::lattice::LabelLattice;
use crate::acl::rules::LabelRuleSet;
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// bole-fo2
/// A declarative binding of a `PolicyHook` to a resource pattern, resolved by the
/// registry to a hook instance. Kept as data so policy stays content-addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub kind: String,
    pub pattern: String,
    pub params: BTreeMap<String, u64>,
}

// bole-fo2
/// The root tying a policy generation together; what a policy ref points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRoot {
    pub lattice: ObjectId,
    pub rules: ObjectId,
    pub parent: Option<ObjectId>,
    pub hooks: Vec<HookSpec>,
}

// bole-fo2
/// An issued, optionally-signed clearance credential for one actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceGrant {
    pub actor: String,
    pub clearances: ClearanceSet,
    pub signature: Option<Vec<u8>>,
}

// bole-fo2
/// Content-addressed policy payload. Each kind is independently addressed so a
/// replica can have/want lattice and ruleset separately during pack negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyObject {
    Lattice(LabelLattice),
    RuleSet(LabelRuleSet),
    Grant(ClearanceGrant),
    Root(PolicyRoot),
    // bole-6i7
    /// The content-addressed set of keys allowed to sign approvals, pinned by the
    /// `refs/policy/approvers` ref.
    Approvers(crate::acl::attestation::ApproverRegistry),
    // bole-6i7
    /// A single stored, head-bound signed approval, pinned by a
    /// `refs/attestations/<id>` ref.
    Attestation(crate::acl::attestation::Attestation),
}

// bole-fo2
#[cfg(test)]
mod tests {
    use super::{ClearanceGrant, HookSpec, PolicyObject, PolicyRoot};
    use crate::acl::clearance::{Capability, Clearance, ClearanceSet};
    use crate::acl::lattice::{Label, LabelLattice};
    use crate::acl::rules::{LabelRule, LabelRuleSet};
    use crate::object::{Object, ObjectId};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    async fn round_trip(store: &ObjectStore, obj: PolicyObject) -> (ObjectId, ObjectId) {
        let wrapped = Object::Policy(obj);
        let id1 = store.put(&wrapped).await.unwrap();
        let got = store.get(&id1).await.unwrap().unwrap();
        assert_eq!(got, wrapped);
        let id2 = store.put(&wrapped).await.unwrap();
        (id1, id2)
    }

    #[tokio::test]
    async fn policy_objects_round_trip_with_stable_ids() {
        let store = ObjectStore::new(MemoryBackend::new());

        let lattice = PolicyObject::Lattice(LabelLattice::two_point());
        let ruleset = PolicyObject::RuleSet(LabelRuleSet {
            rules: vec![LabelRule::Path { glob: "secrets/**".into(), label: Label::protected() }],
        });
        let grant = PolicyObject::Grant(ClearanceGrant {
            actor: "leslie".into(),
            clearances: ClearanceSet {
                clearances: vec![Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::READ,
                    scope: None,
                }],
                confined: false,
            },
            signature: None,
        });
        let root = PolicyObject::Root(PolicyRoot {
            lattice: ObjectId::new([1u8; 32]),
            rules: ObjectId::new([2u8; 32]),
            parent: None,
            hooks: vec![HookSpec {
                kind: "approval".into(),
                pattern: "release/**".into(),
                params: BTreeMap::from([("needed".to_string(), 2u64)]),
            }],
        });

        for obj in [lattice, ruleset, grant, root] {
            let (a, b) = round_trip(&store, obj).await;
            assert_eq!(a, b, "identical content must yield identical ObjectId");
        }
    }
}
