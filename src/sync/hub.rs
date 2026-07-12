// bole-1x2v
//! Owner-authenticated push for the multi-user hub.
//!
//! A hub accepts pushes from many users into one store, namespaced by owner:
//! `refs/users/<owner-fp>/<repo>/<timeline>`. Before any write, the pusher
//! proves possession of the owner key by a challenge–response: the hub sends a
//! random nonce, the pusher signs it (domain-separated) with the owner's
//! ed25519 key, the hub verifies it and builds an [`Accessor`] scoped to
//! `refs/users/<owner-fp>/**`. The existing `apply_push_ops` then refuses, by
//! ACL, any op outside that namespace — so a user can only write their own
//! repos.
//!
//! No TLS: the transport is still trusted-network-only. This adds *ownership*
//! (which namespace a connection may write), not transport confidentiality.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::acl::{Accessor, Permission, TimelineRole};
use crate::collab::fingerprint;
use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;
use crate::sync::negotiate;
use crate::sync::session::{advertise, apply_push_ops, build_pack};
use crate::sync::transport::Conn;
use crate::sync::wire::{
    CapSet, Message, RefApplyStatus, RefStatusEntry, RefUpdateOp, PROTO_VERSION,
};
use crate::store::pack::decode_pack;

// bole-1x2v
/// Domain separator for the hub push-auth challenge. Prepended to the nonce
/// before signing so a hub-auth signature can never be reused as any other
/// bole ed25519 signature (profiles, ref-ops, relay auth, …).
const HUB_PUSH_DOMAIN: &[u8] = b"bole-hub-push-v1\0";

// bole-1x2v
/// The ref namespace an `owner` may write on a hub.
pub fn user_namespace(owner: &[u8; 32]) -> String {
    format!("refs/users/{}/", fingerprint(owner))
}

// bole-1x2v
/// The bytes a pusher signs to answer the hub's challenge.
fn challenge_message(nonce: &[u8; 32]) -> Vec<u8> {
    let mut m = HUB_PUSH_DOMAIN.to_vec();
    m.extend_from_slice(nonce);
    m
}

// bole-1x2v
/// An accessor that may read+write only `refs/users/<owner-fp>/**` — the
/// owner's namespace — and nothing else. Write is what fences ownership; read
/// lets the hub advertise the owner's existing heads for CAS.
fn owner_accessor(owner: &[u8; 32]) -> Accessor {
    let pattern = format!("refs/users/{}/**", fingerprint(owner));
    Accessor::new()
        .with_timeline_role(TimelineRole { pattern: pattern.clone(), permission: Permission::Write })
        .with_timeline_role(TimelineRole { pattern, permission: Permission::Read })
        .with_actor(fingerprint(owner))
}

// bole-1x2v
/// Verifies a hub push-auth proof: `sig` over the domain-tagged `nonce` under
/// `owner`. Fail-closed on a malformed key or signature.
fn verify_proof(owner: &[u8; 32], nonce: &[u8; 32], sig: &[u8]) -> bool {
    let vk = match VerifyingKey::from_bytes(owner) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match sig.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&challenge_message(nonce), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

// bole-1x2v
/// Hub responder: authenticate the pusher, then accept a push scoped to their
/// namespace. Runs the same object-transfer + `apply_push_ops` as the ordinary
/// push path, but with an owner-scoped accessor so out-of-namespace ops are
/// ACL-refused.
pub async fn serve_hub_push(conn: &mut dyn Conn, repo: &Repository) -> Result<()> {
    // 1. Who is pushing?
    let owner = match conn.recv().await? {
        Message::HubHello { owner } => owner,
        _ => {
            conn.send(&Message::Error("expected HubHello".into())).await?;
            return Err(Error::Storage("hub: expected HubHello".into()));
        }
    };
    // 2. Challenge with a fresh nonce.
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    conn.send(&Message::HubChallenge { nonce }).await?;
    // 3. Verify the proof, fail-closed.
    let sig = match conn.recv().await? {
        Message::HubProof { sig } => sig,
        _ => {
            conn.send(&Message::Error("expected HubProof".into())).await?;
            return Err(Error::Storage("hub: expected HubProof".into()));
        }
    };
    if !verify_proof(&owner, &nonce, &sig) {
        conn.send(&Message::Error("hub auth failed".into())).await?;
        return Err(Error::AccessDenied("hub push auth failed".into()));
    }
    let accessor = owner_accessor(&owner);

    // 4. Push exchange with the owner-scoped accessor. bole-1x2v: advertise
    // ONLY the owner's own namespace — the client just needs its own heads for
    // CAS, and a default hub labels all refs bottom (public), which would
    // otherwise short-circuit the read scope and advertise every owner's
    // topology on the push handshake.
    let ns = user_namespace(&owner);
    let refs: Vec<_> = advertise(repo, &accessor)?
        .into_iter()
        .filter(|r| r.name.as_str().starts_with(&ns))
        .collect();
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs, relay_sig: None })
        .await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("hub: expected Pack".into())),
    };
    let decoded = decode_pack(&pack)?;
    let ops = match conn.recv().await? {
        Message::RefUpdate(ops) => ops,
        _ => return Err(Error::Storage("hub: expected RefUpdate".into())),
    };
    for (_id, canonical) in &decoded {
        repo.objects.put_raw(canonical).await?;
    }
    let results = apply_push_ops(repo, &accessor, &ops).await?;
    conn.send(&Message::RefStatus(results)).await?;
    Ok(())
}

// bole-1x2v
/// Hub push initiator: authenticate as `owner` (holding `owner_seed`) and push
/// `timelines` (already namespaced under `refs/users/<owner-fp>/…`). Returns
/// per-ref status; advances remote-tracking refs for accepted ops.
pub async fn hub_push(
    conn: &mut dyn Conn,
    local: &Repository,
    owner_seed: &[u8; 32],
    remote_name: &str,
    pushes: &[(RefName, RefName)],
) -> Result<Vec<RefStatusEntry>> {
    let signing = SigningKey::from_bytes(owner_seed);
    let owner = signing.verifying_key().to_bytes();
    conn.send(&Message::HubHello { owner }).await?;
    let nonce = match conn.recv().await? {
        Message::HubChallenge { nonce } => nonce,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("hub: expected HubChallenge".into())),
    };
    let sig = signing.sign(&challenge_message(&nonce)).to_bytes().to_vec();
    conn.send(&Message::HubProof { sig }).await?;

    let server_refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("hub: expected Welcome".into())),
    };
    let server_have: std::collections::HashSet<ObjectId> =
        server_refs.iter().map(|r| r.target).collect();

    let mut ops = Vec::new();
    let mut wants = Vec::new();
    // Each push is (local timeline name, remote hub name under refs/users/…).
    for (local_name, remote) in pushes {
        let tl = match local.refs.get_timeline(local_name)? {
            Some(t) => t,
            None => continue,
        };
        let tracking = RefName::new(format!("refs/remotes/{remote_name}/{}", remote.as_str()))
            .map_err(|e| Error::Storage(format!("bad tracking ref name: {e}")))?;
        let expected_old = local.refs.get_tag(&tracking)?.map(|t| t.target);
        wants.push(tl.head);
        ops.push(RefUpdateOp { name: remote.clone(), expected_old, new_head: tl.head });
    }
    let missing = negotiate::missing_closure(local, &wants, &server_have).await?;
    let pack = build_pack(local, &missing).await?;
    conn.send(&Message::Pack(pack)).await?;
    conn.send(&Message::RefUpdate(ops.clone())).await?;

    let results = match conn.recv().await? {
        Message::RefStatus(r) => r,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("hub: expected RefStatus".into())),
    };
    let mut tx = local.refs.transaction();
    for entry in &results {
        if entry.status == RefApplyStatus::Ok {
            if let Some(op) = ops.iter().find(|o| o.name == entry.name) {
                let tracking = RefName::new(format!("refs/remotes/{remote_name}/{}", op.name.as_str()))?;
                tx.set(tracking, Ref::Tag(Tag { target: op.new_head, created_at: 0, message: None }));
            }
        }
    }
    tx.commit()?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Object, Snapshot};
    use crate::refs::TimelinePolicy;
    use crate::reporecord::RepoSigner;
    use crate::sync::transport::InProcessConn;
    use std::sync::Arc;
    use std::collections::BTreeMap;

    async fn seed_local_repo(seed: [u8; 32], repo_name: &str) -> (Repository, RefName, ObjectId) {
        let repo = Repository::memory();
        let fp = fingerprint(&RepoSigner::from_seed(seed).public_key());
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let head = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "c".into() })
            .await
            .unwrap();
        let name = RefName::new(format!("refs/users/{fp}/{repo_name}/main")).unwrap();
        repo.refs.create_timeline(name.clone(), head, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        (repo, name, head)
    }

    #[tokio::test]
    async fn authenticated_push_lands_in_owner_namespace() {
        let seed = [3u8; 32];
        let (client, name, head) = seed_local_repo(seed, "grove").await;
        let hub = Repository::memory();
        let (mut a, mut b) = InProcessConn::pair();

        let hub = Arc::new(hub);
        let hub2 = hub.clone();
        let server = tokio::spawn(async move { serve_hub_push(&mut b, &hub2).await });
        let results = hub_push(&mut a, &client, &seed, "hub", &[(name.clone(), name.clone())]).await.unwrap();
        server.await.unwrap().unwrap();

        assert!(matches!(results[0].status, RefApplyStatus::Ok), "push accepted: {:?}", results[0].status);
        assert_eq!(hub.refs.get_timeline(&name).unwrap().unwrap().head, head, "landed in the owner's namespace");
    }

    // bole-1x2v: the push handshake advertises only the pusher's own namespace,
    // not other owners' refs.
    #[tokio::test]
    async fn hub_advertises_only_pushers_namespace() {
        use crate::sync::wire::Message as M;
        let seed_a = [8u8; 32];
        let (_client_a, name_a, _h) = seed_local_repo(seed_a, "grove").await;

        // The hub already hosts another owner B's repo.
        let hub = Repository::memory();
        let bfp = fingerprint(&RepoSigner::from_seed([9u8; 32]).public_key());
        let tree = hub.objects.put_tree(BTreeMap::new()).await.unwrap();
        let bhead = hub.objects.put_snapshot(Snapshot { root: tree, parents: vec![], author: "b".into(), created_at: 0, message: "b".into() }).await.unwrap();
        let bname = RefName::new(format!("refs/users/{bfp}/secret/main")).unwrap();
        hub.refs.create_timeline(bname.clone(), bhead, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let hub = Arc::new(hub);
        let hub2 = hub.clone();
        let (mut a, mut b) = InProcessConn::pair();
        let server = tokio::spawn(async move { serve_hub_push(&mut b, &hub2).await });

        // Drive the handshake as owner A, then read the Welcome adverts.
        let signing = SigningKey::from_bytes(&seed_a);
        a.send(&M::HubHello { owner: signing.verifying_key().to_bytes() }).await.unwrap();
        let nonce = match a.recv().await.unwrap() { M::HubChallenge { nonce } => nonce, m => panic!("{m:?}") };
        a.send(&M::HubProof { sig: signing.sign(&challenge_message(&nonce)).to_bytes().to_vec() }).await.unwrap();
        let refs = match a.recv().await.unwrap() { M::Welcome { refs, .. } => refs, m => panic!("{m:?}") };
        // A's handshake must not reveal B's ref.
        assert!(refs.iter().all(|r| !r.name.as_str().contains(&bfp)), "leaked another owner's refs: {:?}", refs.iter().map(|r| r.name.as_str().to_string()).collect::<Vec<_>>());

        // finish the exchange so the server task completes cleanly
        let _ = name_a;
        a.send(&M::Pack(crate::store::pack::PackBuilder::new().finish().unwrap().0)).await.unwrap();
        a.send(&M::RefUpdate(vec![])).await.unwrap();
        let _ = a.recv().await;
        let _ = server.await;
    }

    #[tokio::test]
    async fn proof_signed_by_wrong_key_is_refused() {
        // The client claims owner = seed's key but signs with a different key by
        // driving the handshake manually.
        let owner = RepoSigner::from_seed([4u8; 32]).public_key();
        let wrong = SigningKey::from_bytes(&[5u8; 32]);
        let hub = Repository::memory();
        let (mut a, mut b) = InProcessConn::pair();
        let hub = Arc::new(hub);
        let hub2 = hub.clone();
        let server = tokio::spawn(async move { serve_hub_push(&mut b, &hub2).await });

        a.send(&Message::HubHello { owner }).await.unwrap();
        let nonce = match a.recv().await.unwrap() { Message::HubChallenge { nonce } => nonce, m => panic!("{m:?}") };
        let bad_sig = wrong.sign(&challenge_message(&nonce)).to_bytes().to_vec();
        a.send(&Message::HubProof { sig: bad_sig }).await.unwrap();
        // The hub rejects with an Error and the server task errors.
        assert!(matches!(a.recv().await.unwrap(), Message::Error(_)), "expected auth rejection");
        assert!(server.await.unwrap().is_err(), "server errors on bad proof");
    }

    #[tokio::test]
    async fn push_outside_owner_namespace_is_denied() {
        // Authenticate correctly as `seed`, but try to push a timeline under a
        // DIFFERENT owner's namespace. The scoped accessor must refuse it.
        let seed = [6u8; 32];
        let other_fp = fingerprint(&RepoSigner::from_seed([7u8; 32]).public_key());
        let client = Repository::memory();
        let tree = client.objects.put_tree(BTreeMap::new()).await.unwrap();
        let head = client.objects.put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "c".into() }).await.unwrap();
        let victim = RefName::new(format!("refs/users/{other_fp}/steal/main")).unwrap();
        client.refs.create_timeline(victim.clone(), head, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let hub = Repository::memory();
        let (mut a, mut b) = InProcessConn::pair();
        let hub = Arc::new(hub);
        let hub2 = hub.clone();
        let server = tokio::spawn(async move { serve_hub_push(&mut b, &hub2).await });
        let results = hub_push(&mut a, &client, &seed, "hub", &[(victim.clone(), victim.clone())]).await.unwrap();
        server.await.unwrap().unwrap();

        assert!(matches!(&results[0].status, RefApplyStatus::Denied(_)), "cross-namespace push refused: {:?}", results[0].status);
        let _ = Object::Snapshot; // silence unused import in some cfgs
        assert!(hub.refs.get_timeline(&victim).unwrap().is_none(), "victim namespace untouched");
    }
}
