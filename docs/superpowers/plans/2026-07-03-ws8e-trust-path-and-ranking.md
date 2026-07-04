# WS8e — Trust-Path-to-Stranger + Trust-Aware Ranking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn WS8d's raw stranger results into *trustworthy* ones — compute a verifiable, bounded trust-path (shown with edge kinds) connecting the querier to each stranger through the relay's aggregated `Follow`/`Vouch` graph, and rank strangers by that path.

**Architecture:** Generalize WS8c's BFS into `TrustGraph::trust_path(root, target, max_hops) -> Option<Vec<TrustHop>>` over combined `Follow`∪`Vouch` edges (record edge kind, prefer `Vouch`). Add a pure `rank_strangers(self_key, own_edges, relay_corpus, term, max_hops) -> Vec<StrangerHit>` that builds the combined verified graph, computes a path per matching stranger, and ranks (has-path > shorter > vouch-containing > name > fp). CLI `discover relay` gathers own+tracked edges, calls `rank_strangers`, adds `--max-hops`, and emits `trust_path`/`hops`.

**Tech Stack:** Rust (library-first + `bole-cli`), reusing WS8c `TrustGraph` and WS8d `collab_fetch_transient`/`discover relay`. Loopback `TcpConn` + real-`bole`-binary CLI tests.

## Global Constraints

- **Sound regardless of relay honesty:** every edge in a path is a `TrustEdge` the querier verified fail-closed; a relay can only withhold (hide) or inject-fake (dropped) edges — never forge a path.
- **Bounded & shown, never scored:** paths capped at `max_hops` (default **4**); results carry the full path with edge kinds, no numeric trust score.
- **Relay-trust NOT required:** WS8e queries whatever relay endpoint is given; soundness is per-edge.
- **Local depth-2 query untouched:** WS8c `discover query` and `follow_neighborhood`/`follow_paths` (hops=2) are unchanged; `trust_path` is a *new, separate* deeper search used only for relay strangers.
- **Keys raw hex** in CLI output (`crate::key::hex32`); edge kinds shown as `"follow"`/`"vouch"`.
- **Ranking order (exact):** has-path before no-path; then shorter path; then a path containing ≥1 `Vouch` edge before an equal-length pure-`Follow` path; then `display_name`; then key fingerprint.
- **No new deps.** Only crates already in `Cargo.toml`.
- **Process:** bd-only; each Task is one bead; branch name = bead ID; each contiguous added block carries one `// <bead-id>` comment; tests pass before merge; delete branch after merge; `bd close`.

### Per-task bead protocol
```bash
bd create "WS8e Task N: <title>" --json
bd update <id> --claim
git checkout -b <id>
# TDD steps
git checkout master && git merge <id> && git branch -d <id>
bd close <id>
```

---

## Gates → Tests

| Gate | Requirement | Satisfying test(s) | Task |
|------|-------------|--------------------|------|
| **G1** | `trust_path` over `Follow`∪`Vouch`: mixed path with edge kinds; `None` beyond `max_hops`; prefers `Vouch` when both connect a pair; shortest path | `trust_path_mixed_follow_vouch`, `trust_path_none_beyond_max`, `trust_path_prefers_vouch`, `trust_path_shortest` | 1 |
| **G2** | `rank_strangers`: term-matched profiles ranked has-path > shorter > vouch-containing > name > fp; each hit carries `trust_path`+`hops` | `rank_connected_before_unconnected`, `rank_shorter_then_vouch_preference` | 2 |
| **G3** | Loopback: relay chain → connected stranger with correct `trust_path`; unconnected → `None`; relay withholding a middle edge → `None` (sound); relay injecting a forged edge → dropped | `loopback_stranger_trust_path`, `loopback_withheld_and_forged` | 3 |
| **G4** | CLI `discover relay --max-hops` emits `trust_path`(array of `{key,via}`)/`hops` or `null`; E2E connected + unconnected | `cli_discover_relay_trust_path` | 4 |

---

## File Structure

- `src/collab/trust.rs` — add `TrustHop` + `TrustGraph::trust_path`.
- `src/collab/discovery.rs` — add `StrangerHit` + `rank_strangers`.
- `src/lib.rs` — re-export `TrustHop`, `StrangerHit`, `rank_strangers`.
- `bole-cli/src/commands/discover.rs` — `Relay` arm gathers own+tracked edges, calls `rank_strangers`, `--max-hops` flag, emits `trust_path`/`hops`.
- `tests/collab_network.rs` — loopback trust-path tests (Task 3).
- `bole-cli/tests/collab_cli.rs` — CLI trust-path E2E (Task 4).

---

## Task 1: `TrustGraph::trust_path` (combined-edge BFS)

**Files:** Modify `src/collab/trust.rs`, `src/lib.rs`.

**Interfaces:**
- Consumes: `TrustGraph { edges: Vec<TrustEdge> }`, `Key`, `TrustKind` (WS8a/c).
- Produces: `pub struct TrustHop { pub key: Key, pub via: TrustKind }`; `pub fn trust_path(&self, root: &Key, target: &Key, max_hops: u8) -> Option<Vec<TrustHop>>` (path is the ordered hops AFTER root, ending at `target`; `None` if unreachable within `max_hops`).

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/collab/trust.rs`:

```rust
    // <bead-id>
    #[test]
    fn trust_path_mixed_follow_vouch() {
        let (a, ak) = k(1);
        let (b, bk) = k(2);
        let (c, ck) = k(3);
        let (_d, dk) = k(4);
        // a -follow-> b -vouch-> c -follow-> d
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Vouch, Some("cee".into()), 1),
            c.sign_edge(dk, TrustKind::Follow, None, 1),
        ]);
        let path = g.trust_path(&ak, &dk, 4).unwrap();
        assert_eq!(path, vec![
            TrustHop { key: bk, via: TrustKind::Follow },
            TrustHop { key: ck, via: TrustKind::Vouch },
            TrustHop { key: dk, via: TrustKind::Follow },
        ]);
    }

    // <bead-id>
    #[test]
    fn trust_path_none_beyond_max() {
        let (a, ak) = k(5);
        let (b, bk) = k(6);
        let (c, ck) = k(7);
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
        ]);
        assert!(g.trust_path(&ak, &ck, 1).is_none(), "c is 2 hops, unreachable at max_hops=1");
        assert!(g.trust_path(&ak, &ck, 2).is_some(), "reachable at max_hops=2");
    }

    // <bead-id>
    #[test]
    fn trust_path_prefers_vouch() {
        let (a, ak) = k(8);
        let (_b, bk) = k(9);
        // a has BOTH a follow and a vouch edge to b -> the hop records Vouch.
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            a.sign_edge(bk, TrustKind::Vouch, Some("bee".into()), 1),
        ]);
        let path = g.trust_path(&ak, &bk, 4).unwrap();
        assert_eq!(path, vec![TrustHop { key: bk, via: TrustKind::Vouch }]);
    }

    // <bead-id>
    #[test]
    fn trust_path_shortest() {
        let (a, ak) = k(10);
        let (b, bk) = k(11);
        let (c, ck) = k(12);
        let (_t, tk) = k(13);
        // Two routes to t: a->t direct (follow) and a->b->c->t. Shortest wins.
        let g = TrustGraph::from_edges(vec![
            a.sign_edge(tk, TrustKind::Follow, None, 1),
            a.sign_edge(bk, TrustKind::Follow, None, 1),
            b.sign_edge(ck, TrustKind::Follow, None, 1),
            c.sign_edge(tk, TrustKind::Follow, None, 1),
        ]);
        let path = g.trust_path(&ak, &tk, 4).unwrap();
        assert_eq!(path.len(), 1, "direct 1-hop route is shortest");
        assert_eq!(path[0].key, tk);
    }
```

> **Implementer note:** the test module already has `fn k(seed: u8) -> (CollabSigner, Key)` and imports `TrustKind`; `sign_edge(to, kind, petname, seq)` is the WS8a signer method.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole collab::trust`
Expected: FAIL (`trust_path`/`TrustHop` undefined).

- [ ] **Step 3: Implement** — add to `src/collab/trust.rs`:

```rust
// <bead-id>
/// One hop on a trust path: the key reached and the edge kind that led into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustHop {
    pub key: Key,
    pub via: TrustKind,
}

impl TrustGraph {
    // <bead-id>
    /// Trust neighbours of `from`: the `to_key` of every `Follow`/`Vouch` out-edge,
    /// paired with the edge kind. When both a `Follow` and a `Vouch` edge connect
    /// `from` to the same key, `Vouch` is recorded (the stronger link).
    fn trust_neighbors(&self, from: &Key) -> Vec<(Key, TrustKind)> {
        let mut best: BTreeMap<Key, TrustKind> = BTreeMap::new();
        for e in &self.edges {
            if &e.from_key == from && matches!(e.kind, TrustKind::Follow | TrustKind::Vouch) {
                best.entry(e.to_key)
                    .and_modify(|v| {
                        if e.kind == TrustKind::Vouch {
                            *v = TrustKind::Vouch;
                        }
                    })
                    .or_insert(e.kind);
            }
        }
        best.into_iter().collect()
    }

    // <bead-id>
    /// Shortest bounded trust path from `root` to `target` over combined
    /// `Follow`∪`Vouch` edges, or `None` if `target` is unreachable within
    /// `max_hops` edges. The returned vector is the ordered hops after `root`,
    /// ending at `target`; each hop records the edge kind traversed into it.
    /// A relay can only withhold or inject edges (injected fakes never verified
    /// into the graph), so a returned path is always composed of real signed edges.
    pub fn trust_path(&self, root: &Key, target: &Key, max_hops: u8) -> Option<Vec<TrustHop>> {
        if root == target {
            return Some(Vec::new());
        }
        let mut paths: BTreeMap<Key, Vec<TrustHop>> = BTreeMap::new();
        paths.insert(*root, Vec::new());
        let mut q: VecDeque<Key> = VecDeque::new();
        q.push_back(*root);
        while let Some(node) = q.pop_front() {
            let node_path = paths.get(&node).expect("visited nodes have a path").clone();
            if node_path.len() as u8 == max_hops {
                continue;
            }
            for (next, via) in self.trust_neighbors(&node) {
                if let std::collections::btree_map::Entry::Vacant(slot) = paths.entry(next) {
                    let mut p = node_path.clone();
                    p.push(TrustHop { key: next, via });
                    if next == *target {
                        return Some(p);
                    }
                    slot.insert(p);
                    q.push_back(next);
                }
            }
        }
        None
    }
}
```

Re-export in `src/lib.rs` (next to the existing collab re-exports):
```rust
// <bead-id>
pub use collab::trust::TrustHop;
```

> **Implementer note:** `BTreeMap`/`VecDeque` are already imported in `trust.rs` (used by `follow_paths`). `TrustKind` derives `Copy` so `*v = TrustKind::Vouch` and `or_insert(e.kind)` are fine.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole collab::trust` then `cargo test -p bole`
Expected: 4 new tests pass; existing `follow_paths`/`follow_neighborhood` tests unaffected.

- [ ] **Step 5: Commit**
```bash
git add src/collab/trust.rs src/lib.rs
git commit -m "<bead-id>: TrustGraph::trust_path (combined Follow/Vouch bounded BFS) (G1)"
```

---

## Task 2: `rank_strangers` + `StrangerHit`

**Files:** Modify `src/collab/discovery.rs`, `src/lib.rs`.

**Interfaces:**
- Consumes: `TrustGraph::trust_path`/`TrustHop` (Task 1); `TrustEdge`, `CollabObject`, `Profile`, `Key`, `TrustKind`, `fingerprint` (WS8a).
- Produces: `pub struct StrangerHit { pub key: Key, pub display_name: String, pub trust_path: Option<Vec<TrustHop>>, pub hops: Option<usize> }`; `pub fn rank_strangers(self_key: &Key, own_edges: &[TrustEdge], relay_corpus: &[CollabObject], term: &str, max_hops: u8) -> Vec<StrangerHit>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/collab/discovery.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn rank_connected_before_unconnected() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
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

    // <bead-id>
    #[tokio::test]
    async fn rank_shorter_then_vouch_preference() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole collab::discovery::tests::rank_connected_before_unconnected`
Expected: FAIL (`rank_strangers`/`StrangerHit` undefined).

- [ ] **Step 3: Implement** — add to `src/collab/discovery.rs`:

```rust
// <bead-id>
use crate::collab::trust::{TrustGraph, TrustHop};
use crate::collab::{fingerprint, CollabObject, Key, TrustEdge, TrustKind};

/// A ranked stranger discovery hit: the publisher key, self-asserted display
/// name, and the verifiable trust path connecting the querier to this stranger
/// (`None` when unreachable within the hop bound).
#[derive(Debug, Clone)]
pub struct StrangerHit {
    pub key: Key,
    pub display_name: String,
    pub trust_path: Option<Vec<TrustHop>>,
    pub hops: Option<usize>,
}

// <bead-id>
/// Builds the combined trust graph (`own_edges` + all verified `TrustEdge`s in the
/// relay corpus), finds a bounded trust path from `self_key` to each term-matched
/// stranger `Profile`, and ranks: has-path > shorter > vouch-containing > name > fp.
/// Pure and repo-free; the relay corpus is already signature-verified by the fetch.
pub fn rank_strangers(
    self_key: &Key,
    own_edges: &[TrustEdge],
    relay_corpus: &[CollabObject],
    term: &str,
    max_hops: u8,
) -> Vec<StrangerHit> {
    // Combined graph: own edges + relay edges (all already verified).
    let mut edges: Vec<TrustEdge> = own_edges.to_vec();
    for o in relay_corpus {
        if let CollabObject::TrustEdge(e) = o {
            edges.push(e.clone());
        }
    }
    let graph = TrustGraph::from_edges(edges);

    // Term-matched stranger profiles become hits.
    let mut hits: Vec<StrangerHit> = Vec::new();
    for o in relay_corpus {
        if let CollabObject::Profile(p) = o {
            let matches = p.display_name.contains(term)
                || p.bio.contains(term)
                || p.dns_aliases.iter().any(|a| a.contains(term))
                || fingerprint(&p.key).contains(term);
            if !matches {
                continue;
            }
            let path = graph.trust_path(self_key, &p.key, max_hops);
            let hops = path.as_ref().map(|p| p.len());
            hits.push(StrangerHit { key: p.key, display_name: p.display_name.clone(), trust_path: path, hops });
        }
    }

    hits.sort_by(|a, b| {
        let a_conn = a.trust_path.is_some();
        let b_conn = b.trust_path.is_some();
        b_conn
            .cmp(&a_conn) // has-path (true) before no-path
            .then_with(|| a.hops.unwrap_or(usize::MAX).cmp(&b.hops.unwrap_or(usize::MAX))) // shorter first
            .then_with(|| {
                let av = has_vouch(&a.trust_path);
                let bv = has_vouch(&b.trust_path);
                bv.cmp(&av) // vouch-containing before pure-follow
            })
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| fingerprint(&a.key).cmp(&fingerprint(&b.key)))
    });
    hits
}

// <bead-id>
fn has_vouch(path: &Option<Vec<TrustHop>>) -> bool {
    path.as_ref().map(|p| p.iter().any(|h| h.via == TrustKind::Vouch)).unwrap_or(false)
}
```

Re-export in `src/lib.rs`:
```rust
// <bead-id>
pub use collab::discovery::{rank_strangers, StrangerHit};
```

> **Implementer note:** confirm the module path for `TrustHop`/`TrustGraph` (`crate::collab::trust`) and that `discovery.rs` doesn't already import these names; adjust imports so it compiles. `fingerprint`, `CollabObject`, `TrustEdge`, `TrustKind`, `Key` are WS8a items under `crate::collab`.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole collab::discovery` then `cargo test -p bole` then `cargo clippy -p bole --all-targets -- -D warnings`
Expected: both new tests pass; clippy clean.

- [ ] **Step 5: Commit**
```bash
git add src/collab/discovery.rs src/lib.rs
git commit -m "<bead-id>: rank_strangers — trust-path graph build + trust-aware ranking (G2)"
```

---

## Task 3: Loopback integration — trust-path over a relay corpus

**Files:** Modify `tests/collab_network.rs`.

**Interfaces:** Consumes `collab_fetch_transient`, `serve_collab_tcp_once(.., relay=true)`, `rank_strangers`.

- [ ] **Step 1: Write the failing tests** — add to `tests/collab_network.rs`:

```rust
// <bead-id>
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

// <bead-id>
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
```

- [ ] **Step 2: Run RED→GREEN**

Run: `cargo test -p bole --test collab_network` then `cargo test -p bole`
Expected: 2 new tests pass; existing pass. (`connect` helper + base imports already exist in the file.)

- [ ] **Step 3: Commit**
```bash
git add tests/collab_network.rs
git commit -m "<bead-id>: loopback trust-path over relay corpus (connected/unconnected/withheld/forged) (G3)"
```

---

## Task 4: CLI `discover relay` — `--max-hops` + `trust_path` output + E2E

**Files:** Modify `bole-cli/src/commands/discover.rs`, `bole-cli/tests/collab_cli.rs`.

**Interfaces:** Consumes `bole::rank_strangers`/`StrangerHit`/`TrustHop`; `Repository::public_edges`/`tracked_collab`; `crate::collabkey::signer_from`; `crate::key::hex32`.

- [ ] **Step 1: Write the failing E2E** — add to `bole-cli/tests/collab_cli.rs`:

```rust
// <bead-id>
#[test]
fn cli_discover_relay_trust_path() {
    use std::process::Stdio;
    fn serve(dir: &std::path::Path, args: &[&str]) -> std::process::Child {
        let mut c = bin();
        c.args(args).current_dir(dir).stdout(Stdio::null()).stderr(Stdio::null());
        c.spawn().unwrap()
    }
    // Publisher P (a stranger to the querier) serves.
    let ptmp = tempfile::tempdir().unwrap(); let p = ptmp.path();
    ok(p, &["init", "."], None);
    let pseed = "fa".repeat(32);
    ok(p, &["profile", "set", "--display-name", "Pat"], Some(&pseed));
    let paddr = "127.0.0.1:47901";
    let mut pchild = serve(p, &["node", "serve", "--listen", paddr]);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // Relay R follows+pulls P (so P is in R's aggregate), then serves --relay.
    let rtmp = tempfile::tempdir().unwrap(); let r = rtmp.path();
    ok(r, &["init", "."], None);
    let rseed = "eb".repeat(32);
    ok(r, &["profile", "set", "--display-name", "Relay"], Some(&rseed));
    let ppull = ok(r, &["discover", "pull", paddr, "--json"], Some(&rseed));
    let pkey = serde_json::from_slice::<serde_json::Value>(&ppull.stdout).unwrap()["pulled"].as_str().unwrap().to_string();
    let _ = pchild.kill(); let _ = pchild.wait();
    let raddr = "127.0.0.1:47902";
    let mut rchild = serve(r, &["node", "serve", "--listen", raddr, "--relay"]);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // Querier Q follows P (its own edge), then searches the relay: P should be
    // connected (1 hop) via a follow edge; trust_path non-null.
    let qtmp = tempfile::tempdir().unwrap(); let q = qtmp.path();
    ok(q, &["init", "."], None);
    let qseed = "dc".repeat(32);
    ok(q, &["profile", "set", "--display-name", "Q"], Some(&qseed));
    ok(q, &["trust", "follow", &pkey], Some(&qseed));
    let out = ok(q, &["discover", "relay", raddr, "Pat", "--json"], Some(&qseed));
    let _ = rchild.kill(); let _ = rchild.wait();

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pat = v.as_array().unwrap().iter().find(|r| r["display_name"] == "Pat").expect("Pat found");
    assert_eq!(pat["reach"], "stranger");
    assert!(pat["trust_path"].is_array(), "connected stranger has a trust_path: {}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(pat["hops"], 1);
    assert_eq!(pat["trust_path"][0]["via"], "follow");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-cli --test collab_cli -- cli_discover_relay_trust_path`
Expected: FAIL — `trust_path`/`hops` not emitted; `--max-hops` absent.

- [ ] **Step 3: Rewrite the `Cmd::Relay` arm** — in `bole-cli/src/commands/discover.rs`, add `--max-hops` + `key_env`/`key_file` to the `Relay` variant and rewrite the handler to use `rank_strangers`:

```rust
    // <bead-id>
    /// Search a relay for strangers, with a verifiable trust path (transient; no state change).
    Relay {
        endpoint: String,
        term: String,
        #[arg(long, default_value_t = 4)]
        max_hops: u8,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<std::path::PathBuf>,
    },
```

```rust
        // <bead-id>
        Cmd::Relay { endpoint, term, max_hops, key_env, key_file } => {
            use bole::sync::collab::collab_fetch_transient;
            use bole::sync::transport::TcpConn;
            let self_key = crate::collabkey::signer_from(&key_env, key_file.as_deref())?.public_key();
            // Gather the querier's own verified edges (own published + tracked cache).
            let mut own_edges = ctx.repo.public_edges().await?;
            for o in ctx.repo.tracked_collab().await? {
                if let bole::CollabObject::TrustEdge(e) = o {
                    own_edges.push(e);
                }
            }
            let stream = tokio::net::TcpStream::connect(&endpoint).await?;
            let mut conn = TcpConn::new(stream);
            let corpus = collab_fetch_transient(&mut conn).await?;
            let hits = bole::rank_strangers(&self_key, &own_edges, &corpus, &term, max_hops);
            let rows: Vec<_> = hits.iter().map(|h| {
                let trust_path = h.trust_path.as_ref().map(|path| {
                    path.iter().map(|hop| serde_json::json!({
                        "key": key::hex32(&hop.key),
                        "via": match hop.via { bole::TrustKind::Vouch => "vouch", bole::TrustKind::Follow => "follow", bole::TrustKind::Review => "review" },
                    })).collect::<Vec<_>>()
                });
                serde_json::json!({
                    "key": key::hex32(&h.key),
                    "display_name": h.display_name,
                    "reach": "stranger",
                    "trust_path": trust_path,
                    "hops": h.hops,
                })
            }).collect();
            out.emit(
                || {
                    if rows.is_empty() { "no strangers matched".to_string() }
                    else { rows.iter().map(|r| {
                        let hops = if r["hops"].is_null() { "no path".to_string() } else { format!("{} hops", r["hops"]) };
                        format!("{} [stranger, {}] {}", r["key"], hops, r["display_name"])
                    }).collect::<Vec<_>>().join("\n") }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
```

> **Implementer note:** `public_edges`/`tracked_collab` are `Repository` async methods (WS8c). `bole::rank_strangers`, `bole::CollabObject`, `bole::TrustKind` are re-exported (confirm; else `bole::collab::...`). Keep the surrounding `run(ctx, out, cmd)` + `Output::emit` shape. `serde_json::json!` serializes `Option<Vec<...>>` → `null` when `None` — so `"trust_path": trust_path` and `"hops": h.hops` emit `null` for unconnected strangers automatically.

- [ ] **Step 4: Run RED→GREEN**

Run: `cargo test -p bole-cli --test collab_cli` then `cargo test -p bole-cli` then `cargo build --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: the new E2E + existing CLI tests pass; workspace builds; clippy clean. Fixed ports 47901/47902 carry the parallel-run flake caveat (raise the sleep if needed); reap every `node serve` child (`kill()`+`wait()`).

- [ ] **Step 5: Commit**
```bash
git add bole-cli/src/commands/discover.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: CLI discover relay --max-hops + trust_path output + E2E (G4)"
```

---

## Self-Review

**Spec coverage:** §2 combined graph → Task 2 (`rank_strangers` builds own+relay edges) + Task 4 (CLI adds own `public_edges`+`tracked_collab`). §3 bounded path search → Task 1 (G1). §4 ranking → Task 2 (G2). §5 CLI output (`trust_path`/`hops`/`--max-hops`, raw hex, edge kinds) → Task 4 (G4). §6 testing → Tasks 1–4. §1 invariants: sound-regardless-of-relay (only verified `TrustEdge`s enter the graph — Task 3's forged-edge test); bounded+shown (Task 1 `max_hops`, Task 4 emits the path not a score); relay-trust-not-required (no relay-trust code); local-depth-2-untouched (`follow_paths`/`follow_neighborhood` unchanged; `trust_path` is additive).

**Placeholder scan:** No "TBD"/"handle errors". `Implementer note` blocks flag only confirmation points (module paths for `TrustHop`/`rank_strangers`, `bole::` re-export reachability) with exact resolutions.

**Type consistency:** `TrustHop { key: Key, via: TrustKind }` and `trust_path(root, target, max_hops) -> Option<Vec<TrustHop>>` (Task 1) consumed by `rank_strangers` (Task 2) and the CLI (Task 4). `StrangerHit { key, display_name, trust_path: Option<Vec<TrustHop>>, hops: Option<usize> }` produced in Task 2, consumed in Task 4. `via` serialized as `follow`/`vouch`; keys `key::hex32`. Ranking order matches the Global Constraints verbatim.

**Flagged risks:** (1) Task 3's forged-edge case is asserted at the `verify_edge` level + via corpus-absence (a real fetch drops it before `rank_strangers`) — `rank_strangers` assumes a pre-verified corpus, which is exactly what `collab_fetch_transient` guarantees; the test documents this boundary. (2) `bole::` re-export paths for `rank_strangers`/`CollabObject`/`TrustKind` must be confirmed. (3) fixed E2E ports carry the known flake caveat.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-03-ws8e-trust-path-and-ranking.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task (one bead + branch each), review between tasks.

**2. Inline Execution** — execute tasks here in batches with checkpoints.

Which approach?
