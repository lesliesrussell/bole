// bole-18p
use async_trait::async_trait;

use crate::collab::CollabObject;
use crate::error::Result;

/// The interface a node (and, later, a relay) exposes to serve its
/// **public-labeled** collaboration objects. v1 implements only the
/// sovereign-node side (`impl for Repository`); relays are a future impl of the
/// same trait, so discovery client code needs no change when they land.
#[async_trait]
pub trait PublicObjectSource {
    /// Every public collaboration object this source is willing to serve. MUST
    /// return only objects pinned under the public prefix — never scoped objects.
    async fn public_objects(&self) -> Result<Vec<CollabObject>>;
}

// bole-3nk
use std::collections::BTreeMap;

use crate::collab::trust::TrustGraph;
use crate::collab::{Key, Profile, TrustEdge};

/// A single discovery hit: the object, the key that published it, and how far /
/// by what route it was reached. Every hit is auditable back to a key + reason.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub key: Key,
    pub object: CollabObject,
    /// 0 = held locally by root; 1 = direct follow; 2 = friend-of-friend.
    pub distance: u8,
    /// Route `root -> ... -> publisher`.
    pub trust_path: Vec<Key>,
}

/// A per-node, trust-graph-local discovery index. No global/relay index exists in
/// this slice; all queries run locally over public objects held plus public
/// objects pulled from the follow-neighborhood.
pub struct Index {
    results: Vec<DiscoveryResult>,
}

fn publisher_key(obj: &CollabObject) -> Key {
    match obj {
        CollabObject::Profile(Profile { key, .. }) => *key,
        CollabObject::TrustEdge(TrustEdge { from_key, .. }) => *from_key,
    }
}

fn recency(obj: &CollabObject) -> u64 {
    match obj {
        CollabObject::Profile(p) => p.seq,
        CollabObject::TrustEdge(e) => e.seq,
    }
}

fn matches(obj: &CollabObject, term: &str) -> bool {
    if term.is_empty() {
        return true;
    }
    match obj {
        CollabObject::Profile(p) => {
            crate::collab::fingerprint(&p.key).contains(term)
                || p.display_name.contains(term)
                || p.bio.contains(term)
                || p.dns_aliases.iter().any(|a| a.contains(term))
        }
        CollabObject::TrustEdge(e) => {
            crate::collab::fingerprint(&e.from_key).contains(term)
                || e.petname.as_deref().map(|n| n.contains(term)).unwrap_or(false)
        }
    }
}

impl Index {
    /// Builds an index from root's own public objects (distance 0) and objects
    /// pulled from follow-neighbors, each tuple `(via, distance, trust_path, objects)`.
    pub fn build(
        root: Key,
        own: Vec<CollabObject>,
        pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)>,
    ) -> Self {
        let mut results = Vec::new();
        for obj in own {
            results.push(DiscoveryResult { key: publisher_key(&obj), object: obj, distance: 0, trust_path: vec![root] });
        }
        for (_via, distance, trust_path, objects) in pulled {
            for obj in objects {
                results.push(DiscoveryResult {
                    key: publisher_key(&obj),
                    object: obj,
                    distance,
                    trust_path: trust_path.clone(),
                });
            }
        }
        Self { results }
    }

    /// Deterministic query: matches `term`, ordered by trust distance ascending
    /// then recency (`seq`) descending. No numeric trust scores.
    pub fn query(&self, term: &str) -> Vec<&DiscoveryResult> {
        let mut hits: Vec<&DiscoveryResult> = self.results.iter().filter(|r| matches(&r.object, term)).collect();
        hits.sort_by(|a, b| {
            a.distance
                .cmp(&b.distance)
                .then_with(|| recency(&b.object).cmp(&recency(&a.object)))
        });
        hits
    }
}

// bole-6hw
/// True iff a collab object's signature verifies against its embedded author key.
fn verified(obj: &CollabObject) -> bool {
    match obj {
        CollabObject::Profile(p) => crate::collab::verify_profile(p),
        CollabObject::TrustEdge(e) => crate::collab::verify_edge(e),
    }
}

/// Gathers a discovery index for `root`: root's own public objects at distance 0,
/// plus the public objects of each key in the bounded `Follow` neighborhood whose
/// source is present in `sources`. A key with no reachable source is simply
/// skipped (graceful degradation), never an error.
pub async fn gather<S: PublicObjectSource>(
    root: Key,
    own: &S,
    graph: &TrustGraph,
    hops: u8,
    sources: &BTreeMap<Key, &S>,
) -> Result<Index> {
    // bole-6hw
    let own_objs: Vec<CollabObject> = own.public_objects().await?.into_iter().filter(verified).collect();
    let neighborhood = graph.follow_neighborhood(&root, hops);
    let mut pulled = Vec::new();
    for (peer, distance) in neighborhood {
        if let Some(src) = sources.get(&peer) {
            match src.public_objects().await {
                // bole-6hw
                Ok(objs) => pulled.push((peer, distance, vec![root, peer], objs.into_iter().filter(verified).collect())),
                Err(_) => continue, // unreachable/failed peer: degrade, don't fail
            }
        }
    }
    Ok(Index::build(root, own_objs, pulled))
}

// bole-3nk
#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabObject, CollabSigner, Key, TrustGraph, TrustKind};
    use std::collections::BTreeMap;

    fn profile(signer: &CollabSigner, name: &str, seq: u64) -> CollabObject {
        CollabObject::Profile(signer.sign_profile(name.into(), String::new(), vec![], vec![], seq))
    }

    fn key_of(obj: &CollabObject) -> Key {
        match obj {
            CollabObject::Profile(p) => p.key,
            CollabObject::TrustEdge(e) => e.from_key,
        }
    }

    #[test]
    fn index_orders_by_distance_then_recency() {
        let root_s = CollabSigner::from_seed([1u8; 32]);
        let near_s = CollabSigner::from_seed([2u8; 32]);
        let far_s = CollabSigner::from_seed([3u8; 32]);

        let rk = root_s.public_key();
        // own: root's own profile (distance 0)
        let own = vec![profile(&root_s, "root", 5)];
        // pulled: near at distance 1 (seq 9), far at distance 2 (seq 1)
        let pulled = vec![
            (near_s.public_key(), 1u8, vec![rk, near_s.public_key()], vec![profile(&near_s, "near", 9)]),
            (far_s.public_key(), 2u8, vec![rk, near_s.public_key(), far_s.public_key()], vec![profile(&far_s, "far", 1)]),
        ];
        let idx = Index::build(rk, own, pulled);

        // Query matches all three "profile" objects; ordering: distance asc, then seq desc.
        let hits = idx.query("");
        let dists: Vec<u8> = hits.iter().map(|r| r.distance).collect();
        assert_eq!(dists, vec![0, 1, 2], "sorted by trust distance");
    }

    #[test]
    fn result_carries_key_and_trust_path() {
        let root_s = CollabSigner::from_seed([4u8; 32]);
        let peer_s = CollabSigner::from_seed([5u8; 32]);
        let rk = root_s.public_key();
        let pk = peer_s.public_key();

        let own = vec![];
        let pulled = vec![(pk, 1u8, vec![rk, pk], vec![profile(&peer_s, "peer", 1)])];
        let idx = Index::build(rk, own, pulled);

        let hits = idx.query("peer");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, pk, "result carries the publishing key");
        assert_eq!(hits[0].trust_path, vec![rk, pk], "result carries the trust path");
        assert_eq!(key_of(&hits[0].object), pk);
    }

    // bole-3nk
    #[test]
    fn same_distance_orders_by_recency_desc() {
        let root_s = CollabSigner::from_seed([11u8; 32]);
        let p1 = CollabSigner::from_seed([12u8; 32]);
        let p2 = CollabSigner::from_seed([13u8; 32]);
        let rk = root_s.public_key();
        // Two peers both at distance 1, seqs 3 and 9.
        let own = vec![];
        let pulled = vec![
            (p1.public_key(), 1u8, vec![rk, p1.public_key()], vec![profile(&p1, "low", 3)]),
            (p2.public_key(), 1u8, vec![rk, p2.public_key()], vec![profile(&p2, "high", 9)]),
        ];
        let idx = Index::build(rk, own, pulled);
        let hits = idx.query("");
        let seqs: Vec<u64> = hits
            .iter()
            .map(|r| match &r.object {
                CollabObject::Profile(p) => p.seq,
                CollabObject::TrustEdge(e) => e.seq,
            })
            .collect();
        assert_eq!(seqs, vec![9, 3], "within a distance, higher seq (more recent) sorts first");
    }

    // bole-6hw
    struct MockSource(Vec<CollabObject>);

    #[async_trait::async_trait]
    impl PublicObjectSource for MockSource {
        async fn public_objects(&self) -> crate::error::Result<Vec<CollabObject>> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn gather_excludes_unverified_objects() {
        let root_s = CollabSigner::from_seed([40u8; 32]);
        let peer_s = CollabSigner::from_seed([41u8; 32]);
        let rk = root_s.public_key();
        let pk = peer_s.public_key();
        let good = peer_s.sign_profile("goodname".into(), String::new(), vec![], vec![], 1);
        let mut bad = peer_s.sign_profile("origname".into(), String::new(), vec![], vec![], 2);
        bad.display_name = "tamperedname".into(); // invalidates the signature
        let own = MockSource(vec![]);
        let peer = MockSource(vec![CollabObject::Profile(good), CollabObject::Profile(bad)]);
        let graph = TrustGraph::from_edges(vec![root_s.sign_edge(pk, TrustKind::Follow, None, 1)]);
        let mut sources: BTreeMap<Key, &MockSource> = BTreeMap::new();
        sources.insert(pk, &peer);
        let idx = gather(rk, &own, &graph, 2, &sources).await.unwrap();
        assert!(!idx.query("goodname").is_empty(), "verified object is indexed");
        assert!(idx.query("tamperedname").is_empty(), "unverified (tampered) object must be excluded");
    }
}
