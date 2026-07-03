// bole-t7c
use std::collections::BTreeMap;

use crate::collab::trust::TrustGraph;
use crate::collab::{fingerprint, Key};

// bole-t7c
/// How a key's display name was resolved. Keys are always canonical; a name is
/// only a label. `Vouch` carries its depth and trust path so a UI can show
/// "via X → Y" and mark depth-2 as a weak hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetnameResolution {
    /// This node's own name for the key (highest precedence).
    Local(String),
    /// A name suggested by the trust graph.
    Vouch { name: String, depth: u8, path: Vec<Key> },
    /// No name known; fall back to the key fingerprint.
    Fingerprint(String),
}

/// Resolves display names for keys under a fixed precedence:
/// local > depth-1 vouch > depth-2 vouch > fingerprint. Never merges two keys.
pub struct Namer<'a> {
    root: Key,
    local: &'a BTreeMap<Key, String>,
    graph: &'a TrustGraph,
}

impl<'a> Namer<'a> {
    pub fn new(root: Key, local: &'a BTreeMap<Key, String>, graph: &'a TrustGraph) -> Self {
        Self { root, local, graph }
    }

    pub fn resolve(&self, key: &Key) -> PetnameResolution {
        if let Some(name) = self.local.get(key) {
            return PetnameResolution::Local(name.clone());
        }
        let mut suggestions = self.graph.vouch_suggestions(&self.root, key, 2);
        // Prefer the shallowest suggestion (depth-1 before depth-2); stable within a depth tier.
        suggestions.sort_by_key(|s| s.depth);
        if let Some(s) = suggestions.into_iter().next() {
            return PetnameResolution::Vouch { name: s.petname, depth: s.depth, path: s.path };
        }
        PetnameResolution::Fingerprint(fingerprint(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::{CollabSigner, TrustKind};

    fn key(seed: u8) -> (CollabSigner, Key) {
        let s = CollabSigner::from_seed([seed; 32]);
        let k = s.public_key();
        (s, k)
    }

    #[test]
    fn petname_precedence_order() {
        let (root, rk) = key(1);
        let (_b, bk) = key(2);
        // root vouches bk="graph-bob"; local map says bk="my-bob".
        let g = TrustGraph::from_edges(vec![root.sign_edge(bk, TrustKind::Vouch, Some("graph-bob".into()), 1)]);

        let mut local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);
        // No local entry -> depth-1 vouch wins.
        match namer.resolve(&bk) {
            PetnameResolution::Vouch { name, depth, .. } => {
                assert_eq!(name, "graph-bob");
                assert_eq!(depth, 1);
            }
            other => panic!("expected Vouch, got {other:?}"),
        }

        // Local entry beats the graph.
        local.insert(bk, "my-bob".into());
        let namer = Namer::new(rk, &local, &g);
        assert!(matches!(namer.resolve(&bk), PetnameResolution::Local(n) if n == "my-bob"));

        // Unknown key -> fingerprint fallback.
        let (_u, uk) = key(9);
        assert!(matches!(namer.resolve(&uk), PetnameResolution::Fingerprint(fp) if fp == fingerprint(&uk)));
    }

    #[test]
    fn same_petname_two_keys_not_merged() {
        let (root, rk) = key(3);
        let (_x, xk) = key(4);
        let (_y, yk) = key(5);
        // root vouches BOTH xk and yk as "alice".
        let g = TrustGraph::from_edges(vec![
            root.sign_edge(xk, TrustKind::Vouch, Some("alice".into()), 1),
            root.sign_edge(yk, TrustKind::Vouch, Some("alice".into()), 1),
        ]);
        let local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);

        let rx = namer.resolve(&xk);
        let ry = namer.resolve(&yk);
        // Same display name, but the keys remain distinct identities: resolution
        // never collapses them, and callers disambiguate by fingerprint.
        assert_ne!(xk, yk);
        assert!(matches!(&rx, PetnameResolution::Vouch { name, .. } if name == "alice"));
        assert!(matches!(&ry, PetnameResolution::Vouch { name, .. } if name == "alice"));
        assert_ne!(fingerprint(&xk), fingerprint(&yk));
    }

    // bole-t7c
    #[test]
    fn depth_one_beats_depth_two() {
        let (root, rk) = key(20);
        let (mid, mk) = key(21);
        let (_t, tk) = key(22);
        // root vouches tk directly ("d1"); root follows mid; mid vouches tk ("d2").
        let g = TrustGraph::from_edges(vec![
            root.sign_edge(tk, TrustKind::Vouch, Some("d1".into()), 1),
            root.sign_edge(mk, TrustKind::Follow, None, 1),
            mid.sign_edge(tk, TrustKind::Vouch, Some("d2".into()), 1),
        ]);
        let local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);
        match namer.resolve(&tk) {
            PetnameResolution::Vouch { name, depth, .. } => {
                assert_eq!(depth, 1, "depth-1 must beat depth-2");
                assert_eq!(name, "d1");
            }
            other => panic!("expected Vouch, got {other:?}"),
        }
    }

    #[test]
    fn depth_two_only_resolution() {
        let (root, rk) = key(30);
        let (mid, mk) = key(31);
        let (_t, tk) = key(32);
        // root follows mid; mid vouches tk. No direct vouch from root -> depth-2 only.
        let g = TrustGraph::from_edges(vec![
            root.sign_edge(mk, TrustKind::Follow, None, 1),
            mid.sign_edge(tk, TrustKind::Vouch, Some("via-mid".into()), 1),
        ]);
        let local = BTreeMap::new();
        let namer = Namer::new(rk, &local, &g);
        match namer.resolve(&tk) {
            PetnameResolution::Vouch { name, depth, path } => {
                assert_eq!(depth, 2);
                assert_eq!(name, "via-mid");
                assert_eq!(path, vec![rk, mk], "depth-2 path is root -> voucher");
            }
            other => panic!("expected depth-2 Vouch, got {other:?}"),
        }
    }
}
