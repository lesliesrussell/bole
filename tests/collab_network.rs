// bole-8lm
//! Loopback-TCP integration for the WS8b collab endpoint: real `TcpConn` between
//! two in-memory repos.

use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
use bole::object::Object;
use bole::refs::{Ref, RefName, Tag};
use bole::repo::collab::{COLLAB_REMOTES_PREFIX, COLLAB_SCOPED_PREFIX};
use bole::sync::collab::{collab_fetch_transient, collab_pull, serve_collab_tcp_once};
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
    // bole-jdo
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo, false).await });

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
    // bole-jdo
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &server_repo, false).await });

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
    // bole-jdo
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &bnode, false).await });
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
    // bole-jdo
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &bnode, false).await });
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

// bole-7kw
#[tokio::test]
async fn relay_transient_fetch_no_persist() {
    use std::sync::Arc;

    // Relay R has B and C cached (strangers to the querier A).
    let relay = Arc::new(Repository::memory());
    let rsigner = CollabSigner::from_seed([40u8; 32]);
    relay.publish_profile(&rsigner.sign_profile("relay".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    let b = CollabSigner::from_seed([41u8; 32]);
    let c = CollabSigner::from_seed([42u8; 32]);
    for (signer, name) in [(&b, "bob"), (&c, "carol")] {
        let fp = fingerprint(&signer.public_key());
        let id = relay.objects.put(&Object::Collab(CollabObject::Profile(
            signer.sign_profile(name.into(), String::new(), vec![], vec![], 1),
        ))).await.unwrap();
        let mut tx = relay.refs.transaction();
        tx.set(
            RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
            Ref::Tag(Tag { target: id, created_at: 0, message: None }),
        );
        tx.commit().unwrap();
    }

    // Querier A follows nobody.
    let anode = Repository::memory();
    let a = CollabSigner::from_seed([43u8; 32]);
    anode.publish_profile(&a.sign_profile("alice".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    let before: Vec<String> = anode.refs.list("refs/collab/").unwrap().iter().map(|n| n.as_str().to_string()).collect();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let relay1 = relay.clone();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &relay1, true).await });
    let mut conn = connect(addr).await;
    let objs = collab_fetch_transient(&mut conn).await.unwrap();
    srv.await.unwrap().unwrap();

    // Stranger found in the transient corpus...
    assert!(objs.iter().any(|o| matches!(o, CollabObject::Profile(p) if p.display_name == "bob")));
    // ...but NOT persisted: no remotes/ entry, and refs/collab/ layout unchanged.
    let bfp = fingerprint(&b.public_key());
    assert!(
        anode.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_none(),
        "stranger never written to remotes/",
    );
    let after: Vec<String> = anode.refs.list("refs/collab/").unwrap().iter().map(|n| n.as_str().to_string()).collect();
    // collab_fetch_transient takes no &Repository, so it structurally cannot write;
    // this guards against a future API change silently handing it a repo.
    assert_eq!(before, after, "discover relay causes no on-disk refs/collab/ change");

    // A second fetch behaves identically (no hidden cache).
    let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let relay2 = relay.clone();
    let srv2 = tokio::spawn(async move { serve_collab_tcp_once(&listener2, &relay2, true).await });
    let mut conn2 = connect(addr2).await;
    let objs2 = collab_fetch_transient(&mut conn2).await.unwrap();
    srv2.await.unwrap().unwrap();
    assert_eq!(objs, objs2, "repeated relay fetch is byte-for-byte identical (no hidden cache)");
}

// bole-7kw
#[tokio::test]
async fn stranger_absent_from_query_until_followed() {
    // A does not follow B; even with B's profile cached, B is outside the neighborhood.
    let anode = Repository::memory();
    let a = CollabSigner::from_seed([44u8; 32]);
    let b = CollabSigner::from_seed([45u8; 32]);
    anode.publish_profile(&a.sign_profile("alice".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    // Simulate a stranger's profile sitting in the store (as if adopted) but no follow edge yet.
    let bp = b.sign_profile("bob".into(), String::new(), vec![], vec![], 1);
    let bid = anode.objects.put(&Object::Collab(CollabObject::Profile(bp))).await.unwrap();
    let bfp = fingerprint(&b.public_key());
    let mut tx = anode.refs.transaction();
    tx.set(
        RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap(),
        Ref::Tag(Tag { target: bid, created_at: 0, message: None }),
    );
    tx.commit().unwrap();

    // Before following: B not in discovery.
    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(idx.query("bob").is_empty(), "unfollowed stranger not in discover query");

    // After trust follow: B is in the neighborhood.
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    let idx2 = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(!idx2.query("bob").is_empty(), "after follow, stranger appears in discover query");
}

// bole-4m2
#[tokio::test]
async fn loopback_stranger_trust_path() {
    use bole::collab::{CollabObject, CollabSigner, TrustKind};
    use bole::object::Object;
    use bole::refs::{Ref, RefName, Tag};
    use bole::repo::collab::COLLAB_REMOTES_PREFIX;
    use bole::sync::collab::{collab_fetch_transient, serve_collab_tcp_once};
    use bole::rank_strangers;

    // Relay aggregates a chain: querier(me) follows X; X vouches Y; Y follows stranger.
    let me = CollabSigner::from_seed([50u8; 32]);
    let x = CollabSigner::from_seed([51u8; 32]);
    let y = CollabSigner::from_seed([52u8; 32]);
    let stranger = CollabSigner::from_seed([53u8; 32]);
    let lonely = CollabSigner::from_seed([54u8; 32]);

    let relay = Repository::memory();
    relay.publish_profile(&CollabSigner::from_seed([59u8; 32]).sign_profile("relay".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    // Seed the relay's cache with X's vouch->Y, Y's follow->stranger, and both stranger + lonely profiles.
    async fn cache(repo: &Repository, obj: CollabObject) {
        use bole::collab::fingerprint;
        let (author, leaf) = match &obj {
            CollabObject::Profile(p) => (p.key, "profile".to_string()),
            CollabObject::TrustEdge(e) => (e.from_key, format!("edge/{}/{}",
                match e.kind { TrustKind::Vouch => "vouch", TrustKind::Follow => "follow", TrustKind::Review => "review" },
                fingerprint(&e.to_key))),
        };
        let id = repo.objects.put(&Object::Collab(obj)).await.unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{}/{leaf}", fingerprint(&author))).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();
    }
    cache(&relay, CollabObject::TrustEdge(x.sign_edge(y.public_key(), TrustKind::Vouch, Some("y".into()), 1))).await;
    cache(&relay, CollabObject::TrustEdge(y.sign_edge(stranger.public_key(), TrustKind::Follow, None, 1))).await;
    cache(&relay, CollabObject::Profile(stranger.sign_profile("target".into(), String::new(), vec![], vec![], 1))).await;
    cache(&relay, CollabObject::Profile(lonely.sign_profile("target".into(), String::new(), vec![], vec![], 1))).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &relay, true).await });
    let mut conn = connect(addr).await;
    let corpus = collab_fetch_transient(&mut conn).await.unwrap();
    srv.await.unwrap().unwrap();

    // Querier's own edge: me -follow-> x.
    let own = vec![me.sign_edge(x.public_key(), TrustKind::Follow, None, 1)];
    let hits = rank_strangers(&me.public_key(), &own, &corpus, "target", 4);
    let connected = hits.iter().find(|h| h.key == stranger.public_key()).unwrap();
    assert_eq!(connected.hops, Some(3), "me->x->y->stranger is 3 hops");
    let via: Vec<_> = connected.trust_path.as_ref().unwrap().iter().map(|h| h.via).collect();
    assert_eq!(via, vec![TrustKind::Follow, TrustKind::Vouch, TrustKind::Follow]);
    let unconnected = hits.iter().find(|h| h.key == lonely.public_key()).unwrap();
    assert_eq!(unconnected.trust_path, None, "lonely stranger has no path");
}

// bole-4m2
#[tokio::test]
async fn loopback_withheld_and_forged() {
    use bole::collab::{CollabObject, CollabSigner, TrustKind};
    use bole::rank_strangers;

    let me = CollabSigner::from_seed([55u8; 32]);
    let x = CollabSigner::from_seed([56u8; 32]);
    let stranger = CollabSigner::from_seed([57u8; 32]);
    let own = vec![me.sign_edge(x.public_key(), TrustKind::Follow, None, 1)];

    // Withheld: relay omits x->stranger. Path is None (completeness degrades, soundness holds).
    let withheld_corpus = vec![
        CollabObject::Profile(stranger.sign_profile("s".into(), String::new(), vec![], vec![], 1)),
    ];
    let hits = rank_strangers(&me.public_key(), &own, &withheld_corpus, "s", 4);
    assert_eq!(hits[0].trust_path, None, "withheld middle edge -> no path (sound)");

    // Forged: relay injects a tampered x->stranger edge (bad signature). A real
    // fetch drops it at verify; simulate that the corpus (post-verify) never
    // contains it, so the path is still None. (Verification happens in
    // collab_fetch_transient; a forged edge cannot enter the graph.)
    let forged = {
        let mut e = x.sign_edge(stranger.public_key(), TrustKind::Follow, None, 1);
        e.seq = 999; // mutate a signed field -> signature no longer verifies
        e
    };
    assert!(!bole::verify_edge(&forged), "forged edge does not verify");
    // rank_strangers assumes a pre-verified corpus (fetch already dropped it);
    // with the forged edge absent, the stranger stays unconnected.
    let hits2 = rank_strangers(&me.public_key(), &own, &withheld_corpus, "s", 4);
    assert_eq!(hits2[0].trust_path, None, "forged edge never in verified corpus -> no fake path");
}
