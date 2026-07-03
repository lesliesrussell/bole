// bole-40q
//! End-to-end: three sovereign in-memory nodes, follow edges, and the
//! discovery invariants — discoverable within depth, invisible beyond the hop
//! limit, scoped objects never discoverable, unreachable peers degrade.

use std::collections::BTreeMap;

use bole::collab::discovery::gather;
use bole::collab::trust::TrustGraph;
use bole::collab::{CollabObject, CollabSigner, Key, TrustKind};
use bole::object::Object;
use bole::refs::{Ref, RefName, Tag};
use bole::repo::collab::COLLAB_SCOPED_PREFIX;
use bole::Repository;

async fn node(seed: u8, name: &str) -> (Repository, CollabSigner, Key) {
    let repo = Repository::memory();
    let signer = CollabSigner::from_seed([seed; 32]);
    let key = signer.public_key();
    repo.publish_profile(&signer.sign_profile(name.into(), String::new(), vec![], vec![], 1))
        .await
        .unwrap();
    (repo, signer, key)
}

#[tokio::test]
async fn three_node_discovery_within_depth() {
    // a -follow-> b -follow-> c
    let (a_repo, a, ak) = node(1, "alice").await;
    let (b_repo, b, bk) = node(2, "bob").await;
    let (c_repo, _c, ck) = node(3, "carol").await;

    // a's local trust view: it follows b; it also knows (from b) that b follows c.
    let graph = TrustGraph::from_edges(vec![
        a.sign_edge(bk, TrustKind::Follow, None, 1),
        b.sign_edge(ck, TrustKind::Follow, None, 1),
    ]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);
    sources.insert(ck, &c_repo);

    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(!idx.query("bob").is_empty(), "b discoverable at depth 1");
    assert!(!idx.query("carol").is_empty(), "c discoverable at depth 2");
}

#[tokio::test]
async fn beyond_hop_limit_invisible() {
    let (a_repo, a, ak) = node(4, "alice").await;
    let (b_repo, b, bk) = node(5, "bob").await;
    let (c_repo, _c, ck) = node(6, "carol").await;
    let graph = TrustGraph::from_edges(vec![
        a.sign_edge(bk, TrustKind::Follow, None, 1),
        b.sign_edge(ck, TrustKind::Follow, None, 1),
    ]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);
    sources.insert(ck, &c_repo);

    // hops = 1: carol (2 hops) must be invisible.
    let idx = gather(ak, &a_repo, &graph, 1, &sources).await.unwrap();
    assert!(!idx.query("bob").is_empty(), "b still visible at depth 1");
    assert!(idx.query("carol").is_empty(), "c beyond hop limit must be invisible");
}

#[tokio::test]
async fn scoped_never_discoverable_e2e() {
    let (a_repo, a, ak) = node(7, "alice").await;
    let (b_repo, _b, bk) = node(8, "bob").await;

    // Bob pins a SCOPED profile (a future capability-scoped object).
    let secret_signer = CollabSigner::from_seed([88u8; 32]);
    let scoped = secret_signer.sign_profile("top-secret".into(), String::new(), vec![], vec![], 1);
    let id = b_repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
    let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/secret");
    let mut tx = b_repo.refs.transaction();
    tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
    tx.commit().unwrap();

    let graph = TrustGraph::from_edges(vec![a.sign_edge(bk, TrustKind::Follow, None, 1)]);
    let mut sources: BTreeMap<Key, &Repository> = BTreeMap::new();
    sources.insert(bk, &b_repo);

    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(idx.query("top-secret").is_empty(), "scoped object must never be discoverable");
    assert!(!idx.query("bob").is_empty(), "bob's public profile still discoverable");
}

#[tokio::test]
async fn unreachable_peer_degrades_gracefully() {
    let (a_repo, a, ak) = node(9, "alice").await;
    let (_b_repo, _b, bk) = node(10, "bob").await;

    // a follows b, but b's source is absent from the map (unreachable).
    let graph = TrustGraph::from_edges(vec![a.sign_edge(bk, TrustKind::Follow, None, 1)]);
    let sources: BTreeMap<Key, &Repository> = BTreeMap::new();

    // Must not error; just yields a staler (b-less) index.
    let idx = gather(ak, &a_repo, &graph, 2, &sources).await.unwrap();
    assert!(idx.query("bob").is_empty(), "unreachable peer simply absent");
    assert!(!idx.query("alice").is_empty(), "own profile still present");
}
