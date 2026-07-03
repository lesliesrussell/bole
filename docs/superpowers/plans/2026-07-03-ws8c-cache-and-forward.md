# WS8c — Cache-and-Forward + Depth-2 Reach Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the follow-graph traversable over the wire — a node re-serves the verified public objects it pulled from peers it directly follows, giving a follower depth-2 reach; plus real depth-2 `trust_path` and petname-aware `discover query`.

**Architecture:** `collab_adverts` advertises `public/**` + `remotes/<fp>/**` filtered to direct-follow authors (never scoped). `collab_pull` goes multi-author: verify every object, file by intrinsic author under `remotes/<author-fp>/`, still return the dialed server's own key. `TrustGraph` gains a predecessor-tracking BFS (`follow_paths`) so `local_discovery_index` emits real `[A,B,C]` paths; a new `Repository::query_discovery` resolves a `Namer` petname + `reach` + `trust_path` per hit, which the CLI `discover query` formats. Fail-closed at pull AND index; depth hard-capped at 2.

**Tech Stack:** Rust (library-first + `bole-cli`), reusing WS5 wire (`Conn`/`Message`/`build_pack`/`missing_closure`) and WS8a/WS8b collab (`Profile`/`TrustEdge`/`verify_*`/`Index`/`TrustGraph`/`Namer`, `refs/collab/{public,remotes,scoped}/`). Loopback `TcpConn` + real-`bole`-binary CLI tests.

## Global Constraints

- **Cached ≠ authored:** every object is verified against its *embedded* author key and filed by *true author* regardless of arrival namespace; a wrong-key signature fails to verify. Fail-closed at pull AND index.
- **Serve horizon (verbatim invariant):** a node may re-serve verified public state for authors it directly follows, and for no others. Never scoped; never non-followed cache.
- **Storage story:** `refs/collab/public/**` = objects this node authored; `refs/collab/remotes/<fp>/**` = this node's by-author cache of pulled state (a node's own objects never go in its own `remotes/`).
- **Depth hard-capped at 2** in both transport (serve horizon) and index (`follow_neighborhood`/`follow_paths` hops = 2). No deeper paths in WS8c.
- **`trust_path` is minimal-hop (BFS shortest)**; no multi-path/weighted trust in WS8c.
- **Petnames are graph-derived** (`Vouch` edges, fingerprint fallback); no new storage format, no new commands. `reach` (`self`/`direct`/`transitive`) is defined by graph distance (0/1/2), not transport provenance.
- **`--json` is the stable contract**; keys shown as raw 64-hex (`crate::key::hex32`), never fingerprint, in CLI output.
- **No new deps.** Only crates already in `Cargo.toml`.
- **Process:** bd-only; each Task is one bead; branch name = bead ID; each contiguous added block carries one `// <bead-id>` comment; tests pass before merge; delete branch after merge; `bd close`.

### Per-task bead protocol
```bash
bd create "WS8c Task N: <title>" --json     # note the id
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
| **G1** | `collab_adverts` advertises `public/**` + followed-author `remotes/`, excludes non-followed `remotes/` and `scoped/` | `adverts_include_followed_remote`, `adverts_exclude_unfollowed_remote`, `collab_adverts_exclude_scoped` (updated) | 1 |
| **G2** | Multi-author `collab_pull` files each verified object by intrinsic author (server-own + cached), returns server key, drops tampered cached object | `pull_files_cached_by_author`, `pull_drops_tampered_cached` | 2 |
| **G3** | `TrustGraph::follow_paths` yields `[root,B,C]` for depth-2, hop-bounded, root excluded | `follow_paths_depth2`, `follow_paths_hop_bound` | 3 |
| **G4** | `local_discovery_index` emits the real `[A,B,C]` `trust_path` for a depth-2 author | `index_emits_depth2_path` | 4 |
| **G5** | `query_discovery` resolves a `Vouch` petname (fingerprint fallback), `reach`, and real `trust_path` | `query_resolves_vouch_petname`, `query_reach_and_path` | 5 |
| **G6** | CLI `discover query --json` emits `key`/`display_name`/`petname`/`reach`/`trust_path` | `cli_query_shows_petname_and_reach` | 6 |
| **G7** | E2E: cache-forward A→B→C depth-2 with `[A,B,C]`; scoped never forwarded; non-followed never served; tamper fail-closed; over-depth D never surfaces; CLI 3-node | `loopback_cache_forward_depth2`, `loopback_scoped_and_unfollowed_never_forwarded`, `loopback_tampered_cached_dropped`, `loopback_over_depth_excluded`, `cli_three_node_transitive` | 7 |

---

## File Structure

- `src/sync/collab.rs` — `collab_adverts` → async + followed-`remotes` advertise; `collab_pull` → multi-author; update the WS8b test that calls `collab_adverts` sync.
- `src/collab/trust.rs` — add `follow_paths`.
- `src/repo/collab.rs` — `local_discovery_index` uses `follow_paths`; add `QueryHit` + `query_discovery`.
- `src/lib.rs` — re-export `QueryHit` if needed by the CLI.
- `bole-cli/src/commands/discover.rs` — `discover query` formats `query_discovery` output.
- `tests/collab_network.rs` — cache-forward loopback + negatives (Task 7).
- `bole-cli/tests/collab_cli.rs` — 3-node transitive E2E (Task 7).

---

## Task 1: Serve horizon — advertise followed-author `remotes/`

**Files:** Modify `src/sync/collab.rs`.

**Interfaces:**
- Consumes: `Repository::public_edges()`, `fingerprint`, `TrustKind`, `COLLAB_PUBLIC_PREFIX`, `COLLAB_REMOTES_PREFIX`.
- Produces: `pub async fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>>` (now **async**).

- [ ] **Step 1: Write the failing tests** — add to the `#[cfg(test)] mod tests` in `src/sync/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn adverts_include_followed_remote() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([1u8; 32]);
        let c = CollabSigner::from_seed([2u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // I follow C.
        repo.publish_edge(&me.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // I have C cached under remotes/<Cfp>/profile (as a pull would have stored).
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).await.unwrap();
        assert!(adverts.iter().any(|r| r.name.as_str() == format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")),
            "followed author's cached profile is advertised");
        assert!(adverts.iter().any(|r| r.name.as_str().contains("/public/profile/")), "own profile still advertised");
    }

    // <bead-id>
    #[tokio::test]
    async fn adverts_exclude_unfollowed_remote() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([3u8; 32]);
        let stranger = CollabSigner::from_seed([4u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // I do NOT follow the stranger, but I have their profile cached.
        let sp = stranger.sign_profile("s".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let sfp = fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let adverts = collab_adverts(&repo).await.unwrap();
        assert!(!adverts.iter().any(|r| r.name.as_str().contains(&sfp)),
            "unfollowed author's cache must NOT be advertised");
    }
```

Also update the existing WS8b test `collab_adverts_exclude_scoped`: change `let adverts = collab_adverts(&repo).unwrap();` to `let adverts = collab_adverts(&repo).await.unwrap();`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole sync::collab`
Expected: build error (`collab_adverts` is sync / new tests + the awaited call don't compile).

- [ ] **Step 3: Implement** — replace `collab_adverts` in `src/sync/collab.rs`:

```rust
// <bead-id>
/// Advertises the node's own public refs (`refs/collab/public/**`) plus the
/// cached refs (`refs/collab/remotes/<fp>/**`) of authors this node DIRECTLY
/// follows — and nothing else. Serve horizon: re-serve verified public state for
/// authors you directly follow, and for no others. Never advertises `scoped/`.
pub async fn collab_adverts(repo: &Repository) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    // Own authored public objects.
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
        }
    }
    // Cached objects of directly-followed authors, keyed by author fingerprint.
    for e in repo.public_edges().await? {
        if e.kind == crate::collab::TrustKind::Follow {
            let fp = crate::collab::fingerprint(&e.to_key);
            let prefix = format!("{COLLAB_REMOTES_PREFIX}{fp}/");
            for name in repo.refs.list(&prefix)? {
                if let Some(tag) = repo.refs.get_tag(&name)? {
                    out.push(RefAdvert { name, target: tag.target, is_timeline: false });
                }
            }
        }
    }
    Ok(out)
}
```

In `serve_collab`, change `let refs = collab_adverts(repo)?;` to `let refs = collab_adverts(repo).await?;`.

> **Implementer note:** `COLLAB_REMOTES_PREFIX` is `pub` in `src/repo/collab.rs`; import it at the top of `src/sync/collab.rs` if not already in scope (`use crate::repo::collab::COLLAB_REMOTES_PREFIX;`). `TrustKind`/`fingerprint` come from `crate::collab`.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole`
Expected: new tests + updated scoped test pass; no regressions.

- [ ] **Step 5: Commit**
```bash
git add src/sync/collab.rs
git commit -m "<bead-id>: serve horizon — advertise followed-author remotes (G1)"
```

---

## Task 2: Multi-author `collab_pull`

**Files:** Modify `src/sync/collab.rs`.

**Interfaces:**
- Consumes: `collab_adverts` (async, Task 1), `verified`/`author`/`fingerprint`/`kind_seg`, `COLLAB_PUBLIC_PREFIX`, `COLLAB_REMOTES_PREFIX`.
- Produces: `pub async fn collab_pull(conn, repo) -> Result<Key>` (returns the server's own key; stores ALL verified objects by intrinsic author).

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/sync/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn pull_files_cached_by_author() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX};

        // Server B: own profile (public/), follows C, and has C cached (remotes/<Cfp>/).
        let server = Repository::memory();
        let b = CollabSigner::from_seed([10u8; 32]);
        let c = CollabSigner::from_seed([11u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        // Client A pulls B.
        let client = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server).await });
        let peer = collab_pull(&mut cl, &client).await.unwrap();
        srv.await.unwrap().unwrap();

        assert_eq!(peer, b.public_key(), "returns the dialed server's own key");
        // B filed under remotes/<Bfp>/, C filed under remotes/<Cfp>/ — by intrinsic author.
        let bfp = fingerprint(&b.public_key());
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_some(),
            "server-own profile filed under its author");
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap()).unwrap().is_some(),
            "cached C profile filed under C, not under B");
        let _ = COLLAB_PUBLIC_PREFIX;
    }

    // <bead-id>
    #[tokio::test]
    async fn pull_drops_tampered_cached() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let server = Repository::memory();
        let b = CollabSigner::from_seed([12u8; 32]);
        let c = CollabSigner::from_seed([13u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        server.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // A TAMPERED C profile cached on B.
        let mut cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        cp.display_name = "tampered".into();
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let client = Repository::memory();
        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server).await });
        collab_pull(&mut cl, &client).await.unwrap();
        srv.await.unwrap().unwrap();

        let bfp = fingerprint(&b.public_key());
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_some(),
            "valid server profile kept");
        assert!(client.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/profile")).unwrap()).unwrap().is_none(),
            "tampered cached C profile dropped (no ref)");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole sync::collab::tests::pull_files_cached_by_author`
Expected: FAIL — current `collab_pull` drops non-peer authors (the `if author(obj) != peer continue`), so C is never filed.

- [ ] **Step 3: Implement** — in `src/sync/collab.rs` `collab_pull`, replace the filing loop (from `let fp = fingerprint(&peer);` through `tx.commit()?;`) with:

```rust
    // <bead-id>
    // Multi-author: file EVERY verified object under the puller's remotes namespace
    // keyed by its INTRINSIC author (server-own and forwarded-cache alike). The
    // `peer` (server's own key) is still returned for `discover pull`/`trust follow`.
    let mut tx = repo.refs.transaction();
    for (_, obj) in &resolved {
        let afp = fingerprint(&author(obj));
        let tracking = match obj {
            CollabObject::Profile(_) => format!("{COLLAB_REMOTES_PREFIX}{afp}/profile"),
            CollabObject::TrustEdge(e) => format!(
                "{COLLAB_REMOTES_PREFIX}{afp}/edge/{}/{}",
                kind_seg(e.kind),
                fingerprint(&e.to_key),
            ),
        };
        let target = repo.objects.put(&Object::Collab(obj.clone())).await?;
        tx.set(RefName::new(tracking)?, Ref::Tag(Tag { target, created_at: 0, message: None }));
    }
    tx.commit()?;
    Ok(peer)
```

Leave the earlier part of `collab_pull` (Hello/Welcome/HaveWant/Pack/decode/put_raw, the `resolved` verification loop, and the `peer` lookup) unchanged — `peer` is still "the author of a verified `Profile`," which for a well-behaved server is the object it advertised under `public/profile`. (Any verified profile establishes an identity; the server's own is present in `public/`.)

> **Implementer note:** delete the now-unused `let fp = fingerprint(&peer);` line and the old per-object `if author(obj) != peer { continue; }` filter. `kind_seg` is `pub(crate)` in `src/repo/collab.rs` (imported already from Task WS8b-x5u); keep that import.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole`
Expected: new tests pass; the WS8b `pull_stores_under_remote_prefix` / `pull_drops_tampered_object` / `pull_errors_with_no_valid_profile` still pass (single-author case is a subset).

- [ ] **Step 5: Commit**
```bash
git add src/sync/collab.rs
git commit -m "<bead-id>: multi-author collab_pull — file cache by intrinsic author (G2)"
```

---

## Task 3: `TrustGraph::follow_paths` (predecessor BFS)

**Files:** Modify `src/collab/trust.rs`.

**Interfaces:**
- Produces: `pub fn follow_paths(&self, root: &Key, hops: u8) -> BTreeMap<Key, Vec<Key>>` — each value is the minimal-hop path `[root, …, key]` (root excluded from the map).

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/collab/trust.rs`:

```rust
    // <bead-id>
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

    // <bead-id>
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
```

> **Implementer note:** the test module already has a helper `fn k(seed: u8) -> (CollabSigner, Key)` and imports `TrustKind`. If the helper's name/shape differs, match the existing tests in the file.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole collab::trust`
Expected: FAIL (`follow_paths` undefined).

- [ ] **Step 3: Implement** — add to `impl TrustGraph` in `src/collab/trust.rs`:

```rust
    // <bead-id>
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
                if !paths.contains_key(&next) {
                    let mut p = node_path.clone();
                    p.push(next);
                    paths.insert(next, p);
                    q.push_back(next);
                }
            }
        }
        paths.remove(root);
        paths
    }
```

> **Implementer note:** `self.follows(&node)` returns `Vec<Key>` (owned) in the current code; iterating yields `Key` (Copy). `VecDeque` and `BTreeMap` are already imported in this file (used by `follow_neighborhood`).

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole collab::trust` then `cargo test -p bole`
Expected: pass.

- [ ] **Step 5: Commit**
```bash
git add src/collab/trust.rs
git commit -m "<bead-id>: TrustGraph follow_paths (predecessor BFS for trust_path) (G3)"
```

---

## Task 4: `local_discovery_index` emits real `trust_path`

**Files:** Modify `src/repo/collab.rs`.

**Interfaces:**
- Consumes: `TrustGraph::follow_paths` (Task 3).
- Produces: `local_discovery_index` now emits `trust_path = [self, …, author]` (unchanged signature).

- [ ] **Step 1: Write the failing test** — add to the test module in `src/repo/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn index_emits_depth2_path() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([70u8; 32]);
        let b = CollabSigner::from_seed([71u8; 32]);
        let c = CollabSigner::from_seed([72u8; 32]);
        // me -follow-> b ; and I have cached b's profile, b's follow-edge to c, and c's profile.
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

        async fn cache(repo: &Repository, obj: CollabObject) {
            let author = match &obj { CollabObject::Profile(p) => p.key, CollabObject::TrustEdge(e) => e.from_key };
            let leaf = match &obj {
                CollabObject::Profile(_) => "profile".to_string(),
                CollabObject::TrustEdge(e) => format!("edge/follow/{}", fingerprint(&e.to_key)),
            };
            let id = repo.objects.put(&Object::Collab(obj)).await.unwrap();
            let fp = fingerprint(&author);
            let mut tx = repo.refs.transaction();
            tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/{leaf}")).unwrap(),
                   Ref::Tag(Tag { target: id, created_at: 0, message: None }));
            tx.commit().unwrap();
        }
        cache(&repo, CollabObject::Profile(b.sign_profile("bob".into(), String::new(), vec![], vec![], 1))).await;
        cache(&repo, CollabObject::TrustEdge(b.sign_edge(c.public_key(), TrustKind::Follow, None, 1))).await;
        cache(&repo, CollabObject::Profile(c.sign_profile("cee".into(), String::new(), vec![], vec![], 1))).await;

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        let cee = idx.query("cee");
        assert_eq!(cee.len(), 1);
        assert_eq!(cee[0].distance, 2, "c reached at depth 2 via cache-forward");
        assert_eq!(cee[0].trust_path, vec![me.public_key(), b.public_key(), c.public_key()], "path [me,b,c]");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole repo::collab::tests::index_emits_depth2_path`
Expected: FAIL — current code emits `trust_path = [self, author]` (`[me, c]`), not `[me, b, c]`.

- [ ] **Step 3: Implement** — in `src/repo/collab.rs` `local_discovery_index`, replace the neighborhood + pulled-assembly block. Change `let neighborhood = graph.follow_neighborhood(self_key, hops);` to `let paths = graph.follow_paths(self_key, hops);`, and replace the `for (author, objs) in by_author { … }` loop with:

```rust
        // <bead-id>
        let mut pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)> = Vec::new();
        for (author, objs) in by_author {
            if let Some(path) = paths.get(&author) {
                let dist = (path.len() as u8) - 1;
                pulled.push((author, dist, path.clone(), objs));
            }
        }
```

(Delete the old `TODO(WS8c)` comment and the `vec![*self_key, author]` stub.)

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole repo::collab` then `cargo test -p bole`
Expected: new test passes; existing `local_index_ranks_by_distance` / `local_index_excludes_unfollowed` still pass (distance-1 path is `[self, author]`, unchanged).

- [ ] **Step 5: Commit**
```bash
git add src/repo/collab.rs
git commit -m "<bead-id>: local_discovery_index emits real depth-2 trust_path (G4)"
```

---

## Task 5: `query_discovery` + `QueryHit` (petname resolution)

**Files:** Modify `src/repo/collab.rs`, `src/lib.rs`.

**Interfaces:**
- Consumes: `local_discovery_index` (Task 4), `Namer`/`PetnameResolution` (`crate::collab::naming`), `TrustGraph`.
- Produces: `pub struct QueryHit { pub key: Key, pub display_name: String, pub petname: Option<String>, pub reach: u8, pub trust_path: Vec<Key> }`; `pub async fn query_discovery(&self, self_key: &Key, hops: u8, term: &str) -> Result<Vec<QueryHit>>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/repo/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn query_resolves_vouch_petname() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([80u8; 32]);
        let b = CollabSigner::from_seed([81u8; 32]);
        // me follows b AND vouches b as "bee".
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Vouch, Some("bee".into()), 1)).await.unwrap();
        // b's profile cached.
        let bp = b.sign_profile("bob-selfname".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(bp))).await.unwrap();
        let bfp = fingerprint(&b.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let hits = repo.query_discovery(&me.public_key(), 2, "bob-selfname").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display_name, "bob-selfname", "self-asserted name is a hint");
        assert_eq!(hits[0].petname.as_deref(), Some("bee"), "trust-graph petname resolved");
        assert_eq!(hits[0].reach, 1, "direct follow");
        assert_eq!(hits[0].trust_path, vec![me.public_key(), b.public_key()]);
    }

    // <bead-id>
    #[tokio::test]
    async fn query_reach_and_path() {
        use crate::collab::CollabSigner;
        let repo = Repository::memory();
        let me = CollabSigner::from_seed([82u8; 32]);
        repo.publish_profile(&me.sign_profile("myself".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let hits = repo.query_discovery(&me.public_key(), 2, "myself").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].reach, 0, "own profile is self");
        assert_eq!(hits[0].petname, None, "no vouch for self -> fingerprint fallback -> None");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole repo::collab::tests::query_resolves_vouch_petname`
Expected: FAIL (`query_discovery`/`QueryHit` undefined).

- [ ] **Step 3: Implement** — add near the top of `src/repo/collab.rs` (after the imports) the struct, and the method to `impl Repository`:

```rust
// <bead-id>
/// One resolved discovery hit for the CLI: the canonical key, the author's
/// self-asserted display name (a hint), the trust-graph-resolved petname (None
/// when only the fingerprint is known), the reach distance (0/1/2), and the
/// minimal-hop trust path.
#[derive(Debug, Clone)]
pub struct QueryHit {
    pub key: Key,
    pub display_name: String,
    pub petname: Option<String>,
    pub reach: u8,
    pub trust_path: Vec<Key>,
}
```

```rust
    // <bead-id>
    /// Runs the local discovery index for `term` and resolves a trust-scoped
    /// petname (via `Namer` over the combined follow/vouch graph; fingerprint
    /// fallback → None) plus reach + trust path for each hit.
    pub async fn query_discovery(&self, self_key: &Key, hops: u8, term: &str) -> Result<Vec<QueryHit>> {
        use crate::collab::naming::{Namer, PetnameResolution};
        let idx = self.local_discovery_index(self_key, hops).await?;

        // Rebuild the combined edge graph for petname resolution.
        let mut edges: Vec<TrustEdge> = self.public_edges().await?;
        for o in self.tracked_collab().await? {
            if let CollabObject::TrustEdge(e) = o {
                edges.push(e);
            }
        }
        let graph = TrustGraph::from_edges(edges);
        let local: std::collections::BTreeMap<Key, String> = std::collections::BTreeMap::new();
        let namer = Namer::new(*self_key, &local, &graph);

        let mut hits = Vec::new();
        for r in idx.query(term) {
            let display_name = match &r.object {
                CollabObject::Profile(p) => p.display_name.clone(),
                CollabObject::TrustEdge(_) => String::new(),
            };
            let petname = match namer.resolve(&r.key) {
                PetnameResolution::Local(n) => Some(n),
                PetnameResolution::Vouch { name, .. } => Some(name),
                PetnameResolution::Fingerprint(_) => None,
            };
            hits.push(QueryHit {
                key: r.key,
                display_name,
                petname,
                reach: r.distance,
                trust_path: r.trust_path.clone(),
            });
        }
        Ok(hits)
    }
```

Re-export in `src/lib.rs`:
```rust
// <bead-id>
pub use repo::collab::QueryHit;
```

> **Implementer note:** confirm `Namer::new(root: Key, local: &BTreeMap<Key,String>, graph: &TrustGraph)` and the `PetnameResolution` variants (`Local(String)`, `Vouch { name, depth, path }`, `Fingerprint(String)`) against `src/collab/naming.rs`; match exactly. `TrustGraph`/`TrustEdge`/`CollabObject`/`Key` are already imported in `repo/collab.rs`. If `repo::collab::QueryHit` isn't publicly reachable for the re-export, ensure `mod collab` is `pub` (it is per WS8a).

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole repo::collab` then `cargo test -p bole`
Expected: both new tests pass.

- [ ] **Step 5: Commit**
```bash
git add src/repo/collab.rs src/lib.rs
git commit -m "<bead-id>: query_discovery with Namer petname + reach + trust_path (G5)"
```

---

## Task 6: CLI `discover query` — petname/reach/trust_path output

**Files:** Modify `bole-cli/src/commands/discover.rs`.

**Interfaces:**
- Consumes: `Repository::query_discovery` + `QueryHit` (Task 5); `crate::key::hex32`.

- [ ] **Step 1: Write the failing test** — add to `bole-cli/tests/collab_cli.rs`:

```rust
// <bead-id>
#[test]
fn cli_query_shows_petname_and_reach() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "a1".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Myself"], Some(&seed));
    let out = ok(w, &["discover", "query", "Myself", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let row = &v[0];
    assert_eq!(row["reach"], "self");
    assert!(row.get("display_name").is_some(), "display_name field present");
    assert!(row.get("petname").is_some(), "petname field present (may be null)");
    assert!(row.get("trust_path").is_some(), "trust_path field present");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-cli --test collab_cli -- cli_query_shows_petname_and_reach`
Expected: FAIL (current JSON emits `name`/`distance`, not `reach`/`petname`/`trust_path`).

- [ ] **Step 3: Implement** — in `bole-cli/src/commands/discover.rs`, replace the `Cmd::Query` body's row construction:

```rust
        // <bead-id>
        Cmd::Query { term, hops, key_env, key_file } => {
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            let hits = ctx.repo.query_discovery(&self_key, hops, &term).await?;
            let rows: Vec<_> = hits
                .iter()
                .map(|h| {
                    let reach = match h.reach {
                        0 => "self",
                        1 => "direct",
                        _ => "transitive",
                    };
                    serde_json::json!({
                        "key": key::hex32(&h.key),
                        "display_name": h.display_name,
                        "petname": h.petname,
                        "reach": reach,
                        "trust_path": h.trust_path.iter().map(key::hex32).collect::<Vec<_>>(),
                    })
                })
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no matches".to_string()
                    } else {
                        rows.iter()
                            .map(|r| format!("{} [{}] {}", r["key"], r["reach"], r["display_name"]))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
```

> **Implementer note:** match the existing `Cmd::Query` arm's surrounding structure (the `out.emit(human, json)` call and the `Ok(())` return) — only the row mapping + `query_discovery` call change. `key::hex32` takes `&[u8;32]`; `h.trust_path.iter().map(key::hex32)` passes `&Key` — if the closure-coercion doesn't infer, use `.map(|k| key::hex32(k))`.

- [ ] **Step 4: Run to verify GREEN**

Run: `cargo test -p bole-cli --test collab_cli` then `cargo build --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: the new test + existing CLI tests pass; clippy clean.

- [ ] **Step 5: Commit**
```bash
git add bole-cli/src/commands/discover.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: CLI discover query — petname/reach/trust_path (G6)"
```

---

## Task 7: Integration + E2E (cache-forward, negatives, 3-node CLI)

**Files:** Modify `tests/collab_network.rs`, `bole-cli/tests/collab_cli.rs`.

**Interfaces:** Consumes everything above.

- [ ] **Step 1: Write the failing integration tests** — add to `tests/collab_network.rs`:

```rust
// <bead-id>
// Helper: publish a profile + follow edge on `node` for `who`, and (optionally)
// seed `node`'s cache with objects authored by a third party.
async fn seed_profile(node: &Repository, who: &bole::collab::CollabSigner, name: &str) {
    node.publish_profile(&who.sign_profile(name.into(), String::new(), vec![], vec![], 1)).await.unwrap();
}

#[tokio::test]
async fn loopback_cache_forward_depth2() {
    use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
    use bole::object::Object;
    use bole::refs::{Ref, RefName, Tag};
    use bole::repo::collab::COLLAB_REMOTES_PREFIX;

    // B follows C and has C cached. A follows B and pulls B; A must gain C at depth 2.
    let bnode = Repository::memory();
    let b = CollabSigner::from_seed([30u8; 32]);
    let c = CollabSigner::from_seed([31u8; 32]);
    seed_profile(&bnode, &b, "bob").await;
    bnode.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    // cache C's profile + C's own follow-edge (harmless) on B
    let cfp = fingerprint(&c.public_key());
    for (leaf, obj) in [
        ("profile".to_string(), CollabObject::Profile(c.sign_profile("cee".into(), String::new(), vec![], vec![], 1))),
    ] {
        let id = bnode.objects.put(&Object::Collab(obj)).await.unwrap();
        let mut tx = bnode.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{cfp}/{leaf}")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();
    }

    let anode = Repository::memory();
    let a = CollabSigner::from_seed([32u8; 32]);
    seed_profile(&anode, &a, "alice").await;
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { bole::sync::collab::serve_collab_tcp_once(&listener, &bnode).await });
    let mut conn = connect(addr).await;
    bole::sync::collab::collab_pull(&mut conn, &anode).await.unwrap();
    srv.await.unwrap().unwrap();

    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    let cee = idx.query("cee");
    assert_eq!(cee.len(), 1, "C discoverable via cache-forward");
    assert_eq!(cee[0].distance, 2);
    assert_eq!(cee[0].trust_path, vec![a.public_key(), b.public_key(), c.public_key()]);
}

#[tokio::test]
async fn loopback_over_depth_excluded() {
    use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
    use bole::object::Object;
    use bole::refs::{Ref, RefName, Tag};
    use bole::repo::collab::COLLAB_REMOTES_PREFIX;

    // B follows C only (NOT D). B has BOTH C and D cached (D arrived via C earlier).
    // A follows B, pulls B: A must get C (depth-2) but never D (depth-3).
    let bnode = Repository::memory();
    let b = CollabSigner::from_seed([33u8; 32]);
    let c = CollabSigner::from_seed([34u8; 32]);
    let d = CollabSigner::from_seed([35u8; 32]);
    seed_profile(&bnode, &b, "bob").await;
    bnode.publish_edge(&b.sign_edge(c.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    for (signer, name) in [(&c, "cee"), (&d, "dee")] {
        let fp = fingerprint(&signer.public_key());
        let id = bnode.objects.put(&Object::Collab(CollabObject::Profile(
            signer.sign_profile(name.into(), String::new(), vec![], vec![], 1)))).await.unwrap();
        let mut tx = bnode.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();
    }

    let anode = Repository::memory();
    let a = CollabSigner::from_seed([36u8; 32]);
    seed_profile(&anode, &a, "alice").await;
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { bole::sync::collab::serve_collab_tcp_once(&listener, &bnode).await });
    let mut conn = connect(addr).await;
    bole::sync::collab::collab_pull(&mut conn, &anode).await.unwrap();
    srv.await.unwrap().unwrap();

    // D was never advertised by B (D not in B's follow set), so A never received it.
    let dfp = fingerprint(&d.public_key());
    assert!(anode.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{dfp}/profile")).unwrap()).unwrap().is_none(),
        "D never forwarded (over-depth)");
    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(idx.query("dee").is_empty(), "D never surfaces in discovery");
    assert!(!idx.query("cee").is_empty(), "C still reachable at depth 2");
}
```

> **Implementer note:** `tests/collab_network.rs` already has a `connect(addr)` helper and imports from WS8b — reuse them; add only what's new. The `loopback_scoped_and_unfollowed_never_forwarded` and `loopback_tampered_cached_dropped` cases are covered by the Task-1/Task-2 unit tests (`adverts_exclude_unfollowed_remote`, `collab_adverts_exclude_scoped`, `pull_drops_tampered_cached`); if you prefer an integration-level restatement, add one mirroring `loopback_over_depth_excluded` with a scoped/tampered object on B and asserting absence on A — otherwise cite the unit coverage in your report.

- [ ] **Step 2: Write the failing CLI E2E** — add to `bole-cli/tests/collab_cli.rs`:

```rust
// <bead-id>
#[test]
fn cli_three_node_transitive() {
    use std::process::Stdio;
    // C serves; B follows+pulls C, then B serves; A follows+pulls B; A query finds C transitive.
    fn serve(dir: &std::path::Path, addr: &str) -> std::process::Child {
        let mut cmd = bin();
        cmd.args(["node", "serve", "--listen", addr]).current_dir(dir)
            .stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().unwrap()
    }

    // C
    let ctmp = tempfile::tempdir().unwrap(); let c = ctmp.path();
    ok(c, &["init", "."], None);
    let cseed = "c1".repeat(32);
    ok(c, &["profile", "set", "--display-name", "Carol"], Some(&cseed));
    let caddr = "127.0.0.1:47701";
    let mut cchild = serve(c, caddr);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // B follows C, pulls C, then serves
    let btmp = tempfile::tempdir().unwrap(); let b = btmp.path();
    ok(b, &["init", "."], None);
    let bseed = "b1".repeat(32);
    ok(b, &["profile", "set", "--display-name", "Bob"], Some(&bseed));
    let cpull = ok(b, &["discover", "pull", caddr, "--json"], Some(&bseed));
    let ckey = serde_json::from_slice::<serde_json::Value>(&cpull.stdout).unwrap()["pulled"].as_str().unwrap().to_string();
    ok(b, &["trust", "follow", &ckey], Some(&bseed));
    let _ = cchild.kill(); let _ = cchild.wait();
    let baddr = "127.0.0.1:47702";
    let mut bchild = serve(b, baddr);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // A follows B, pulls B, queries "Carol" -> transitive
    let atmp = tempfile::tempdir().unwrap(); let a = atmp.path();
    ok(a, &["init", "."], None);
    let aseed = "a2".repeat(32);
    ok(a, &["profile", "set", "--display-name", "Alice"], Some(&aseed));
    let bpull = ok(a, &["discover", "pull", baddr, "--json"], Some(&aseed));
    let bkey = serde_json::from_slice::<serde_json::Value>(&bpull.stdout).unwrap()["pulled"].as_str().unwrap().to_string();
    ok(a, &["trust", "follow", &bkey], Some(&aseed));
    let q = ok(a, &["discover", "query", "Carol", "--json"], Some(&aseed));
    let _ = bchild.kill(); let _ = bchild.wait();

    let v: serde_json::Value = serde_json::from_slice(&q.stdout).unwrap();
    let carol = v.as_array().unwrap().iter().find(|r| r["display_name"] == "Carol");
    assert!(carol.is_some(), "Carol discoverable transitively: {}", String::from_utf8_lossy(&q.stdout));
    assert_eq!(carol.unwrap()["reach"], "transitive");
}
```

> **Implementer note:** this E2E requires B to actually *cache* C before serving — `discover pull` (Task 2) stores C under B's `remotes/<Cfp>/`, and `trust follow <ckey>` makes C a followed author so B's `collab_adverts` (Task 1) will forward C. Fixed ports (47701/47702) carry the same parallel-run caveat noted in WS8b; raise the sleep or use port 0 + captured addr if flaky. Reap every spawned `node serve` child (`kill()` + `wait()`).

- [ ] **Step 3: Run RED→GREEN**

Run: `cargo test -p bole --test collab_network` then `cargo test -p bole-cli --test collab_cli` then `cargo test --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass; clippy clean.

- [ ] **Step 4: Commit**
```bash
git add tests/collab_network.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: cache-forward + over-depth integration and 3-node CLI E2E (G7)"
```

---

## Self-Review

**Spec coverage:** §2 serve horizon → Task 1 (G1). §3 multi-author pull + storage story → Task 2 (G2). §4 predecessor BFS + real trust_path → Tasks 3–4 (G3, G4). §5 petname-aware query → Tasks 5–6 (G5, G6). §6 testing incl. negative tamper + over-depth → Task 7 (G7) plus Task-1/2 unit negatives. §1 invariants (cached≠authored, fail-closed pull+index, depth-2 both layers) enforced by the verify-per-object logic (Task 2), `tracked_collab` re-verify (already shipped), and the hops=2 cap (Tasks 3–4). §7 scope boundary → nothing here builds relays, stranger-search, poll, DNS, local-petname, or concurrent-serve.

**Placeholder scan:** No "TBD"/"handle errors". `Implementer note` blocks flag only codebase-confirmation points (async ripple, `Namer::new` signature, `key::hex32` closure coercion, existing test helpers) — each with the exact resolution named.

**Type consistency:** `collab_adverts` async everywhere it's called (serve_collab + updated WS8b test); `follow_paths(&Key,u8)->BTreeMap<Key,Vec<Key>>` consumed by `local_discovery_index`; `QueryHit{key,display_name,petname,reach,trust_path}` produced by `query_discovery` and consumed by `discover.rs`; `reach` = distance 0/1/2 mapped to self/direct/transitive; `key::hex32` for all displayed keys. `Index::query` returns `&DiscoveryResult{key,object,distance,trust_path}` (WS8a) — `trust_path` now the real path from Task 4.

**Flagged risks:** (1) making `collab_adverts` async ripples to `serve_collab` and the WS8b `collab_adverts_exclude_scoped` test — both updated in Task 1. (2) `Namer`/`PetnameResolution` module path must be confirmed (`crate::collab::naming`). (3) fixed E2E ports carry the WS8b parallel-run flake caveat.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-03-ws8c-cache-and-forward.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task (one bead + branch each), review between tasks.

**2. Inline Execution** — execute tasks here in batches with checkpoints.

Which approach?
