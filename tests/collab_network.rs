// bole-8lm
//! Loopback-TCP integration for the WS8b collab endpoint: real `TcpConn` between
//! two in-memory repos.

use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
use bole::object::Object;
use bole::refs::{Ref, RefName, Tag};
use bole::repo::collab::{COLLAB_REMOTES_PREFIX, COLLAB_SCOPED_PREFIX};
use bole::sync::collab::{collab_pull, serve_collab_tcp_once};
use bole::Repository;
use tokio::net::{TcpListener, TcpStream};

async fn connect(addr: std::net::SocketAddr) -> bole::sync::transport::TcpConn {
    let stream = TcpStream::connect(addr).await.unwrap();
    bole::sync::transport::TcpConn::new(stream)
}

#[tokio::test]
async fn loopback_pull_roundtrip() {
    let server_repo = Repository::memory();
    let b = CollabSigner::from_seed([21u8; 32]);
    let c = CollabSigner::from_seed([22u8; 32]);
    server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    server_repo.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo).await });

    let client_repo = Repository::memory();
    let mut conn = connect(addr).await;
    let peer = collab_pull(&mut conn, &client_repo).await.unwrap();
    srv.await.unwrap().unwrap();

    assert_eq!(peer, b.public_key());
    let names = client_repo.refs.list(&format!("{COLLAB_REMOTES_PREFIX}{}/", fingerprint(&b.public_key()))).unwrap();
    assert!(names.iter().any(|n| n.as_str().contains("/profile")));
    assert!(names.iter().any(|n| n.as_str().contains("/edge/")));
}

#[tokio::test]
async fn loopback_scoped_never_pulled() {
    let server_repo = Repository::memory();
    let b = CollabSigner::from_seed([23u8; 32]);
    server_repo.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    // Pin a scoped object on the server.
    let scoped = b.sign_profile("secret".into(), String::new(), vec![], vec![], 9);
    let id = server_repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
    let mut tx = server_repo.refs.transaction();
    tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
           Ref::Tag(Tag { target: id, created_at: 0, message: None }));
    tx.commit().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo).await });

    let client_repo = Repository::memory();
    let mut conn = connect(addr).await;
    collab_pull(&mut conn, &client_repo).await.unwrap();
    srv.await.unwrap().unwrap();

    // The scoped object's id must not be present locally under any tracking ref,
    // and (since it was never advertised) not fetched at all.
    let all = client_repo.refs.list(COLLAB_REMOTES_PREFIX).unwrap();
    assert!(all.iter().all(|n| !n.as_str().contains("secret")));
    // The only tracked profile is bob's public one (seq 1), not the scoped seq-9 one.
    let fp = fingerprint(&b.public_key());
    let prof_ref = RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap();
    let tag = client_repo.refs.get_tag(&prof_ref).unwrap().unwrap();
    match client_repo.objects.get(&tag.target).await.unwrap().unwrap() {
        Object::Collab(CollabObject::Profile(p)) => assert_eq!(p.seq, 1),
        other => panic!("expected profile, got {other:?}"),
    }
}

// bole-95v
async fn seed_profile(node: &bole::Repository, who: &CollabSigner, name: &str) {
    node.publish_profile(&who.sign_profile(name.into(), String::new(), vec![], vec![], 1)).await.unwrap();
}

#[tokio::test]
async fn loopback_cache_forward_depth2() {
    // B follows C and has C cached. A follows B and pulls B; A must gain C at depth 2.
    let bnode = Repository::memory();
    let b = CollabSigner::from_seed([30u8; 32]);
    let c = CollabSigner::from_seed([31u8; 32]);
    seed_profile(&bnode, &b, "bob").await;
    bnode.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    // cache C's profile on B
    let cfp = fingerprint(&c.public_key());
    let id = bnode.objects.put(&Object::Collab(CollabObject::Profile(
        c.sign_profile("cee".into(), String::new(), vec![], vec![], 1),
    ))).await.unwrap();
    let mut tx = bnode.refs.transaction();
    tx.set(
        RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
        Ref::Tag(Tag { target: id, created_at: 0, message: None }),
    );
    tx.commit().unwrap();

    let anode = Repository::memory();
    let a = CollabSigner::from_seed([32u8; 32]);
    seed_profile(&anode, &a, "alice").await;
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &bnode).await });
    let mut conn = connect(addr).await;
    collab_pull(&mut conn, &anode).await.unwrap();
    srv.await.unwrap().unwrap();

    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    let cee = idx.query("cee");
    assert_eq!(cee.len(), 1, "C discoverable via cache-forward");
    assert_eq!(cee[0].distance, 2);
    assert_eq!(cee[0].trust_path, vec![a.public_key(), b.public_key(), c.public_key()]);
}

#[tokio::test]
async fn loopback_over_depth_excluded() {
    // B follows C only (NOT D). B has BOTH C and D cached.
    // A follows B, pulls B: A must get C (depth-2) but never D (depth-3).
    let bnode = Repository::memory();
    let b = CollabSigner::from_seed([33u8; 32]);
    let c = CollabSigner::from_seed([34u8; 32]);
    let d = CollabSigner::from_seed([35u8; 32]);
    seed_profile(&bnode, &b, "bob").await;
    bnode.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    // cache both C and D profiles on B
    for (signer, name) in [(&c, "cee"), (&d, "dee")] {
        let fp = fingerprint(&signer.public_key());
        let id = bnode.objects.put(&Object::Collab(CollabObject::Profile(
            signer.sign_profile(name.into(), String::new(), vec![], vec![], 1),
        ))).await.unwrap();
        let mut tx = bnode.refs.transaction();
        tx.set(
            RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
            Ref::Tag(Tag { target: id, created_at: 0, message: None }),
        );
        tx.commit().unwrap();
    }

    let anode = Repository::memory();
    let a = CollabSigner::from_seed([36u8; 32]);
    seed_profile(&anode, &a, "alice").await;
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &bnode).await });
    let mut conn = connect(addr).await;
    collab_pull(&mut conn, &anode).await.unwrap();
    srv.await.unwrap().unwrap();

    // D was never advertised by B (D not in B's follow set), so A never received it.
    let dfp = fingerprint(&d.public_key());
    assert!(
        anode.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{dfp}/profile")).unwrap()).unwrap().is_none(),
        "D never forwarded (over-depth)",
    );
    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(idx.query("dee").is_empty(), "D never surfaces in discovery");
    assert!(!idx.query("cee").is_empty(), "C still reachable at depth 2");
}
