// bole-p6j
use std::collections::{BTreeMap, VecDeque};

use crate::collab::{Key, TrustEdge, TrustKind};

/// A read-only view over trust edges, indexed for depth-bounded traversal.
/// The graph *suggests*; it never confers authority (roots stay authoritative).
pub struct TrustGraph {
    edges: Vec<TrustEdge>,
}

/// A petname suggested for a key by the trust graph, with its trust route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VouchSuggestion {
    pub petname: String,
    /// 1 = direct vouch by root; 2 = friend-of-friend.
    pub depth: u8,
    /// The route `root -> ... -> voucher` whose last hop authored the vouch.
    pub path: Vec<Key>,
}

impl TrustGraph {
    pub fn from_edges(edges: Vec<TrustEdge>) -> Self {
        Self { edges }
    }

    fn follows(&self, from: &Key) -> Vec<Key> {
        self.edges
            .iter()
            .filter(|e| e.kind == TrustKind::Follow && &e.from_key == from)
            .map(|e| e.to_key)
            .collect()
    }

    /// BFS over `Follow` edges from `root`, bounded to `hops`. Returns each
    /// reachable key mapped to its minimum hop distance (root excluded).
    pub fn follow_neighborhood(&self, root: &Key, hops: u8) -> BTreeMap<Key, u8> {
        let mut dist: BTreeMap<Key, u8> = BTreeMap::new();
        let mut q: VecDeque<(Key, u8)> = VecDeque::new();
        q.push_back((*root, 0));
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(*root);
        while let Some((node, d)) = q.pop_front() {
            if d == hops {
                continue;
            }
            for next in self.follows(&node) {
                if seen.insert(next) {
                    dist.insert(next, d + 1);
                    q.push_back((next, d + 1));
                }
            }
        }
        dist
    }

    // bole-36y
    /// BFS over `Follow` edges from `root`, bounded to `hops`, returning each
    /// reachable key mapped to its minimal-hop path `[root, …, key]` (root itself
    /// excluded). Shortest-path by construction; WS8c ignores multi-path/weighted
    /// trust.
    pub fn follow_paths(&self, root: &Key, hops: u8) -> BTreeMap<Key, Vec<Key>> {
        let mut paths: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
        paths.insert(*root, vec![*root]);
        let mut q: VecDeque<Key> = VecDeque::new();
        q.push_back(*root);
        while let Some(node) = q.pop_front() {
            let node_path = paths.get(&node).expect("visited nodes have a path").clone();
            if (node_path.len() as u8 - 1) == hops {
                continue;
            }
            for next in self.follows(&node) {
                if let std::collections::btree_map::Entry::Vacant(e) = paths.entry(next) {
                    let mut p = node_path.clone();
                    p.push(next);
                    e.insert(p);
                    q.push_back(next);
                }
            }
        }
        paths.remove(root);
        paths
    }

    /// Vouch suggestions for `target` reachable from `root` within `max_depth`.
    /// Depth-1: a direct `Vouch` edge authored by `root`. Depth-2: a `Vouch`
    /// authored by a key `root` directly `Follow`s. Deeper is not returned.
    pub fn vouch_suggestions(&self, root: &Key, target: &Key, max_depth: u8) -> Vec<VouchSuggestion> {
        let mut out = Vec::new();
        // Depth 1: root vouches for target directly.
        if max_depth >= 1 {
            for e in &self.edges {
                if e.kind == TrustKind::Vouch && &e.from_key == root && &e.to_key == target {
                    if let Some(name) = &e.petname {
                        out.push(VouchSuggestion { petname: name.clone(), depth: 1, path: vec![*root] });
                    }
                }
            }
        }
        // Depth 2: a key root follows vouches for target.
        if max_depth >= 2 {
            let direct_follows: Vec<Key> = self.follows(root);
            for voucher in direct_follows {
                for e in &self.edges {
                    if e.kind == TrustKind::Vouch && e.from_key == voucher && &e.to_key == target {
                        if let Some(name) = &e.petname {
                            out.push(VouchSuggestion {
                                petname: name.clone(),
                                depth: 2,
                                path: vec![*root, voucher],
                            });
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::CollabSigner;

    fn k(seed: u8) -> (CollabSigner, Key) {
        let s = CollabSigner::from_seed([seed; 32]);
        let key = s.public_key();
        (s, key)
    }

    #[test]
    fn follow_neighborhood_respects_hops() {
        let (a, ak) = k(1);
        let (b, bk) = k(2);
        let (_c, ck) = k(3);
        // a -follow-> b -follow-> c
        let edges = vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
        ];
        let g = TrustGraph::from_edges(edges);

        let n1 = g.follow_neighborhood(&ak, 1);
        assert_eq!(n1.get(&bk), Some(&1));
        assert!(!n1.contains_key(&ck), "c is 2 hops away; excluded at hops=1");

        let n2 = g.follow_neighborhood(&ak, 2);
        assert_eq!(n2.get(&bk), Some(&1));
        assert_eq!(n2.get(&ck), Some(&2));
        assert!(!n2.contains_key(&ak), "root is never in its own neighborhood");
    }

    #[test]
    fn vouch_depth_one_and_two() {
        let (a, ak) = k(4);
        let (b, bk) = k(5);
        let (_c, ck) = k(6);
        // a -follow-> b (so b's vouch is reachable at depth 2 via follow path),
        // b -vouch("cee")-> c ; a -vouch("bee")-> b
        let edges = vec![
            a.sign_edge(bk, TrustKind::Vouch, Some("bee".into()), 1),
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Vouch, Some("cee".into()), 1),
        ];
        let g = TrustGraph::from_edges(edges);

        let direct = g.vouch_suggestions(&ak, &bk, 2);
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].petname, "bee");
        assert_eq!(direct[0].depth, 1);

        let indirect = g.vouch_suggestions(&ak, &ck, 2);
        assert_eq!(indirect.len(), 1);
        assert_eq!(indirect[0].petname, "cee");
        assert_eq!(indirect[0].depth, 2);
        assert_eq!(indirect[0].path, vec![ak, bk], "path shows the trust route root->voucher");
    }

    #[test]
    fn hop_limit_does_not_change_identity() {
        let (a, ak) = k(7);
        let (_b, bk) = k(8);
        let edges = vec![a.sign_edge(bk, TrustKind::Follow, None, 1)];
        let g = TrustGraph::from_edges(edges);
        // Whatever the hop limit, b's key (identity) is unchanged.
        assert!(g.follow_neighborhood(&ak, 0).is_empty());
        assert_eq!(g.follow_neighborhood(&ak, 1).keys().next(), Some(&bk));
        assert_eq!(g.follow_neighborhood(&ak, 5).get(&bk), Some(&1));
    }

    // bole-36y
    #[test]
    fn follow_paths_depth2() {
        let (a, ak) = k(1);
        let (b, bk) = k(2);
        let (_c, ck) = k(3);
        // a -follow-> b -follow-> c
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
        ]);
        let paths = g.follow_paths(&ak, 2);
        assert_eq!(paths.get(&bk), Some(&vec![ak, bk]), "direct path [a,b]");
        assert_eq!(paths.get(&ck), Some(&vec![ak, bk, ck]), "depth-2 path [a,b,c]");
        assert!(!paths.contains_key(&ak), "root excluded");
    }

    // bole-36y
    #[test]
    fn follow_paths_hop_bound() {
        let (a, ak) = k(4);
        let (b, bk) = k(5);
        let (_c, ck) = k(6);
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
        ]);
        let paths = g.follow_paths(&ak, 1);
        assert!(paths.contains_key(&bk), "b at depth 1 included");
        assert!(!paths.contains_key(&ck), "c at depth 2 excluded at hops=1");
    }
}
