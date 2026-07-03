// bole-g7i
//! The collaboration-serve endpoint (WS8b): serves ONLY the public collab
//! namespace (`refs/collab/public/**`) over the WS5 wire, anonymously and
//! read-only. Never advertises any other ref, so scoped objects cannot leak.

use std::collections::HashSet;

use crate::collab::{fingerprint, verify_edge, verify_profile, CollabObject, Key, TrustKind};
use crate::error::{Error, Result};
use crate::object::Object;
use crate::refs::{Ref, RefName, Tag};
use crate::repo::collab::{kind_seg, COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX};
use crate::repo::Repository;
use crate::store::pack::decode_pack;
use crate::sync::negotiate;
use crate::sync::session::build_pack;
use crate::sync::transport::Conn;
use crate::sync::wire::{CapSet, Intent, Message, RefAdvert, PROTO_VERSION};

// bole-0nk
/// Advertises the node's own public refs (`refs/collab/public/**`) plus the
/// cached refs (`refs/collab/remotes/<fp>/**`) of authors this node DIRECTLY
/// follows — and nothing else. Serve horizon: re-serve verified public state for
/// authors you directly follow, and for no others. Never advertises `scoped/`.
pub async fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    // Own authored public objects.
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
        }
    }
    // Cached objects of directly-followed authors, keyed by author fingerprint.
    for e in repo.public_edges().await? {
        if e.kind == TrustKind::Follow {
            let fp = fingerprint(&e.to_key);
            let prefix = format!("{COLLAB_REMOTES_PREFIX}{fp}/");
            for name in repo.refs.list(&prefix)? {
                if let Some(tag) = repo.refs.get_tag(&name)? {
                    out.push(RefAdvert { name, target: tag.target, is_timeline: false });
                }
            }
        }
    }
    Ok(out)
}

// bole-g7i
/// Read-only, anonymous responder for the collaboration endpoint. Advertises only
/// the public collab refs, then serves the requested object closure. Never
/// accepts pushes; never advertises anything outside `refs/collab/public/`.
pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository) -> Result<()> {
    match conn.recv().await? {
        Message::Hello { intent: Intent::Fetch, .. } | Message::Hello { intent: Intent::Clone, .. } => {}
        Message::Hello { intent: Intent::Push, .. } => {
            conn.send(&Message::Error("collab endpoint is read-only".into())).await?;
            return Err(Error::Storage("collab: push not permitted".into()));
        }
        _ => {
            conn.send(&Message::Error("expected Hello".into())).await?;
            return Err(Error::Storage("collab: expected Hello".into()));
        }
    }
    let refs = collab_adverts(repo).await?;
    let authorized: HashSet<_> = refs.iter().map(|r| r.target).collect();
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs }).await?;
    let (want, have) = match conn.recv().await? {
        Message::HaveWant { want, have } => (want, have),
        _ => return Err(Error::Storage("collab: expected HaveWant".into())),
    };
    // Never trust client-named roots: only advertised (public) targets are servable.
    let want: Vec<_> = want.into_iter().filter(|w| authorized.contains(w)).collect();
    let have: HashSet<_> = have.into_iter().collect();
    let missing = negotiate::missing_closure(repo, &want, &have).await?;
    let pack = build_pack(repo, &missing).await?;
    conn.send(&Message::Pack(pack)).await?;
    conn.send(&Message::Done).await?;
    Ok(())
}

// bole-x5u
/// Returns `true` iff the collab object's signature verifies against its embedded author key.
fn verified(obj: &CollabObject) -> bool {
    match obj {
        CollabObject::Profile(p) => verify_profile(p),
        CollabObject::TrustEdge(e) => verify_edge(e),
    }
}

// bole-x5u
/// Returns the author key of a collab object (the identity that signed it).
fn author(obj: &CollabObject) -> Key {
    match obj {
        CollabObject::Profile(p) => p.key,
        CollabObject::TrustEdge(e) => e.from_key,
    }
}

// bole-x5u
/// Pulls a peer's public collab objects over `conn`, verifying every signature
/// (fail-closed) and keeping only those authored by the peer's own profile key
/// (serve-own-only). Survivors are pinned under
/// `refs/collab/remotes/<peerkey-fp>/`, never merged into the local public set.
/// Returns the peer's key. Errors if the peer served no valid profile.
pub async fn collab_pull(conn: &mut dyn Conn, repo: &Repository) -> Result<Key> {
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Fetch,
    })
    .await?;
    let refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let want: Vec<_> = refs.iter().map(|r| r.target).collect();
    // bole-g87: send empty have to avoid leaking local store metadata to untrusted peers
    let have: Vec<_> = vec![];
    conn.send(&Message::HaveWant { want, have }).await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
    match conn.recv().await? {
        Message::Done => {}
        other => return Err(Error::Storage(format!("collab: expected Done, got {other:?}"))),
    }
    for (_id, canonical) in decode_pack(&pack)? {
        repo.objects.put_raw(&canonical).await?;
    }

    // Resolve advertised objects, verify signatures, and identify the peer (the
    // single Profile's author key). Fail-closed: drop any object that doesn't verify.
    let mut resolved: Vec<(RefName, CollabObject)> = Vec::new();
    for r in &refs {
        if let Some(Object::Collab(obj)) = repo.objects.get(&r.target).await? {
            if verified(&obj) {
                resolved.push((r.name.clone(), obj));
            }
        }
    }
    let peer = resolved
        .iter()
        .find_map(|(_, o)| {
            if matches!(o, CollabObject::Profile(_)) {
                Some(author(o))
            } else {
                None
            }
        })
        .ok_or_else(|| Error::Storage("collab: peer served no valid profile".into()))?;

    let fp = fingerprint(&peer);
    let mut tx = repo.refs.transaction();
    for (_, obj) in &resolved {
        if author(obj) != peer {
            continue; // serve-own-only: drop objects not authored by the peer
        }
        let tracking = match obj {
            CollabObject::Profile(_) => format!("{COLLAB_REMOTES_PREFIX}{fp}/profile"),
            CollabObject::TrustEdge(e) => {
                format!(
                    "{COLLAB_REMOTES_PREFIX}{fp}/edge/{}/{}",
                    kind_seg(e.kind),
                    fingerprint(&e.to_key),
                )
            }
        };
        let target = repo
            .objects
            .put(&Object::Collab(obj.clone()))
            .await?;
        tx.set(RefName::new(tracking)?, Ref::Tag(Tag { target, created_at: 0, message: None }));
    }
    tx.commit()?;
    Ok(peer)
}

// bole-8lm
/// Accepts one TCP connection and serves the collab endpoint on it.
pub async fn serve_collab_tcp_once(
    listener: &tokio::net::TcpListener,
    repo: &Repository,
) -> Result<()> {
    let (stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let mut conn = crate::sync::transport::TcpConn::new(stream);
    serve_collab(&mut conn, repo).await
}

// bole-g7i
#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::CollabSigner;
    use crate::object::Object;
    use crate::refs::{Ref, RefName, Tag};
    use crate::repo::collab::COLLAB_SCOPED_PREFIX;

    #[tokio::test]
    async fn collab_adverts_exclude_scoped() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([1u8; 32]);
        repo.publish_profile(&a.sign_profile("A".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // Pin a scoped object directly.
        let scoped = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(crate::collab::CollabObject::Profile(scoped))).await.unwrap();
        let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/x");
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).await.unwrap();
        assert!(adverts.iter().all(|r| r.name.as_str().starts_with(COLLAB_PUBLIC_PREFIX)));
        assert!(adverts.iter().any(|r| r.name.as_str().contains("/public/profile/")));
        assert!(!adverts.iter().any(|r| r.name.as_str().contains("/scoped/")));
    }

    #[tokio::test]
    async fn serve_collab_never_offers_scoped() {
        use crate::sync::transport::InProcessConn;
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([2u8; 32]);
        repo.publish_profile(&a.sign_profile("A".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let scoped = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(crate::collab::CollabObject::Profile(scoped))).await.unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut server, mut client) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut server, &repo).await });
        // Minimal client: Hello(Fetch) -> read Welcome adverts.
        client.send(&Message::Hello { proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: CapSet::EMPTY, intent: Intent::Fetch }).await.unwrap();
        let welcome = client.recv().await.unwrap();
        let refs = match welcome { Message::Welcome { refs, .. } => refs, other => panic!("expected Welcome, got {other:?}") };
        assert!(refs.iter().all(|r| r.name.as_str().starts_with(COLLAB_PUBLIC_PREFIX)));
        assert!(!refs.iter().any(|r| r.name.as_str().contains("/scoped/")));
        // Drain the rest so the server task finishes cleanly.
        client.send(&Message::HaveWant { want: refs.iter().map(|r| r.target).collect(), have: vec![] }).await.unwrap();
        let _pack = client.recv().await.unwrap();
        let _done = client.recv().await.unwrap();
        srv.await.unwrap().unwrap();
    }

    // bole-x5u
    #[tokio::test]
    async fn pull_stores_under_remote_prefix() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::fingerprint;
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        // Server B publishes a profile + a follow edge.
        let server_repo = Repository::memory();
        let b = CollabSigner::from_seed([3u8; 32]);
        let c = CollabSigner::from_seed([4u8; 32]);
        server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server_repo.publish_edge(&b.sign_edge(c.public_key(), crate::collab::TrustKind::Follow, None, 1)).await.unwrap();

        // Client A pulls B.
        let client_repo = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo).await });
        let peer = collab_pull(&mut cl, &client_repo).await.unwrap();
        srv.await.unwrap().unwrap();

        assert_eq!(peer, b.public_key());
        let fp = fingerprint(&b.public_key());
        let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{fp}/")).unwrap();
        assert!(names.iter().any(|n| n.as_str().contains("/profile")), "peer profile tracked");
        assert!(names.iter().any(|n| n.as_str().contains("/edge/")), "peer edge tracked");
    }

    // bole-x5u
    #[tokio::test]
    async fn pull_drops_tampered_object() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX};

        // Server B has a VALID profile plus a TAMPERED edge pinned under public.
        let server_repo = Repository::memory();
        let b = CollabSigner::from_seed([5u8; 32]);
        let c = CollabSigner::from_seed([6u8; 32]);
        server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let mut bad = b.sign_edge(c.public_key(), crate::collab::TrustKind::Follow, None, 1);
        bad.kind = crate::collab::TrustKind::Vouch; // invalidates signature
        let bad_id = server_repo.objects.put(&Object::Collab(CollabObject::TrustEdge(bad))).await.unwrap();
        let mut tx = server_repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_PUBLIC_PREFIX}edge/bad")).unwrap(),
               Ref::Tag(Tag { target: bad_id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let client_repo = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo).await });
        collab_pull(&mut cl, &client_repo).await.unwrap();
        srv.await.unwrap().unwrap();

        let fp = fingerprint(&b.public_key());
        let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{fp}/")).unwrap();
        assert!(names.iter().any(|n| n.as_str().contains("/profile")), "valid profile kept");
        assert!(!names.iter().any(|n| n.as_str().contains("/edge/")), "tampered edge dropped");
    }

    // bole-0nk
    #[tokio::test]
    async fn adverts_include_followed_remote() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([1u8; 32]);
        let c = CollabSigner::from_seed([2u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // I follow C.
        repo.publish_edge(&me.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // I have C cached under remotes/<Cfp>/profile (as a pull would have stored).
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).await.unwrap();
        assert!(adverts.iter().any(|r| r.name.as_str() == format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")),
            "followed author's cached profile is advertised");
        assert!(adverts.iter().any(|r| r.name.as_str().contains("/public/profile/")), "own profile still advertised");
    }

    // bole-0nk
    #[tokio::test]
    async fn adverts_exclude_unfollowed_remote() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([3u8; 32]);
        let stranger = CollabSigner::from_seed([4u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // I do NOT follow the stranger, but I have their profile cached.
        let sp = stranger.sign_profile("s".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let sfp = fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).await.unwrap();
        assert!(!adverts.iter().any(|r| r.name.as_str().contains(&sfp)),
            "unfollowed author's cache must NOT be advertised");
    }

    // bole-x5u
    #[tokio::test]
    async fn pull_errors_with_no_valid_profile() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::CollabObject;
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_PUBLIC_PREFIX;
        use crate::collab::CollabSigner;

        let server_repo = Repository::memory();
        let b = CollabSigner::from_seed([7u8; 32]);
        // Only a TAMPERED profile is served — nothing verifies.
        let mut bad = b.sign_profile("bob".into(), String::new(), vec![], vec![], 1);
        bad.display_name = "tampered".into();
        let id = server_repo.objects.put(&Object::Collab(CollabObject::Profile(bad))).await.unwrap();
        let mut tx = server_repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_PUBLIC_PREFIX}profile/x")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let client_repo = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo).await });
        let res = collab_pull(&mut cl, &client_repo).await;
        srv.await.unwrap().unwrap();
        assert!(res.is_err(), "pull must error when no valid profile is served");
    }
}
