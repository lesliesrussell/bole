// bole-ub3h
//! Repo records — a user's announcement that they own a named repository.
//!
//! A [`RepoRecord`] is a signed, content-addressed record that developer
//! `owner` has a repo named `name`. Like a [`Profile`](crate::Profile) it is
//! metadata only — it grants nothing and enforces nothing; it is what a hub
//! (Grove) enumerates to list "all of a user's repos" under their profile. The
//! repo's actual content (timelines/snapshots) lives under the hub's per-owner
//! ref namespace, gated by ACL — that is a later slice; this one is the record.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;

// bole-ub3h
/// Domain-separation tag for repo-record signatures.
const REPO_RECORD_DOMAIN: &[u8] = b"bole-repo-record-v1\0";

// bole-ub3h
/// A signed announcement of a repo owned by `owner`. `seq` is a per-(owner,name)
/// monotonic counter so a newer record supersedes an older one (e.g. a
/// description edit). Canonical author is `owner` (its key verifies `sig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRecord {
    pub owner: Key,
    pub name: String,
    pub description: String,
    pub seq: u64,
    /// Ed25519 signature (64 bytes) over the domain-separated unsigned fields.
    pub sig: Vec<u8>,
}

// bole-ub3h
#[derive(Serialize)]
struct RepoMsg<'a> {
    owner: &'a Key,
    name: &'a str,
    description: &'a str,
    seq: u64,
}

// bole-ub3h
fn repo_message(r: &RepoRecord) -> Vec<u8> {
    let mut m = REPO_RECORD_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&RepoMsg {
        owner: &r.owner,
        name: &r.name,
        description: &r.description,
        seq: r.seq,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

// bole-ub3h
/// Signs [`RepoRecord`]s under a held Ed25519 key. Mirrors
/// [`CollabSigner`](crate::CollabSigner).
pub struct RepoSigner {
    signing: SigningKey,
}

impl RepoSigner {
    /// Builds a signer from a 32-byte seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }

    /// The public key that owns — and verifies — this signer's repo records.
    pub fn public_key(&self) -> Key {
        self.signing.verifying_key().to_bytes()
    }

    /// Signs a record announcing the repo `name`.
    pub fn sign_repo(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        seq: u64,
    ) -> RepoRecord {
        let mut r = RepoRecord {
            owner: self.public_key(),
            name: name.into(),
            description: description.into(),
            seq,
            sig: Vec::new(),
        };
        r.sig = self.signing.sign(&repo_message(&r)).to_bytes().to_vec();
        r
    }
}

// bole-ub3h
/// Verifies a repo record's signature against its embedded owner key.
/// Fail-closed: a malformed key or signature returns `false`.
pub fn verify_repo(r: &RepoRecord) -> bool {
    let vk = match VerifyingKey::from_bytes(&r.owner) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match r.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&repo_message(r), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_tamper() {
        let signer = RepoSigner::from_seed([1u8; 32]);
        let r = signer.sign_repo("dotfiles", "my shell + editor config", 1);
        assert_eq!(r.owner, signer.public_key());
        assert_eq!(r.name, "dotfiles");
        assert!(verify_repo(&r));

        let mut r1 = signer.sign_repo("dotfiles", "d", 1);
        r1.name = "secrets".into();
        assert!(!verify_repo(&r1), "tampered name");
        let mut r2 = signer.sign_repo("dotfiles", "d", 1);
        r2.description = "evil".into();
        assert!(!verify_repo(&r2), "tampered description");
        let mut r3 = signer.sign_repo("dotfiles", "d", 1);
        r3.seq = 99;
        assert!(!verify_repo(&r3), "tampered seq");
        let mut r4 = signer.sign_repo("dotfiles", "d", 1);
        r4.owner = RepoSigner::from_seed([2u8; 32]).public_key();
        assert!(!verify_repo(&r4), "swapped owner");
    }

    #[test]
    fn malformed_signature_is_false_not_panic() {
        let signer = RepoSigner::from_seed([3u8; 32]);
        let mut r = signer.sign_repo("r", "d", 0);
        r.sig = vec![0u8; 5];
        assert!(!verify_repo(&r));
    }
}
