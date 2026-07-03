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
