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

use crate::collab::trust::{TrustGraph, TrustHop};
use crate::collab::{fingerprint, key_hex, Key, Profile, TrustEdge, TrustKind};

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

// bole-jom
/// A ranked stranger discovery hit: the publisher key, self-asserted display
/// name, and the verifiable trust path connecting the querier to this stranger
/// (`None` when unreachable within the hop bound).
#[derive(Debug, Clone)]
pub struct StrangerHit {
    pub key: Key,
    pub display_name: String,
    pub trust_path: Option<Vec<TrustHop>>,
    pub hops: Option<usize>,
    // bole-yc9x
    pub relays: Vec<Key>,
}

// bole-jom
/// Builds the combined trust graph (`own_edges` + all `TrustEdge`s in the
/// relay corpus), finds a bounded trust path from `self_key` to each
/// term-matched stranger `Profile`, and ranks: has-path > shorter >
/// vouch-containing > display_name > key fingerprint.
/// Pure and repo-free; the relay corpus is already signature-verified by the fetch.
pub fn rank_strangers(
    self_key: &Key,
    own_edges: &[TrustEdge],
    relay_corpus: &[CollabObject],
    term: &str,
    max_hops: u8,
) -> Vec<StrangerHit> {
    let mut edges: Vec<TrustEdge> = own_edges.to_vec();
    for o in relay_corpus {
        if let CollabObject::TrustEdge(e) = o {
            edges.push(e.clone());
        }
    }
    let graph = TrustGraph::from_edges(edges);

    let mut hits: Vec<StrangerHit> = Vec::new();
    for o in relay_corpus {
        if let CollabObject::Profile(p) = o {
            let term_matches = p.display_name.contains(term)
                || p.bio.contains(term)
                || p.dns_aliases.iter().any(|a| a.contains(term))
                || key_hex(&p.key).contains(term); // bole-gp0
            if !term_matches {
                continue;
            }
            let path = graph.trust_path(self_key, &p.key, max_hops);
            let hops = path.as_ref().map(|v| v.len());
            hits.push(StrangerHit {
                key: p.key,
                display_name: p.display_name.clone(),
                trust_path: path,
                hops,
                relays: Vec::new(), // bole-yc9x
            });
        }
    }

    hits.sort_by(|a, b| {
        let a_conn = a.trust_path.is_some();
        let b_conn = b.trust_path.is_some();
        b_conn
            .cmp(&a_conn)
            .then_with(|| a.hops.unwrap_or(usize::MAX).cmp(&b.hops.unwrap_or(usize::MAX)))
            .then_with(|| {
                let av = has_vouch(&a.trust_path);
                let bv = has_vouch(&b.trust_path);
                bv.cmp(&av)
            })
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| fingerprint(&a.key).cmp(&fingerprint(&b.key)))
    });
    hits
}

// bole-jom
fn has_vouch(path: &Option<Vec<TrustHop>>) -> bool {
    path.as_ref().map(|p| p.iter().any(|h| h.via == TrustKind::Vouch)).unwrap_or(false)
}

// bole-yc9x
/// Merges per-relay verified corpora and ranks strangers once (WS8e semantics),
/// attributing each hit to the relay keys that served its profile. Profiles are
/// deduped by key (highest `seq` wins); edges are deduped by content id.
pub fn rank_strangers_multi(
    self_key: &Key,
    own_edges: &[TrustEdge],
    per_relay: &[(Key, Vec<CollabObject>)],
    term: &str,
    max_hops: u8,
) -> Vec<StrangerHit> {
    use std::collections::BTreeMap;
    // Dedup profiles by key (highest seq); track serving relays per profile key.
    let mut best: BTreeMap<Key, Profile> = BTreeMap::new();
    let mut served_by: BTreeMap<Key, std::collections::BTreeSet<Key>> = BTreeMap::new();
    // Dedup edges by content id.
    let mut edges: BTreeMap<crate::object::ObjectId, TrustEdge> = BTreeMap::new();
    for (relay_key, corpus) in per_relay {
        for obj in corpus {
            match obj {
                CollabObject::Profile(p) => {
                    served_by.entry(p.key).or_default().insert(*relay_key);
                    match best.get(&p.key) {
                        Some(cur) if cur.seq >= p.seq => {}
                        _ => {
                            best.insert(p.key, p.clone());
                        }
                    }
                }
                CollabObject::TrustEdge(e) => {
                    // CONTROLLER-CORRECTED: codec::object_id takes SERIALIZED bytes (&[u8]),
                    // not an Object, and is pub(crate) (reachable here — same crate). This
                    // mirrors ObjectStore::put (store/mod.rs:41-42).
                    let wrapped = crate::object::Object::Collab(CollabObject::TrustEdge(e.clone()));
                    let bytes = crate::codec::serialize(&wrapped)
                        .expect("postcard serialization is infallible for owned data");
                    let id = crate::codec::object_id(&bytes);
                    edges.entry(id).or_insert_with(|| e.clone());
                }
            }
        }
    }
    let merged: Vec<CollabObject> = best
        .values()
        .cloned()
        .map(CollabObject::Profile)
        .chain(edges.values().cloned().map(CollabObject::TrustEdge))
        .collect();
    let mut hits = rank_strangers(self_key, own_edges, &merged, term, max_hops);
    for h in &mut hits {
        if let Some(set) = served_by.get(&h.key) {
            h.relays = set.iter().copied().collect();
        }
    }
    hits
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

    // bole-jom
    #[tokio::test]
    async fn rank_connected_before_unconnected() {
        use crate::collab::{CollabSigner, TrustKind};
        let me = CollabSigner::from_seed([20u8; 32]);
        let x = CollabSigner::from_seed([21u8; 32]);
        let connected = CollabSigner::from_seed([22u8; 32]);
        let lonely = CollabSigner::from_seed([23u8; 32]);
        // me -follow-> x -follow-> connected. `lonely` is in the corpus but unreachable.
        let own_edges = vec![me.sign_edge(x.public_key(), TrustKind::Follow, None, 1)];
        let corpus = vec![
            CollabObject::TrustEdge(x.sign_edge(connected.public_key(), TrustKind::Follow, None, 1)),
            CollabObject::Profile(connected.sign_profile("targetname".into(), String::new(), vec![], vec![], 1)),
            CollabObject::Profile(lonely.sign_profile("targetname".into(), String::new(), vec![], vec![], 1)),
        ];
        let hits = rank_strangers(&me.public_key(), &own_edges, &corpus, "targetname", 4);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].key, connected.public_key(), "connected stranger ranks first");
        assert_eq!(hits[0].hops, Some(2));
        assert!(hits[0].trust_path.is_some());
        assert_eq!(hits[1].key, lonely.public_key());
        assert_eq!(hits[1].trust_path, None, "unconnected stranger has no path, ranked last");
    }

    // bole-jom
    #[tokio::test]
    async fn rank_shorter_then_vouch_preference() {
        use crate::collab::{CollabSigner, TrustKind};
        let me = CollabSigner::from_seed([24u8; 32]);
        // Two 1-hop strangers: one reached by Vouch, one by Follow. Vouch ranks first.
        let vouched = CollabSigner::from_seed([25u8; 32]);
        let followed = CollabSigner::from_seed([26u8; 32]);
        let own_edges = vec![
            me.sign_edge(vouched.public_key(), TrustKind::Vouch, Some("v".into()), 1),
            me.sign_edge(followed.public_key(), TrustKind::Follow, None, 1),
        ];
        let corpus = vec![
            CollabObject::Profile(vouched.sign_profile("cand".into(), String::new(), vec![], vec![], 1)),
            CollabObject::Profile(followed.sign_profile("cand".into(), String::new(), vec![], vec![], 1)),
        ];
        let hits = rank_strangers(&me.public_key(), &own_edges, &corpus, "cand", 4);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].key, vouched.public_key(), "equal-length vouch path ranks above follow");
    }

    // bole-gp0
    #[tokio::test]
    async fn rank_matches_raw_key_hex_not_fingerprint() {
        use crate::collab::{key_hex, CollabSigner};
        let me = CollabSigner::from_seed([30u8; 32]);
        let stranger = CollabSigner::from_seed([31u8; 32]);
        // The stranger's display_name does NOT contain the term; only its raw key
        // hex does. Searching the full 64-char raw hex must find it — that hex can
        // never be a substring of the equal-length, different-valued fingerprint,
        // so a hit proves we match raw key hex (WS8d parity), not the fingerprint.
        let raw = key_hex(&stranger.public_key());
        assert!(
            !crate::collab::fingerprint(&stranger.public_key()).contains(&raw),
            "raw key hex must not appear in the fingerprint (precondition)"
        );
        let corpus = vec![CollabObject::Profile(stranger.sign_profile(
            "zzz".into(),
            String::new(),
            vec![],
            vec![],
            1,
        ))];
        let hits = rank_strangers(&me.public_key(), &[], &corpus, &raw, 4);
        assert_eq!(hits.len(), 1, "stranger found by its raw key hex");
        assert_eq!(hits[0].key, stranger.public_key());
        // The same corpus is NOT found when searching by an unrelated term.
        let none = rank_strangers(&me.public_key(), &[], &corpus, "no-such-term", 4);
        assert!(none.is_empty(), "unrelated term matches nothing");
    }

    // bole-yc9x
    #[tokio::test]
    async fn multi_merges_dedups_and_attributes() {
        use crate::collab::{CollabSigner, TrustKind};
        let me = CollabSigner::from_seed([50u8; 32]);
        let x = CollabSigner::from_seed([51u8; 32]);
        let stranger = CollabSigner::from_seed([52u8; 32]);
        let ra = [0xAAu8; 32]; // relay A key
        let rb = [0xBBu8; 32]; // relay B key

        let own_edges = vec![me.sign_edge(x.public_key(), TrustKind::Follow, None, 1)];
        // Relay A supplies the x->stranger edge; relay B supplies the stranger's profile (also A).
        let edge_xs =
            CollabObject::TrustEdge(x.sign_edge(stranger.public_key(), TrustKind::Follow, None, 1));
        let prof = CollabObject::Profile(
            stranger.sign_profile("cand".into(), String::new(), vec![], vec![], 1),
        );
        let per_relay = vec![
            (ra, vec![edge_xs.clone(), prof.clone()]),
            (rb, vec![prof.clone()]),
        ];

        let hits = rank_strangers_multi(&me.public_key(), &own_edges, &per_relay, "cand", 4);
        assert_eq!(hits.len(), 1, "stranger deduped to one hit");
        assert_eq!(hits[0].key, stranger.public_key());
        assert_eq!(hits[0].hops, Some(2), "trust path me->x->stranger");
        let mut relays = hits[0].relays.clone();
        relays.sort();
        assert_eq!(relays, vec![ra, rb], "attributed to both relays that served the profile");
    }
}
