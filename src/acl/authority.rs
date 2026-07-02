// bole-0tp
//! Policy authority: signed, content-addressed policy roots verified against a
//! per-repo trusted key set (WS5 §5 "highest-rooted-wins").
//!
//! Policy is already content-addressed (`Object::Policy`, WS1). This module adds
//! the missing verb: verify an offered `PolicyRoot` chain to a `TrustAnchor`-signed
//! root, fail-closed on anything unsigned/untrusted/unresolvable, and resolve two
//! candidate policies by preferring the strictly-longer verified lineage.
//!
//! Signatures are **detached** (over the root's `ObjectId`): a content-addressed
//! object cannot embed a signature of itself without a hash cycle, so root
//! signatures live in a [`SignatureStore`] keyed by the root id.

use std::collections::HashMap;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::acl::hook::resolve_hook;
use crate::acl::policy_object::PolicyObject;
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::store::ObjectStore;

// bole-0tp
/// A trusted signing key: the anchor a root's signature must verify under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustAnchor {
    pub key_id: String,
    /// Ed25519 public key bytes.
    pub public_key: [u8; 32],
}

// bole-0tp
/// The per-repo set of trusted signing keys.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    pub anchors: Vec<TrustAnchor>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, anchor: TrustAnchor) {
        self.anchors.push(anchor);
    }
    pub fn find(&self, key_id: &str) -> Option<&TrustAnchor> {
        self.anchors.iter().find(|a| a.key_id == key_id)
    }
}

// bole-0tp
/// A detached signature over a `PolicyRoot`'s `ObjectId`, by a named key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootSignature {
    pub root: ObjectId,
    pub key_id: String,
    /// Ed25519 signature bytes (64).
    pub sig: Vec<u8>,
}

// bole-0tp
/// Detached root signatures, keyed by the signed root id.
#[derive(Debug, Clone, Default)]
pub struct SignatureStore {
    by_root: HashMap<ObjectId, RootSignature>,
}

impl SignatureStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, sig: RootSignature) {
        self.by_root.insert(sig.root, sig);
    }
    pub fn get(&self, root: &ObjectId) -> Option<&RootSignature> {
        self.by_root.get(root)
    }
}

// bole-0tp
/// Holds an Ed25519 signing key for a policy admin. The signing key never leaves
/// this type; only the public anchor and detached signatures are exported.
pub struct PolicySigner {
    key_id: String,
    signing: SigningKey,
}

impl PolicySigner {
    /// Builds a signer from a 32-byte seed (deterministic — the operator holds
    /// the seed out of band; a KMS-backed signer can replace this later).
    pub fn from_seed(key_id: impl Into<String>, seed: [u8; 32]) -> Self {
        Self { key_id: key_id.into(), signing: SigningKey::from_bytes(&seed) }
    }

    /// The public [`TrustAnchor`] a verifier pins to trust this signer.
    pub fn anchor(&self) -> TrustAnchor {
        TrustAnchor {
            key_id: self.key_id.clone(),
            public_key: self.signing.verifying_key().to_bytes(),
        }
    }

    /// A detached signature over `root`'s domain-separated message.
    pub fn sign_root(&self, root: ObjectId) -> RootSignature {
        let sig = self.signing.sign(&policy_root_message(root));
        RootSignature { root, key_id: self.key_id.clone(), sig: sig.to_bytes().to_vec() }
    }
}

// bole-m2p
/// Domain-separation tag for policy-root signatures. Prefixing the signed bytes
/// with a per-scheme constant prevents a signature made in one context (or by a
/// key reused across bole's other Ed25519 schemes — attestations, ref-ops) from
/// verifying as a policy-root signature. Without it, sign_root signed the bare
/// 32-byte id, so cross-scheme reuse was prevented only by incidental length
/// differences. Versioned so the format can evolve.
const POLICY_ROOT_DOMAIN: &[u8] = b"bole-policy-root-v1\0";

// bole-m2p
/// The domain-separated message signed for a policy root: tag || id.
fn policy_root_message(root: ObjectId) -> Vec<u8> {
    let mut m = Vec::with_capacity(POLICY_ROOT_DOMAIN.len() + 32);
    m.extend_from_slice(POLICY_ROOT_DOMAIN);
    m.extend_from_slice(root.as_bytes());
    m
}

// bole-0tp
/// The result of verifying an offered policy root chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyVerdict {
    /// The chain verified; `depth` is its length (tip → trusted root).
    Accept { depth: u64 },
    /// Rejected (unsigned / untrusted key / unknown hook / broken chain).
    Reject(String),
}

// bole-0tp
/// Verifies that `sig` over `root` is valid under some anchor in `trust`.
fn verify_root_signature(root: ObjectId, sig: &RootSignature, trust: &TrustStore) -> bool {
    if sig.root != root {
        return false;
    }
    let anchor = match trust.find(&sig.key_id) {
        Some(a) => a,
        None => return false,
    };
    let vk = match VerifyingKey::from_bytes(&anchor.public_key) {
        Ok(vk) => vk,
        Err(_) => return false,
    };
    let sig_bytes: [u8; 64] = match sig.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    // bole-m2p: verify over the domain-separated message, matching sign_root.
    vk.verify(&policy_root_message(root), &signature).is_ok()
}

// bole-0tp
/// Loads the `PolicyRoot` at `id`, or `None` if absent / not a policy root.
async fn load_root(
    objects: &ObjectStore,
    id: &ObjectId,
) -> Result<Option<crate::acl::policy_object::PolicyRoot>> {
    match objects.get(id).await? {
        Some(Object::Policy(PolicyObject::Root(r))) => Ok(Some(r)),
        _ => Ok(None),
    }
}

// bole-0tp
/// Walks the `parent` chain from `tip`, returning the root ids tip→root0.
/// Errors if any root is missing or the chain has a cycle.
pub async fn chain_of(objects: &ObjectStore, tip: ObjectId) -> Result<Vec<ObjectId>> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = Some(tip);
    while let Some(id) = cur {
        if !seen.insert(id) {
            return Err(Error::PolicyViolation("policy chain has a cycle".into()));
        }
        let root = load_root(objects, &id)
            .await?
            .ok_or_else(|| Error::PolicyViolation(format!("policy root not found: {id}")))?;
        chain.push(id);
        cur = root.parent;
    }
    Ok(chain)
}

// bole-0tp
/// Verifies an offered `PolicyRoot` chain: every root in the chain must be
/// present, its `lattice`/`rules` objects present, every `HookSpec.kind`
/// resolvable (fail-closed), and (v1 direct-anchor rule) each root
/// `TrustAnchor`-signed. Returns `Accept { depth }` or a `Reject` reason.
pub async fn verify_chain(
    objects: &ObjectStore,
    tip: ObjectId,
    sigs: &SignatureStore,
    trust: &TrustStore,
) -> Result<PolicyVerdict> {
    let chain = match chain_of(objects, tip).await {
        Ok(c) => c,
        Err(Error::PolicyViolation(reason)) => return Ok(PolicyVerdict::Reject(reason)),
        Err(e) => return Err(e),
    };
    for id in &chain {
        let root = load_root(objects, id).await?.expect("chain_of validated presence");
        // Referenced lattice/rules objects must be present (integrity).
        if objects.get(&root.lattice).await?.is_none() {
            return Ok(PolicyVerdict::Reject(format!("missing lattice for root {id}")));
        }
        if objects.get(&root.rules).await?.is_none() {
            return Ok(PolicyVerdict::Reject(format!("missing rules for root {id}")));
        }
        // Every hook kind must resolve in this replica (fail-closed).
        for hook in &root.hooks {
            if resolve_hook(hook).is_err() {
                return Ok(PolicyVerdict::Reject(format!(
                    "unknown hook kind '{}' in root {id}",
                    hook.kind
                )));
            }
        }
        // Direct-anchor signature requirement.
        match sigs.get(id) {
            Some(sig) if verify_root_signature(*id, sig, trust) => {}
            Some(_) => return Ok(PolicyVerdict::Reject(format!("bad signature on root {id}"))),
            None => return Ok(PolicyVerdict::Reject(format!("unsigned policy root {id}"))),
        }
    }
    Ok(PolicyVerdict::Accept { depth: chain.len() as u64 })
}

// bole-0tp
/// How two verified policy tips reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyResolution {
    /// `a` wins (it is `b` or a strictly-longer descendant lineage).
    Left,
    /// `b` wins.
    Right,
    /// No shared trusted root, or an equal-depth branch — refuse to auto-resolve.
    Diverged { left: ObjectId, right: ObjectId },
}

// bole-0tp
/// Highest-rooted-wins reconciliation (WS5 §5.3). Both tips must verify and share
/// a trusted root; the tip whose chain contains the other wins (policy
/// fast-forward). A genuine branch (neither an ancestor of the other, or no
/// shared root) is `Diverged` and must be reconciled by an authorised admin.
pub async fn reconcile(
    objects: &ObjectStore,
    a: ObjectId,
    b: ObjectId,
    sigs: &SignatureStore,
    trust: &TrustStore,
) -> Result<PolicyResolution> {
    // Both must independently verify.
    if !matches!(verify_chain(objects, a, sigs, trust).await?, PolicyVerdict::Accept { .. }) {
        return Err(Error::PolicyViolation(format!("candidate {a} does not verify")));
    }
    if !matches!(verify_chain(objects, b, sigs, trust).await?, PolicyVerdict::Accept { .. }) {
        return Err(Error::PolicyViolation(format!("candidate {b} does not verify")));
    }
    if a == b {
        return Ok(PolicyResolution::Left);
    }
    let chain_a = chain_of(objects, a).await?;
    let chain_b = chain_of(objects, b).await?;
    // Must share the same trusted root (last element).
    if chain_a.last() != chain_b.last() {
        return Ok(PolicyResolution::Diverged { left: a, right: b });
    }
    // If b is in a's ancestry, a is strictly further along → a wins (and vice versa).
    if chain_a.contains(&b) {
        Ok(PolicyResolution::Left)
    } else if chain_b.contains(&a) {
        Ok(PolicyResolution::Right)
    } else {
        Ok(PolicyResolution::Diverged { left: a, right: b })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::lattice::LabelLattice;
    use crate::acl::policy_object::{HookSpec, PolicyRoot};
    use crate::acl::rules::LabelRuleSet;
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use std::collections::BTreeMap;

    async fn base_lattice_rules(store: &ObjectStore) -> (ObjectId, ObjectId) {
        let lat = store.put(&Object::Policy(PolicyObject::Lattice(LabelLattice::two_point()))).await.unwrap();
        let rules = store.put(&Object::Policy(PolicyObject::RuleSet(LabelRuleSet::default()))).await.unwrap();
        (lat, rules)
    }

    async fn put_root(
        store: &ObjectStore,
        lattice: ObjectId,
        rules: ObjectId,
        parent: Option<ObjectId>,
        hooks: Vec<HookSpec>,
    ) -> ObjectId {
        store
            .put(&Object::Policy(PolicyObject::Root(PolicyRoot { lattice, rules, parent, hooks })))
            .await
            .unwrap()
    }

    // bole-m2p
    #[test]
    fn bare_id_signature_is_rejected_domain_separation() {
        // A signature over the BARE id (the pre-bole-m2p format, and the shape a
        // cross-scheme reuse would produce) must not verify as a policy root.
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let root = ObjectId::new([3u8; 32]);
        let mut trust = TrustStore::new();
        trust.add(TrustAnchor { key_id: "admin".into(), public_key: sk.verifying_key().to_bytes() });

        let bare = sk.sign(root.as_bytes());
        let bad = RootSignature { root, key_id: "admin".into(), sig: bare.to_bytes().to_vec() };
        assert!(
            !verify_root_signature(root, &bad, &trust),
            "a bare-id signature must be rejected — domain separation not enforced"
        );

        // The domain-separated signature verifies.
        let good_sig = sk.sign(&policy_root_message(root));
        let good = RootSignature { root, key_id: "admin".into(), sig: good_sig.to_bytes().to_vec() };
        assert!(verify_root_signature(root, &good, &trust));
    }

    #[tokio::test]
    async fn sign_and_verify_root() {
        let store = ObjectStore::new(MemoryBackend::new());
        let (lat, rules) = base_lattice_rules(&store).await;
        let root = put_root(&store, lat, rules, None, vec![]).await;

        let signer = PolicySigner::from_seed("admin", [7u8; 32]);
        let mut trust = TrustStore::new();
        trust.add(signer.anchor());
        let mut sigs = SignatureStore::new();
        sigs.insert(signer.sign_root(root));

        assert_eq!(verify_chain(&store, root, &sigs, &trust).await.unwrap(), PolicyVerdict::Accept { depth: 1 });
    }

    #[tokio::test]
    async fn unsigned_and_untrusted_and_bad_hook_reject() {
        let store = ObjectStore::new(MemoryBackend::new());
        let (lat, rules) = base_lattice_rules(&store).await;
        let signer = PolicySigner::from_seed("admin", [1u8; 32]);
        let mut trust = TrustStore::new();
        trust.add(signer.anchor());

        // Unsigned → reject.
        let root = put_root(&store, lat, rules, None, vec![]).await;
        let empty = SignatureStore::new();
        assert!(matches!(verify_chain(&store, root, &empty, &trust).await.unwrap(), PolicyVerdict::Reject(_)));

        // Signed by an untrusted key → reject.
        let stranger = PolicySigner::from_seed("evil", [2u8; 32]);
        let mut sigs = SignatureStore::new();
        sigs.insert(stranger.sign_root(root));
        assert!(matches!(verify_chain(&store, root, &sigs, &trust).await.unwrap(), PolicyVerdict::Reject(_)));

        // Unknown hook kind → reject (fail-closed), even when properly signed.
        let bad_root = put_root(
            &store, lat, rules, None,
            vec![HookSpec { kind: "not-a-real-hook".into(), pattern: "**".into(), params: BTreeMap::new() }],
        ).await;
        let mut sigs2 = SignatureStore::new();
        sigs2.insert(signer.sign_root(bad_root));
        assert!(matches!(verify_chain(&store, bad_root, &sigs2, &trust).await.unwrap(), PolicyVerdict::Reject(_)));
    }

    #[tokio::test]
    async fn longer_chain_wins_and_divergent_refuses() {
        let store = ObjectStore::new(MemoryBackend::new());
        let (lat, rules) = base_lattice_rules(&store).await;
        let signer = PolicySigner::from_seed("admin", [3u8; 32]);
        let mut trust = TrustStore::new();
        trust.add(signer.anchor());
        let mut sigs = SignatureStore::new();

        // Chain: r0 (root) → r1 (parent r0) → r2 (parent r1).
        let r0 = put_root(&store, lat, rules, None, vec![]).await;
        let r1 = put_root(&store, lat, rules, Some(r0), vec![]).await;
        let r2 = put_root(&store, lat, rules, Some(r1), vec![]).await;
        for r in [r0, r1, r2] {
            sigs.insert(signer.sign_root(r));
        }

        // r2 (depth 3) descends from r1 (depth 2) → r2 wins.
        assert_eq!(reconcile(&store, r2, r1, &sigs, &trust).await.unwrap(), PolicyResolution::Left);
        assert_eq!(reconcile(&store, r1, r2, &sigs, &trust).await.unwrap(), PolicyResolution::Right);

        // A sibling branch off r1 with the same trusted root but neither an
        // ancestor of the other → Diverged.
        let r2b = put_root(&store, lat, rules, Some(r1), vec![HookSpec {
            kind: "timeline-policy".into(), pattern: "*".into(), params: BTreeMap::new(),
        }]).await;
        sigs.insert(signer.sign_root(r2b));
        assert!(matches!(reconcile(&store, r2, r2b, &sigs, &trust).await.unwrap(), PolicyResolution::Diverged { .. }));
    }
}
