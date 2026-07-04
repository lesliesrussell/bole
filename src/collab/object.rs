// bole-eup
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;

/// Domain-separation tag for `Profile` signatures.
const COLLAB_PROFILE_DOMAIN: &[u8] = b"bole-collab-profile-v1\0";

// bole-2zq
const COLLAB_EDGE_DOMAIN: &[u8] = b"bole-collab-edge-v1\0";

// bole-jtf
/// Domain separator for the relay-auth possession handshake. Prepended to the
/// client nonce before signing so a relay-auth signature can never be confused
/// with a signature over an arbitrary 32-byte challenge from any other feature.
pub const COLLAB_RELAY_AUTH_DOMAIN: &[u8] = b"bole-relay-auth-v1";

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

// bole-2zq
/// Typed trust relation. `Vouch` = identity trust (drives petname suggestions);
/// `Follow` = discovery trust (drives the discovery neighborhood); `Review` =
/// reserved for future PR/review workflows (signed and stored, not yet consulted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustKind {
    Vouch,
    Follow,
    Review,
}

// bole-2zq
/// A directed, signed trust edge from `from_key` to `to_key`. `petname` is
/// meaningful only on `Vouch` edges and ignored elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEdge {
    pub from_key: Key,
    pub to_key: Key,
    pub kind: TrustKind,
    pub petname: Option<String>,
    pub seq: u64,
    pub sig: Vec<u8>,
}

/// The tagged union of collaboration objects (TrustEdge added in Task 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollabObject {
    Profile(Profile),
    // bole-2zq
    TrustEdge(TrustEdge),
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

// bole-2zq
#[derive(Serialize)]
struct EdgeMsg<'a> {
    from_key: &'a Key,
    to_key: &'a Key,
    kind: &'a TrustKind,
    petname: &'a Option<String>,
    seq: u64,
}

// bole-2zq
fn edge_message(e: &TrustEdge) -> Vec<u8> {
    let mut m = COLLAB_EDGE_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&EdgeMsg {
        from_key: &e.from_key,
        to_key: &e.to_key,
        kind: &e.kind,
        petname: &e.petname,
        seq: e.seq,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

// bole-jtf
/// The exact bytes a relay signs to prove possession of its key: the domain
/// separator followed by the client's nonce.
fn relay_challenge_message(nonce: &[u8; 32]) -> Vec<u8> {
    let mut m = COLLAB_RELAY_AUTH_DOMAIN.to_vec();
    m.extend_from_slice(nonce);
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

    // bole-2zq
    pub fn sign_edge(
        &self,
        to_key: Key,
        kind: TrustKind,
        petname: Option<String>,
        seq: u64,
    ) -> TrustEdge {
        let mut e = TrustEdge {
            from_key: self.public_key(),
            to_key,
            kind,
            petname,
            seq,
            sig: Vec::new(),
        };
        e.sig = self.signing.sign(&edge_message(&e)).to_bytes().to_vec();
        e
    }

    // bole-jtf
    /// Signs the domain-separated relay-auth challenge for `nonce`, proving
    /// possession of this signer's key to a client that pinned its public key.
    pub fn sign_relay_challenge(&self, nonce: &[u8; 32]) -> [u8; 64] {
        self.signing.sign(&relay_challenge_message(nonce)).to_bytes()
    }

    // bole-jtf
    #[cfg(test)]
    pub fn sign_relay_challenge_raw_for_test(&self, nonce: &[u8; 32]) -> [u8; 64] {
        self.signing.sign(nonce).to_bytes()
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

// bole-2zq
/// True iff `e.sig` verifies against `e.from_key` over the domain-separated fields.
pub fn verify_edge(e: &TrustEdge) -> bool {
    let vk = match VerifyingKey::from_bytes(&e.from_key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match e.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&edge_message(e), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

// bole-jtf
/// True iff `sig` is `key`'s Ed25519 signature over `COLLAB_RELAY_AUTH_DOMAIN || nonce`.
pub fn verify_relay_challenge(key: &Key, nonce: &[u8; 32], sig: &[u8; 64]) -> bool {
    let vk = match VerifyingKey::from_bytes(key) {
        Ok(v) => v,
        Err(_) => return false,
    };
    vk.verify(&relay_challenge_message(nonce), &ed25519_dalek::Signature::from_bytes(sig)).is_ok()
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

    // bole-2zq
    #[test]
    fn trust_edge_signature_verifies() {
        let a = CollabSigner::from_seed([2u8; 32]);
        let b = CollabSigner::from_seed([3u8; 32]);
        let e = a.sign_edge(b.public_key(), TrustKind::Vouch, Some("bee".into()), 1);
        assert!(verify_edge(&e));
        assert_eq!(e.from_key, a.public_key());
        assert_eq!(e.to_key, b.public_key());
        assert_eq!(e.kind, TrustKind::Vouch);

        let mut tampered = e.clone();
        tampered.kind = TrustKind::Follow;
        assert!(!verify_edge(&tampered), "kind is a signed field");
    }

    #[tokio::test]
    async fn review_edge_round_trips() {
        let store = ObjectStore::new(MemoryBackend::new());
        let a = CollabSigner::from_seed([4u8; 32]);
        let b = CollabSigner::from_seed([5u8; 32]);
        // Review is reserved: signed and stored now, consulted by no subsystem yet.
        let e = a.sign_edge(b.public_key(), TrustKind::Review, None, 1);
        assert!(verify_edge(&e));
        let wrapped = Object::Collab(CollabObject::TrustEdge(e));
        let id = store.put(&wrapped).await.unwrap();
        assert_eq!(store.get(&id).await.unwrap().unwrap(), wrapped);
    }

    // bole-jtf
    #[test]
    fn relay_challenge_accepts_valid_and_rejects_tampering() {
        let signer = CollabSigner::from_seed([1u8; 32]);
        let nonce = [42u8; 32];
        let sig = signer.sign_relay_challenge(&nonce);

        // Accept: right key, right nonce, domain-separated.
        assert!(verify_relay_challenge(&signer.public_key(), &nonce, &sig));

        // Reject: wrong key.
        let other = CollabSigner::from_seed([2u8; 32]);
        assert!(!verify_relay_challenge(&other.public_key(), &nonce, &sig));

        // Reject: different nonce (replay of a signature for another challenge).
        let nonce2 = [43u8; 32];
        assert!(!verify_relay_challenge(&signer.public_key(), &nonce2, &sig));

        // Reject: a signature over the BARE nonce (no domain separator).
        let bare = signer_sign_raw(&signer, &nonce); // helper below
        assert!(!verify_relay_challenge(&signer.public_key(), &nonce, &bare));
    }

    // bole-jtf
    // Test-only: sign the raw nonce with no domain separator, to prove the domain
    // separator is load-bearing.
    fn signer_sign_raw(signer: &CollabSigner, nonce: &[u8; 32]) -> [u8; 64] {
        signer.sign_relay_challenge_raw_for_test(nonce)
    }
}
