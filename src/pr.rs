// bole-060a
//! Change proposals — the object layer of the PR system.
//!
//! A [`ChangeProposal`] is a signed, content-addressed request to merge one
//! timeline (`source`) into another (`target`). It is metadata only: like a
//! [`Profile`](crate::Profile) it grants nothing and never overrides the
//! lattice/ACLs — the actual merge is still gated by `check_merge` and the
//! approval `PolicyHook`. This slice defines the object, its signing, and
//! fail-closed verification; later slices add the CLI, review threads, and the
//! approval-gated merge action.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;

// bole-060a
/// Domain-separation tag for change-proposal signatures. Prevents a proposal
/// signature from being confused with any other bole Ed25519 scheme.
const PROPOSAL_DOMAIN: &[u8] = b"bole-change-proposal-v1\0";

// bole-060a
/// A signed request to merge `source` into `target`. `source`/`target` are
/// timeline ref names (e.g. `feature/x`, `release/1.0`), not object ids — the
/// proposal tracks intent; the heads are resolved at merge time. Canonical
/// author is `author` (its key verifies `sig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeProposal {
    pub author: Key,
    pub source: String,
    pub target: String,
    pub title: String,
    pub created_at: u64,
    /// Ed25519 signature (64 bytes) over the domain-separated unsigned fields.
    pub sig: Vec<u8>,
}

// bole-060a
#[derive(Serialize)]
struct ProposalMsg<'a> {
    author: &'a Key,
    source: &'a str,
    target: &'a str,
    title: &'a str,
    created_at: u64,
}

// bole-060a
fn proposal_message(p: &ChangeProposal) -> Vec<u8> {
    let mut m = PROPOSAL_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&ProposalMsg {
        author: &p.author,
        source: &p.source,
        target: &p.target,
        title: &p.title,
        created_at: p.created_at,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

// bole-060a
/// Signs [`ChangeProposal`]s under a held Ed25519 key. Mirrors
/// [`CollabSigner`](crate::CollabSigner); a KMS-backed signer can replace this.
pub struct ProposalSigner {
    signing: SigningKey,
}

impl ProposalSigner {
    /// Builds a signer from a 32-byte seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }

    /// The public key that authors — and verifies — this signer's proposals.
    pub fn public_key(&self) -> Key {
        self.signing.verifying_key().to_bytes()
    }

    /// Signs a proposal merging `source` into `target`.
    pub fn sign_proposal(
        &self,
        source: impl Into<String>,
        target: impl Into<String>,
        title: impl Into<String>,
        created_at: u64,
    ) -> ChangeProposal {
        let mut p = ChangeProposal {
            author: self.public_key(),
            source: source.into(),
            target: target.into(),
            title: title.into(),
            created_at,
            sig: Vec::new(),
        };
        p.sig = self.signing.sign(&proposal_message(&p)).to_bytes().to_vec();
        p
    }
}

// bole-060a
/// Verifies a proposal's signature against its embedded author key. Fail-closed:
/// a malformed key or signature returns `false`.
pub fn verify_proposal(p: &ChangeProposal) -> bool {
    let vk = match VerifyingKey::from_bytes(&p.author) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match p.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&proposal_message(p), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let signer = ProposalSigner::from_seed([1u8; 32]);
        let p = signer.sign_proposal("feature/x", "release/1.0", "Add x", 42);
        assert_eq!(p.author, signer.public_key());
        assert_eq!(p.source, "feature/x");
        assert_eq!(p.target, "release/1.0");
        assert!(verify_proposal(&p));
    }

    #[test]
    fn tampered_fields_fail_verification() {
        let signer = ProposalSigner::from_seed([2u8; 32]);
        // Each mutation must break the signature.
        let mut p = signer.sign_proposal("feature/x", "main", "t", 1);
        p.target = "release/prod".into();
        assert!(!verify_proposal(&p), "tampered target must not verify");

        let mut p2 = signer.sign_proposal("feature/x", "main", "t", 1);
        p2.title = "malicious".into();
        assert!(!verify_proposal(&p2), "tampered title must not verify");

        let mut p3 = signer.sign_proposal("feature/x", "main", "t", 1);
        p3.author = ProposalSigner::from_seed([3u8; 32]).public_key();
        assert!(!verify_proposal(&p3), "swapped author must not verify");

        let mut p4 = signer.sign_proposal("feature/x", "main", "t", 1);
        p4.source = "feature/evil".into();
        assert!(!verify_proposal(&p4), "tampered source must not verify");

        let mut p5 = signer.sign_proposal("feature/x", "main", "t", 1);
        p5.created_at = 999;
        assert!(!verify_proposal(&p5), "tampered created_at must not verify");
    }

    #[test]
    fn malformed_signature_is_false_not_panic() {
        let signer = ProposalSigner::from_seed([4u8; 32]);
        let mut p = signer.sign_proposal("a", "b", "t", 0);
        p.sig = vec![0u8; 10]; // wrong length
        assert!(!verify_proposal(&p));
    }
}
