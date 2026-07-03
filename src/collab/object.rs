// bole-eup
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;

/// Domain-separation tag for `Profile` signatures.
const COLLAB_PROFILE_DOMAIN: &[u8] = b"bole-collab-profile-v1\0";

/// A self-signed, per-key, monotonic self-description. Metadata only — it grants
/// nothing and never overrides the lattice/ACLs. Canonical identity is `key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub key: Key,
    pub display_name: String,
    pub bio: String,
    pub endpoints: Vec<String>,
    pub dns_aliases: Vec<String>,
    pub seq: u64,
    /// Ed25519 signature (64 bytes) over the domain-separated unsigned fields.
    pub sig: Vec<u8>,
}

/// The tagged union of collaboration objects (TrustEdge added in Task 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollabObject {
    Profile(Profile),
}

#[derive(Serialize)]
struct ProfileMsg<'a> {
    key: &'a Key,
    display_name: &'a str,
    bio: &'a str,
    endpoints: &'a [String],
    dns_aliases: &'a [String],
    seq: u64,
}

fn profile_message(p: &Profile) -> Vec<u8> {
    let mut m = COLLAB_PROFILE_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&ProfileMsg {
        key: &p.key,
        display_name: &p.display_name,
        bio: &p.bio,
        endpoints: &p.endpoints,
        dns_aliases: &p.dns_aliases,
        seq: p.seq,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

/// Holds a signing key and issues signed collaboration objects.
pub struct CollabSigner {
    signing: SigningKey,
}

impl CollabSigner {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }

    pub fn public_key(&self) -> Key {
        self.signing.verifying_key().to_bytes()
    }

    pub fn sign_profile(
        &self,
        display_name: String,
        bio: String,
        endpoints: Vec<String>,
        dns_aliases: Vec<String>,
        seq: u64,
    ) -> Profile {
        let mut p = Profile {
            key: self.public_key(),
            display_name,
            bio,
            endpoints,
            dns_aliases,
            seq,
            sig: Vec::new(),
        };
        p.sig = self.signing.sign(&profile_message(&p)).to_bytes().to_vec();
        p
    }
}

/// True iff `p.sig` verifies against `p.key` over the domain-separated fields.
pub fn verify_profile(p: &Profile) -> bool {
    let vk = match VerifyingKey::from_bytes(&p.key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match p.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&profile_message(p), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Object;
    use crate::store::{memory::MemoryBackend, ObjectStore};

    #[tokio::test]
    async fn collab_object_round_trips_with_stable_ids() {
        let store = ObjectStore::new(MemoryBackend::new());
        let signer = CollabSigner::from_seed([7u8; 32]);
        let p = signer.sign_profile("Alice".into(), "hi".into(), vec![], vec![], 1);
        let wrapped = Object::Collab(CollabObject::Profile(p));
        let id1 = store.put(&wrapped).await.unwrap();
        let got = store.get(&id1).await.unwrap().unwrap();
        assert_eq!(got, wrapped);
        let id2 = store.put(&wrapped).await.unwrap();
        assert_eq!(id1, id2, "content-addressed id must be stable");
    }

    #[test]
    fn profile_signature_verifies() {
        let signer = CollabSigner::from_seed([9u8; 32]);
        let p = signer.sign_profile("Bob".into(), String::new(), vec!["n1".into()], vec![], 3);
        assert!(verify_profile(&p));
        assert_eq!(p.key, signer.public_key());
    }

    #[test]
    fn tampered_profile_rejected() {
        let signer = CollabSigner::from_seed([1u8; 32]);
        let mut p = signer.sign_profile("Carol".into(), String::new(), vec![], vec![], 1);
        p.display_name = "Mallory".into(); // mutate a signed field
        assert!(!verify_profile(&p));
    }
}
