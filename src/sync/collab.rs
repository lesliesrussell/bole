// bole-g7i
//! The collaboration-serve endpoint (WS8b): serves ONLY the public collab
//! namespace (`refs/collab/public/**`) over the WS5 wire, anonymously and
//! read-only. Never advertises any other ref, so scoped objects cannot leak.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::repo::collab::COLLAB_PUBLIC_PREFIX;
use crate::repo::Repository;
use crate::sync::negotiate;
use crate::sync::session::build_pack;
use crate::sync::transport::Conn;
use crate::sync::wire::{CapSet, Intent, Message, RefAdvert, PROTO_VERSION};

// bole-g7i
/// Advertises exactly the refs under `refs/collab/public/` — the entire public
/// collab surface, and nothing else. This is the single M2 enforcement point.
pub fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
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
    let refs = collab_adverts(repo)?;
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

        let adverts = collab_adverts(&repo).unwrap();
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
}
