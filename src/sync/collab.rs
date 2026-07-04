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

// bole-jdo
/// Advertises the node's own public objects (`refs/collab/public/**`) plus cached
/// objects (`refs/collab/remotes/<fp>/**`). When `relay` is false (ordinary node),
/// only the cache of directly-followed authors is advertised (WS8c serve horizon).
/// When `relay` is true, ALL cached objects are advertised (a relay aggregates and
/// re-serves broadly). Never advertises `refs/collab/scoped/` in either mode.
pub async fn collab_adverts(repo: &Repository, relay: bool) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
        }
    }
    if relay {
        // Relay: advertise the entire cache, horizon off.
        for name in repo.refs.list(COLLAB_REMOTES_PREFIX)? {
            if let Some(tag) = repo.refs.get_tag(&name)? {
                out.push(RefAdvert { name, target: tag.target, is_timeline: false });
            }
        }
    } else {
        // Ordinary node: only directly-followed authors' cache (WS8c serve horizon).
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
    }
    Ok(out)
}

// bole-jdo
/// Read-only, anonymous responder for the collaboration endpoint. Advertises the
/// node's public collab refs plus cached refs per `collab_adverts` — when `relay`
/// is true, the whole cached aggregate; otherwise only directly-followed authors'
/// cache. Then serves the requested object closure. Never accepts pushes; never
/// advertises `refs/collab/scoped/` in any mode.
pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository, relay: bool) -> Result<()> {
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
    let refs = collab_adverts(repo, relay).await?;
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
/// Pulls a node's advertised public + followed-cache collab objects over `conn`,
/// verifying every object against its embedded author key (fail-closed; drop
/// invalid). Each survivor is filed under the puller's
/// `refs/collab/remotes/<intrinsic-author-fp>/…` by its TRUE author — the dialed
/// server's own objects and its forwarded cache alike. Returns the dialed server's
/// own key (author of a verified `public/` Profile); errors if none is served.
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
    // bole-gvj
    // The dialed server's identity is the author of a verified Profile advertised
    // under `public/` (its own), not merely the first profile in the set — several
    // cached profiles may also be present. Pin the invariant to the namespace.
    let peer = resolved
        .iter()
        .find_map(|(name, o)| {
            if name.as_str().starts_with(COLLAB_PUBLIC_PREFIX) && matches!(o, CollabObject::Profile(_)) {
                Some(author(o))
            } else {
                None
            }
        })
        .ok_or_else(|| Error::Storage("collab: peer served no valid profile".into()))?;

    // bole-gvj
    // Multi-author: file EVERY verified object under the puller's remotes namespace
    // keyed by its INTRINSIC author (server-own and forwarded-cache alike). The
    // `peer` (server's own key) is still returned for `discover pull`/`trust follow`.
    let mut tx = repo.refs.transaction();
    for (_, obj) in &resolved {
        let afp = fingerprint(&author(obj));
        let tracking = match obj {
            CollabObject::Profile(_) => format!("{COLLAB_REMOTES_PREFIX}{afp}/profile"),
            CollabObject::TrustEdge(e) => format!(
                "{COLLAB_REMOTES_PREFIX}{afp}/edge/{}/{}",
                kind_seg(e.kind),
                fingerprint(&e.to_key),
            ),
        };
        let target = repo.objects.put(&Object::Collab(obj.clone())).await?;
        tx.set(RefName::new(tracking)?, Ref::Tag(Tag { target, created_at: 0, message: None }));
    }
    tx.commit()?;
    Ok(peer)
}

// bole-jdo
/// Accepts one TCP connection and serves the collab endpoint on it.
pub async fn serve_collab_tcp_once(
    listener: &tokio::net::TcpListener,
    repo: &Repository,
    relay: bool,
) -> Result<()> {
    let (stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let mut conn = crate::sync::transport::TcpConn::new(stream);
    serve_collab(&mut conn, repo, relay).await
}

// bole-63b
/// Fetches a node's advertised public collab objects over `conn` and returns the
/// signature-verified ones, WITHOUT touching any repository. Pure fetch+verify:
/// used by relay stranger-search, where results are transient and never persisted.
/// Fail-closed: any object whose signature does not verify is dropped.
pub async fn collab_fetch_transient(conn: &mut dyn Conn) -> Result<Vec<CollabObject>> {
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
    conn.send(&Message::HaveWant { want, have: vec![] }).await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
    match conn.recv().await? {
        Message::Done => {}
        other => return Err(Error::Storage(format!("collab: expected Done, got {other:?}"))),
    }
    let mut out = Vec::new();
    for (_id, canonical) in decode_pack(&pack)? {
        if let Ok(Object::Collab(obj)) = crate::codec::deserialize(&canonical) {
            if verified(&obj) {
                out.push(obj);
            }
        }
    }
    Ok(out)
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

        // bole-jdo
        let adverts = collab_adverts(&repo, false).await.unwrap();
        assert!(!adverts.iter().any(|r| r.name.as_str().contains("/scoped/")), "scoped refs are never advertised");
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
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut server, &repo, false).await });
        // Minimal client: Hello(Fetch) -> read Welcome adverts.
        client.send(&Message::Hello { proto_min: PROTO_VERSION, proto_max: PROTO_VERSION, caps: CapSet::EMPTY, intent: Intent::Fetch }).await.unwrap();
        let welcome = client.recv().await.unwrap();
        let refs = match welcome { Message::Welcome { refs, .. } => refs, other => panic!("expected Welcome, got {other:?}") };
        assert!(!refs.iter().any(|r| r.name.as_str().contains("/scoped/")), "scoped refs are never served");
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
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo, false).await });
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
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo, false).await });
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

        // bole-jdo
        let adverts = collab_adverts(&repo, false).await.unwrap();
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

        // bole-jdo
        let adverts = collab_adverts(&repo, false).await.unwrap();
        assert!(!adverts.iter().any(|r| r.name.as_str().contains(&sfp)),
            "unfollowed author's cache must NOT be advertised");
    }

    // bole-gvj
    #[tokio::test]
    async fn pull_files_cached_by_author() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX};

        // Server B: own profile (public/), follows C, and has C cached (remotes/<Cfp>/).
        let server = Repository::memory();
        let b = CollabSigner::from_seed([10u8; 32]);
        let c = CollabSigner::from_seed([11u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        // Client A pulls B.
        let client = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, false).await });
        let peer = collab_pull(&mut cl, &client).await.unwrap();
        srv.await.unwrap().unwrap();

        assert_eq!(peer, b.public_key(), "returns the dialed server's own key");
        // B filed under remotes/<Bfp>/, C filed under remotes/<Cfp>/ — by intrinsic author.
        let bfp = fingerprint(&b.public_key());
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_some(),
            "server-own profile filed under its author");
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap()).unwrap().is_some(),
            "cached C profile filed under C, not under B");
        assert!(client.refs.list(COLLAB_PUBLIC_PREFIX).unwrap().is_empty(),
            "pull files into remotes/, never the puller's own public/");
    }

    // bole-gvj
    #[tokio::test]
    async fn pull_drops_tampered_cached() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let server = Repository::memory();
        let b = CollabSigner::from_seed([12u8; 32]);
        let c = CollabSigner::from_seed([13u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // A TAMPERED C profile cached on B.
        let mut cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        cp.display_name = "tampered".into();
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let client = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, false).await });
        collab_pull(&mut cl, &client).await.unwrap();
        srv.await.unwrap().unwrap();

        let bfp = fingerprint(&b.public_key());
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_some(),
            "valid server profile kept");
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap()).unwrap().is_none(),
            "tampered cached C profile dropped (no ref)");
    }

    // bole-jdo
    #[tokio::test]
    async fn adverts_relay_includes_unfollowed() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([90u8; 32]);
        let stranger = CollabSigner::from_seed([91u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // A stranger cached but NOT followed.
        let sp = stranger.sign_profile("s".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let sfp = fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        // relay=false → excluded (WS8c horizon); relay=true → included.
        let non_relay = collab_adverts(&repo, false).await.unwrap();
        assert!(!non_relay.iter().any(|r| r.name.as_str().contains(&sfp)), "non-relay excludes unfollowed");
        let relay = collab_adverts(&repo, true).await.unwrap();
        assert!(relay.iter().any(|r| r.name.as_str() == format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")),
            "relay advertises unfollowed cache");
    }

    // bole-jdo
    #[tokio::test]
    async fn adverts_relay_excludes_scoped() {
        use crate::collab::{CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_SCOPED_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([92u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let scoped = me.sign_profile("secret".into(), String::new(), vec![], vec![], 2);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        // Even in relay mode, scoped is never advertised.
        let relay = collab_adverts(&repo, true).await.unwrap();
        assert!(!relay.iter().any(|r| r.name.as_str().contains("/scoped/")), "relay never advertises scoped");
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
        // bole-jdo
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server_repo, false).await });
        let res = collab_pull(&mut cl, &client_repo).await;
        srv.await.unwrap().unwrap();
        assert!(res.is_err(), "pull must error when no valid profile is served");
    }

    // bole-63b
    #[tokio::test]
    async fn transient_fetch_returns_verified() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{CollabObject, CollabSigner};

        // A relay-style server with two authors cached (B own profile + C cached).
        let server = Repository::memory();
        let b = CollabSigner::from_seed([93u8; 32]);
        let c = CollabSigner::from_seed([94u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // cache C directly under remotes and serve with relay=true so it's advertised
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = crate::collab::fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(crate::refs::RefName::new(format!("{}{cfp}/profile", crate::repo::collab::COLLAB_REMOTES_PREFIX)).unwrap(),
               crate::refs::Ref::Tag(crate::refs::Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, true).await });
        let objs = collab_fetch_transient(&mut cl).await.unwrap();
        srv.await.unwrap().unwrap();

        let names: Vec<String> = objs.iter().filter_map(|o| match o {
            CollabObject::Profile(p) => Some(p.display_name.clone()),
            _ => None,
        }).collect();
        assert!(names.contains(&"bob".to_string()) && names.contains(&"cee".to_string()),
            "transient fetch returns both verified profiles");
    }

    // bole-63b
    #[tokio::test]
    async fn transient_fetch_drops_tampered() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{CollabObject, CollabSigner};

        let server = Repository::memory();
        let b = CollabSigner::from_seed([95u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // A tampered profile pinned under public/ (so it's advertised) but won't verify.
        let mut bad = b.sign_profile("origname".into(), String::new(), vec![], vec![], 2);
        bad.display_name = "tampered".into();
        let bid = server.objects.put(&Object::Collab(CollabObject::Profile(bad))).await.unwrap();
        let mut tx = server.refs.transaction();
        tx.set(crate::refs::RefName::new(format!("{}profile/bad", crate::repo::collab::COLLAB_PUBLIC_PREFIX)).unwrap(),
               crate::refs::Ref::Tag(crate::refs::Tag { target: bid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, true).await });
        let objs = collab_fetch_transient(&mut cl).await.unwrap();
        srv.await.unwrap().unwrap();

        assert!(objs.iter().all(|o| !matches!(o, CollabObject::Profile(p) if p.display_name == "tampered")),
            "tampered object is dropped fail-closed");
        assert!(objs.iter().any(|o| matches!(o, CollabObject::Profile(p) if p.display_name == "bob")),
            "valid object still returned");
    }

    // bole-su8
    #[tokio::test]
    async fn collab_adverts_exclude_relays() {
        use crate::collab::RelayPin;
        use crate::repo::collab::COLLAB_RELAYS_PREFIX;
        let repo = Repository::memory();
        // Pin a relay AND publish a public profile so adverts are non-empty.
        repo.add_relay(RelayPin { key: [5u8; 32], endpoint: "x:1".into() }).await.unwrap();
        let a = CollabSigner::from_seed([5u8; 32]);
        repo.publish_profile(&a.sign_profile("A".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        for relay in [false, true] {
            let adverts = collab_adverts(&repo, relay).await.unwrap();
            for a in &adverts {
                assert!(
                    !a.name.as_str().starts_with(COLLAB_RELAYS_PREFIX),
                    "relays/ must never be advertised (relay={relay})"
                );
            }
        }
    }
}
