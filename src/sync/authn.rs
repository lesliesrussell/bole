// bole-6h7
//! Authn/authz for sync: map an authenticated connection principal to a bole
//! actor and build the [`Accessor`] every push/fetch is then checked against
//! (WS5 §6). There is no second authorization model — the resulting `Accessor`
//! is evaluated by the *same* WS1 rules `advance_timeline` uses. Optional signed
//! ref updates (§6.4) tie a head move to an actor's key, not just the connection.

use std::collections::HashMap;
use std::sync::Arc;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

use crate::acl::{Accessor, AclStore};
use crate::error::Result;
use crate::object::ObjectId;
use crate::refs::RefName;

// bole-6h7
/// A stable authenticated principal surfaced by the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    SshKey(String),
    Token(String),
    Mtls(String),
    Anonymous,
}

// bole-6h7
/// How the principal was authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    Ssh,
    Token,
    Mtls,
    None,
}

// bole-6h7
/// The verified identity of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    pub principal: Principal,
    pub method: AuthMethod,
}

// bole-6h7
/// Maps authenticated principals to bole actor names (the server's
/// `authorized_keys` equivalent). An unmapped principal is anonymous.
#[derive(Debug, Clone, Default)]
pub struct ActorMap {
    ssh: HashMap<String, String>,
    token: HashMap<String, String>,
    mtls: HashMap<String, String>,
}

impl ActorMap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn map_ssh_key(&mut self, key_id: impl Into<String>, actor: impl Into<String>) {
        self.ssh.insert(key_id.into(), actor.into());
    }
    pub fn map_token(&mut self, token: impl Into<String>, actor: impl Into<String>) {
        self.token.insert(token.into(), actor.into());
    }
    pub fn map_mtls(&mut self, subject: impl Into<String>, actor: impl Into<String>) {
        self.mtls.insert(subject.into(), actor.into());
    }
    /// The bole actor a principal maps to, if any.
    pub fn actor_for(&self, principal: &Principal) -> Option<&str> {
        match principal {
            Principal::SshKey(k) => self.ssh.get(k).map(String::as_str),
            Principal::Token(t) => self.token.get(t).map(String::as_str),
            Principal::Mtls(s) => self.mtls.get(s).map(String::as_str),
            Principal::Anonymous => None,
        }
    }
}

// bole-6h7
/// Builds the `Accessor` for a connection: the mapped actor's stored clearance
/// grant over the repo's lattice + rules. An unmapped principal (or a mapped
/// actor with no grant) becomes an anonymous accessor — no clearances, so it may
/// read only public (lattice-bottom) resources and can write nothing.
pub fn accessor_for(
    store: &AclStore,
    actor_map: &ActorMap,
    principal: &Principal,
) -> Result<Accessor> {
    let lattice = Arc::new(store.lattice()?);
    let rules = Arc::new(store.label_ruleset()?);
    if let Some(actor) = actor_map.actor_for(principal) {
        if let Some(grant) = store.grant(actor)? {
            return Ok(Accessor::from_parts(lattice, rules, grant.clearances));
        }
    }
    // Anonymous / no grant: empty clearances (public-read, no write).
    Ok(Accessor::from_parts(
        lattice,
        rules,
        crate::acl::clearance::ClearanceSet::default(),
    ))
}

// bole-6h7
/// Signs ref updates on behalf of an actor (SIGNED_REFS, §6.4).
pub struct RefSigner {
    signing: SigningKey,
}

impl RefSigner {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }
    /// This signer's public key (registered against the actor for verification).
    pub fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }
    /// A detached signature over `(name, expected_old, new_head)`.
    pub fn sign_ref_op(&self, name: &RefName, expected_old: &Option<ObjectId>, new_head: &ObjectId) -> Vec<u8> {
        self.signing.sign(&ref_op_message(name, expected_old, new_head)).to_bytes().to_vec()
    }
}

// bole-6h7
/// Verifies a ref-update signature against `public_key`.
pub fn verify_ref_op(
    name: &RefName,
    expected_old: &Option<ObjectId>,
    new_head: &ObjectId,
    sig: &[u8],
    public_key: &[u8; 32],
) -> bool {
    let vk = match VerifyingKey::from_bytes(public_key) {
        Ok(vk) => vk,
        Err(_) => return false,
    };
    let sig_bytes: [u8; 64] = match sig.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    vk.verify(&ref_op_message(name, expected_old, new_head), &signature).is_ok()
}

// bole-m2p
/// Domain-separation tag for signed ref updates (see the analogous tags in
/// `acl::authority` / `acl::attestation`). Binds the signature to this scheme.
const REF_OP_DOMAIN: &[u8] = b"bole-ref-op-v1\0";

// bole-6h7
/// Canonical signed message for a ref update.
fn ref_op_message(name: &RefName, expected_old: &Option<ObjectId>, new_head: &ObjectId) -> Vec<u8> {
    let mut m = Vec::new();
    // bole-m2p: domain tag first, so this can't be confused with another scheme.
    m.extend_from_slice(REF_OP_DOMAIN);
    m.extend_from_slice(name.as_str().as_bytes());
    m.push(0);
    if let Some(old) = expected_old {
        m.extend_from_slice(old.as_bytes());
    }
    m.push(0);
    m.extend_from_slice(new_head.as_bytes());
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
    use crate::acl::policy_object::ClearanceGrant;
    use crate::acl::lattice::Label;
    use crate::acl::{Accessor, ResourceRef};
    use crate::repo::Repository;

    #[test]
    fn mapped_actor_gets_grant_anonymous_gets_public_read() {
        let repo = Repository::memory();
        // Grant "alice" write on all timelines.
        repo.acls
            .set_grant(ClearanceGrant {
                actor: "alice".into(),
                clearances: ClearanceSet {
                    clearances: vec![Clearance {
                        ceiling: Label::protected(),
                        cap: Capability::WRITE | Capability::READ,
                        scope: Some(ClearanceScope::Timeline("**".into())),
                    }],
                    confined: false,
                },
                signature: None,
            })
            .unwrap();

        let mut map = ActorMap::new();
        map.map_token("secret-token", "alice");

        // A token that maps to alice → write-capable accessor.
        let alice = accessor_for(&repo.acls, &map, &Principal::Token("secret-token".into())).unwrap();
        assert!(alice.can_write(&Label::protected(), ResourceRef::Timeline("main")));

        // An unmapped token → anonymous: no clearances, so it cannot write and
        // cannot read anything above public. (Public/bottom resources are made
        // visible by the repo's `label == bottom` short-circuit, not by the
        // accessor itself, so `can_read(public)` is legitimately false here.)
        let anon = accessor_for(&repo.acls, &map, &Principal::Token("unknown".into())).unwrap();
        assert!(!anon.can_write(&Label::protected(), ResourceRef::Timeline("main")));
        assert!(!anon.can_read(&Label::protected(), ResourceRef::Timeline("main")));

        // Explicit Anonymous principal → same as unmapped.
        let anon2 = accessor_for(&repo.acls, &map, &Principal::Anonymous).unwrap();
        assert!(!anon2.can_write(&Label::protected(), ResourceRef::Timeline("main")));
        let _ = Accessor::new();
    }

    #[tokio::test]
    async fn push_over_session_authorized_via_principal() {
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use crate::sync::session::{client_fetch, client_push, serve};
        use crate::sync::transport::InProcessConn;
        use crate::sync::wire::RefApplyStatus;
        use std::collections::BTreeMap;
        use std::sync::Arc;

        async fn commit(repo: &Repository, parent: Option<ObjectId>, p: &[u8]) -> ObjectId {
            let b = repo.objects.put_blob(bytes::Bytes::copy_from_slice(p)).await.unwrap();
            let mut e = BTreeMap::new();
            e.insert("f".to_string(), TreeEntry { id: b, kind: EntryKind::Blob });
            let t = repo.objects.put_tree(e).await.unwrap();
            repo.objects
                .put_snapshot(Snapshot { root: t, parents: parent.into_iter().collect(), author: "t".into(), created_at: 0, message: "m".into() })
                .await
                .unwrap()
        }

        let server = Arc::new(Repository::memory());
        let base = commit(&server, None, b"base").await;
        let name = RefName::new("main").unwrap();
        server.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        // Grant "alice" write; map her SSH key.
        server.acls.set_grant(ClearanceGrant {
            actor: "alice".into(),
            clearances: ClearanceSet {
                clearances: vec![Clearance { ceiling: Label::protected(), cap: Capability::WRITE, scope: Some(ClearanceScope::Timeline("**".into())) }],
                confined: false,
            },
            signature: None,
        }).unwrap();
        let mut map = ActorMap::new();
        map.map_ssh_key("alice-key", "alice");
        let map = Arc::new(map);

        let client = Repository::memory();
        // Fetch (as an anonymous reader) to seed the client + tracking ref.
        {
            let (mut cc, mut sc) = InProcessConn::pair();
            let (srv, m) = (server.clone(), map.clone());
            let h = tokio::spawn(async move {
                let acc = accessor_for(&srv.acls, &m, &Principal::Anonymous).unwrap();
                serve(&mut sc, &srv, &acc).await
            });
            client_fetch(&mut cc, &client, "origin").await.unwrap();
            h.await.unwrap().unwrap();
        }
        client.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let next = commit(&client, Some(base), b"next").await;
        client.refs.advance_head(&name, next).unwrap();

        // Push authorized as alice (server builds her accessor from the principal).
        let (mut cc, mut sc) = InProcessConn::pair();
        let (srv, m) = (server.clone(), map.clone());
        let h = tokio::spawn(async move {
            let acc = accessor_for(&srv.acls, &m, &Principal::SshKey("alice-key".into())).unwrap();
            serve(&mut sc, &srv, &acc).await
        });
        let res = client_push(&mut cc, &client, "origin", std::slice::from_ref(&name)).await.unwrap();
        h.await.unwrap().unwrap();
        assert_eq!(res[0].status, RefApplyStatus::Ok);
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, next);

        // An anonymous principal cannot push.
        let another = commit(&client, Some(next), b"more").await;
        client.refs.advance_head(&name, another).unwrap();
        let (mut cc, mut sc) = InProcessConn::pair();
        let srv = server.clone();
        let m = map.clone();
        let h = tokio::spawn(async move {
            let acc = accessor_for(&srv.acls, &m, &Principal::Anonymous).unwrap();
            serve(&mut sc, &srv, &acc).await
        });
        let res2 = client_push(&mut cc, &client, "origin", std::slice::from_ref(&name)).await.unwrap();
        h.await.unwrap().unwrap();
        assert!(matches!(res2[0].status, RefApplyStatus::Denied(_)));
        // Server head unchanged after the denied push.
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, next);
    }

    #[test]
    fn signed_ref_op_verifies_and_rejects_tampering() {
        let signer = RefSigner::from_seed([9u8; 32]);
        let name = RefName::new("main").unwrap();
        let old = Some(ObjectId::from_content(b"old"));
        let new = ObjectId::from_content(b"new");
        let sig = signer.sign_ref_op(&name, &old, &new);

        assert!(verify_ref_op(&name, &old, &new, &sig, &signer.public_key()));
        // Tampered new head → reject.
        let other = ObjectId::from_content(b"evil");
        assert!(!verify_ref_op(&name, &old, &other, &sig, &signer.public_key()));
        // Wrong key → reject.
        let stranger = RefSigner::from_seed([1u8; 32]);
        assert!(!verify_ref_op(&name, &old, &new, &sig, &stranger.public_key()));
    }
}
