// bole-n9fx
//! Server-side relay search: given a served corpus of verified collaboration
//! objects, select the profiles matching a term and the DIRECTED
//! reverse-reachability edge ball around them — the exact edge set a client's
//! forward (`from_key -> to_key`), `<= max_hops` trust-path BFS into a match can
//! traverse. Filtering only; trust stays client-side (the client re-verifies).
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::collab::{key_hex, CollabObject, Key, Profile, TrustEdge};
use crate::object::{Object, ObjectId};

// bole-n9fx
/// True iff `p` matches `term` on the SAME fields as `rank_strangers`:
/// display_name, bio, any dns_alias, or the raw key hex.
fn profile_matches(p: &Profile, term: &str) -> bool {
    p.display_name.contains(term)
        || p.bio.contains(term)
        || p.dns_aliases.iter().any(|a| a.contains(term))
        || key_hex(&p.key).contains(term)
}

// bole-n9fx
/// The content id of an edge, matching how `ObjectStore::put` addresses objects.
fn edge_id(e: &TrustEdge) -> ObjectId {
    let bytes = crate::codec::serialize(&Object::Collab(CollabObject::TrustEdge(e.clone())))
        .expect("postcard serialization is infallible for owned data");
    crate::codec::object_id(&bytes)
}

// bole-n9fx
/// Matching profiles + the deduped union of directed reverse-reachability edge
/// balls around each match. `max_hops` bounds the reverse BFS depth (in edges).
pub fn search_ball(corpus: &[CollabObject], term: &str, max_hops: u8) -> Vec<CollabObject> {
    // Partition the corpus.
    let mut profiles: Vec<&Profile> = Vec::new();
    let mut edges: Vec<&TrustEdge> = Vec::new();
    for o in corpus {
        match o {
            CollabObject::Profile(p) => profiles.push(p),
            CollabObject::TrustEdge(e) => edges.push(e),
        }
    }

    // Reverse adjacency: for a node `n`, the edges whose `to_key == n` (i.e. edges
    // pointing INTO `n`), so BFS backward walks predecessors.
    let mut incoming: BTreeMap<Key, Vec<&TrustEdge>> = BTreeMap::new();
    for e in &edges {
        incoming.entry(e.to_key).or_default().push(e);
    }

    let mut out: Vec<CollabObject> = Vec::new();
    let mut ball: BTreeSet<ObjectId> = BTreeSet::new();

    for p in &profiles {
        if !profile_matches(p, term) {
            continue;
        }
        out.push(CollabObject::Profile((*p).clone()));
        // Reverse BFS from the matched stranger's key, bounded by max_hops edges.
        // Frontier holds (node, depth-of-node-from-match).
        let mut visited: BTreeSet<Key> = BTreeSet::new();
        let mut q: VecDeque<(Key, u8)> = VecDeque::new();
        q.push_back((p.key, 0));
        visited.insert(p.key);
        while let Some((node, depth)) = q.pop_front() {
            if depth >= max_hops {
                continue; // no edge at this depth would be within max_hops
            }
            if let Some(preds) = incoming.get(&node) {
                for e in preds {
                    // This edge (from -> node) is at reverse depth `depth + 1` <= max_hops.
                    let id = edge_id(e);
                    if ball.insert(id) {
                        out.push(CollabObject::TrustEdge((*e).clone()));
                    }
                    if visited.insert(e.from_key) {
                        q.push_back((e.from_key, depth + 1));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabSigner, CollabObject, TrustKind, key_hex};

    // Helper: sign a profile / edge into CollabObjects.
    fn prof(s: &CollabSigner, name: &str, bio: &str) -> CollabObject {
        CollabObject::Profile(s.sign_profile(name.into(), bio.into(), vec![], vec![], 1))
    }
    fn edge(from: &CollabSigner, to: &CollabSigner) -> CollabObject {
        CollabObject::TrustEdge(from.sign_edge(to.public_key(), TrustKind::Follow, None, 1))
    }

    #[test]
    fn matches_same_fields_as_rank_strangers_incl_key_hex() {
        let a = CollabSigner::from_seed([1u8; 32]);
        let corpus = vec![prof(&a, "zzz", "")]; // name/bio do NOT contain the term
        // Match on raw key hex substring only.
        let hex = key_hex(&a.public_key());
        let hits = search_ball(&corpus, &hex, 4);
        assert_eq!(hits.len(), 1, "profile found by its raw key hex");
        // A non-matching term selects nothing.
        assert!(search_ball(&corpus, "no-such-term", 4).is_empty());
    }

    #[test]
    fn reverse_ball_completeness_and_bound() {
        // Chain a -> b -> c -> target (Follow edges). Search for "target".
        let a = CollabSigner::from_seed([2u8; 32]);
        let b = CollabSigner::from_seed([3u8; 32]);
        let c = CollabSigner::from_seed([4u8; 32]);
        let t = CollabSigner::from_seed([5u8; 32]);
        let e_ab = edge(&a, &b);
        let e_bc = edge(&b, &c);
        let e_ct = edge(&c, &t);
        let corpus = vec![prof(&t, "target", ""), e_ab.clone(), e_bc.clone(), e_ct.clone()];

        // max_hops >= 3: all three edges are within the reverse ball of target.
        let hits = search_ball(&corpus, "target", 3);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert_eq!(edges.len(), 3, "all 3 chain edges returned at max_hops=3");

        // max_hops = 2: the far edge a->b (reverse depth 3) is dropped.
        let hits2 = search_ball(&corpus, "target", 2);
        let edges2: Vec<_> = hits2.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert_eq!(edges2.len(), 2, "only c->t and b->c within reverse depth 2");
    }

    #[test]
    fn directedness_edge_away_from_match_excluded() {
        // target -> x (edge points AWAY from target; not on any forward path INTO target).
        let t = CollabSigner::from_seed([6u8; 32]);
        let x = CollabSigner::from_seed([7u8; 32]);
        let e_tx = edge(&t, &x);
        let corpus = vec![prof(&t, "target", ""), e_tx];
        let hits = search_ball(&corpus, "target", 4);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        assert!(edges.is_empty(), "an edge leaving the match is not reverse-reachable");
    }

    #[test]
    fn multi_match_union_deduped() {
        // Two matches share a common predecessor edge s -> shared; shared -> t1, shared -> t2.
        let s = CollabSigner::from_seed([8u8; 32]);
        let shared = CollabSigner::from_seed([9u8; 32]);
        let t1 = CollabSigner::from_seed([10u8; 32]);
        let t2 = CollabSigner::from_seed([11u8; 32]);
        let e_s_shared = edge(&s, &shared);
        let e_shared_t1 = CollabObject::TrustEdge(shared.sign_edge(t1.public_key(), TrustKind::Follow, None, 1));
        let e_shared_t2 = CollabObject::TrustEdge(shared.sign_edge(t2.public_key(), TrustKind::Follow, None, 1));
        let corpus = vec![
            prof(&t1, "target-one", ""), prof(&t2, "target-two", ""),
            e_s_shared, e_shared_t1, e_shared_t2,
        ];
        // Term "target" matches both t1 and t2.
        let hits = search_ball(&corpus, "target", 4);
        let edges: Vec<_> = hits.iter().filter(|o| matches!(o, CollabObject::TrustEdge(_))).collect();
        // e_s_shared appears exactly once (shared reverse ball), plus the two shared->tN edges.
        assert_eq!(edges.len(), 3, "union of reverse balls, shared edge deduped once");
        let profs = hits.iter().filter(|o| matches!(o, CollabObject::Profile(_))).count();
        assert_eq!(profs, 2, "both matching profiles returned");
    }
}
