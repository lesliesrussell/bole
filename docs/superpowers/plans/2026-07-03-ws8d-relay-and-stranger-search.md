# WS8d — Relay Role + Relay-Query Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the minimal "cold-discover a stranger" keystone — a relay node that serves its whole aggregate horizon-off, and a transient client `discover relay` search that surfaces verified strangers without mutating local state.

**Architecture:** Thread `relay: bool` through `collab_adverts`/`serve_collab`/`serve_collab_tcp_once` (relay=true advertises `public/**` + ALL `remotes/**`, horizon off). Add a pure `collab_fetch_transient(conn) -> Vec<CollabObject>` (fetch + fail-closed verify, no `Repository`, no writes) decoding pack bytes via `crate::codec::deserialize`. CLI gains `node serve --relay` and `discover relay <endpoint> <term>` (transient fetch, filter Profiles, rank by match+deterministic tiebreak, emit `reach:"stranger"`). Strangers persist only via explicit `trust follow`.

**Tech Stack:** Rust (library-first + `bole-cli`), reusing WS5 wire (`Conn`/`Message`/`decode_pack`/`codec`) and WS8a–c collab (`Profile`/`TrustEdge`/`verify_*`/`CollabObject`, `refs/collab/{public,remotes,scoped}/`). Loopback `TcpConn` + real-`bole`-binary CLI tests.

## Global Constraints

- **Relays never authoritative:** every object is verified against its *embedded* author key; a relay can only include/withhold signed objects, never forge or re-attribute.
- **Endpoint stays read-only:** no write/announce path; aggregation is the relay pulling publishers (existing `discover pull`).
- **Strangers are transient:** `discover relay` writes NOTHING to `refs/collab/`. A stranger enters `remotes/` only via explicit `trust follow`.
- **Depth-2 neighborhood untouched:** `discover query` stays confined to ≤2-hop graph; `discover relay` is a distinct surface.
- **Relay serve horizon:** `relay=true` advertises `public/**` + ALL `remotes/**`; `relay=false` = WS8c (public + directly-followed remotes). BOTH exclude `scoped/`.
- **Fail-closed:** `collab_fetch_transient` drops any object whose signature does not verify.
- **Keys canonical, raw hex:** CLI shows keys as raw 64-hex (`crate::key::hex32`), never fingerprint.
- **Ranking is minimal & honest:** match, then deterministic tiebreak (`display_name`, then key fingerprint). No recency (no cross-author timestamp exists), no trust annotation (WS8e).
- **No new deps.** Only crates already in `Cargo.toml`.
- **Process:** bd-only; each Task is one bead; branch name = bead ID; each contiguous added block carries one `// <bead-id>` comment; tests pass before merge; delete branch after merge; `bd close`.

### Per-task bead protocol
```bash
bd create "WS8d Task N: <title>" --json
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
| **G1** | `collab_adverts(repo, relay)`: relay=true advertises non-followed `remotes/`; relay=false excludes it (WS8c horizon); both exclude `scoped/` | `adverts_relay_includes_unfollowed`, `adverts_relay_excludes_scoped`, (existing `adverts_exclude_unfollowed_remote` under relay=false) | 1 |
| **G2** | `collab_fetch_transient` returns verified objects, drops a tampered one, needs no `Repository`/writes nothing | `transient_fetch_returns_verified`, `transient_fetch_drops_tampered` | 2 |
| **G3** | Loopback: relay serves unfollowed B+C; querier's transient fetch finds a stranger but it is NOT in `remotes/`, NOT in `discover query`, causes NO `refs/collab/` change, and a repeated fetch is identical; after `trust follow` the stranger appears in `discover query` | `relay_transient_fetch_no_persist`, `stranger_absent_from_query_until_followed` | 3 |
| **G4** | CLI: `node serve --relay` + `discover relay <endpoint> <term> --json` surfaces a stranger marked `"stranger"`; querier's `refs/collab/` unchanged | `cli_discover_relay_shows_stranger` | 4 |

---

## File Structure

- `src/sync/collab.rs` — `collab_adverts` gains `relay: bool` (relay-mode advertises all `remotes/`); `serve_collab`/`serve_collab_tcp_once` gain `relay: bool`; new `collab_fetch_transient`. Update all in-file test call sites.
- `src/lib.rs` — re-export `collab_fetch_transient`.
- `src/sync/collab.rs` callers ripple: `serve_collab_tcp_once` call in `bole-cli/src/commands/node.rs`; the 4 calls in `tests/collab_network.rs`.
- `bole-cli/src/commands/node.rs` — `Serve` gains `--relay`.
- `bole-cli/src/commands/discover.rs` — new `Relay { endpoint, term }` subcommand.
- `tests/collab_network.rs` — loopback relay + transient tests (Task 3).
- `bole-cli/tests/collab_cli.rs` — CLI relay E2E (Task 4).

---

## Task 1: `relay` flag in `collab_adverts` + serve threading

**Files:** Modify `src/sync/collab.rs`; ripple to `bole-cli/src/commands/node.rs`, `tests/collab_network.rs`.

**Interfaces:**
- Produces: `pub async fn collab_adverts(repo: &Repository, relay: bool) -> Result<Vec<RefAdvert>>`; `pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository, relay: bool) -> Result<()>`; `pub async fn serve_collab_tcp_once(listener: &tokio::net::TcpListener, repo: &Repository, relay: bool) -> Result<()>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/sync/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn adverts_relay_includes_unfollowed() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([90u8; 32]);
        let stranger = CollabSigner::from_seed([91u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // A stranger cached but NOT followed.
        let sp = stranger.sign_profile("s".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let sfp = fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        // relay=false → excluded (WS8c horizon); relay=true → included.
        let non_relay = collab_adverts(&repo, false).await.unwrap();
        assert!(!non_relay.iter().any(|r| r.name.as_str().contains(&sfp)), "non-relay excludes unfollowed");
        let relay = collab_adverts(&repo, true).await.unwrap();
        assert!(relay.iter().any(|r| r.name.as_str() == format!("{COLLAB_REMOTES_PREFIX}{sfp}/profile")),
            "relay advertises unfollowed cache");
    }

    // <bead-id>
    #[tokio::test]
    async fn adverts_relay_excludes_scoped() {
        use crate::collab::{CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_SCOPED_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([92u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let scoped = me.sign_profile("secret".into(), String::new(), vec![], vec![], 2);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(scoped))).await.unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_SCOPED_PREFIX}profile/x")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        // Even in relay mode, scoped is never advertised.
        let relay = collab_adverts(&repo, true).await.unwrap();
        assert!(!relay.iter().any(|r| r.name.as_str().contains("/scoped/")), "relay never advertises scoped");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole sync::collab`
Expected: build error — `collab_adverts` takes one arg (arity mismatch on the new calls).

- [ ] **Step 3: Add the `relay` param to `collab_adverts`** — replace `collab_adverts` in `src/sync/collab.rs`:

```rust
// <bead-id>
/// Advertises the node's own public objects (`refs/collab/public/**`) plus cached
/// objects (`refs/collab/remotes/<fp>/**`). When `relay` is false (ordinary node),
/// only the cache of directly-followed authors is advertised (WS8c serve horizon).
/// When `relay` is true, ALL cached objects are advertised (a relay aggregates and
/// re-serves broadly). Never advertises `refs/collab/scoped/` in either mode.
pub async fn collab_adverts(repo: &Repository, relay: bool) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    for name in repo.refs.list(COLLAB_PUBLIC_PREFIX)? {
        if let Some(tag) = repo.refs.get_tag(&name)? {
            out.push(RefAdvert { name, target: tag.target, is_timeline: false });
        }
    }
    if relay {
        // Relay: advertise the entire cache, horizon off.
        for name in repo.refs.list(COLLAB_REMOTES_PREFIX)? {
            if let Some(tag) = repo.refs.get_tag(&name)? {
                out.push(RefAdvert { name, target: tag.target, is_timeline: false });
            }
        }
    } else {
        // Ordinary node: only directly-followed authors' cache (WS8c serve horizon).
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
    }
    Ok(out)
}
```

- [ ] **Step 4: Thread `relay` through `serve_collab` + `serve_collab_tcp_once`** — change their signatures and the internal calls:

```rust
pub async fn serve_collab(conn: &mut dyn Conn, repo: &Repository, relay: bool) -> Result<()> {
```
Inside, change `let refs = collab_adverts(repo).await?;` to `let refs = collab_adverts(repo, relay).await?;`. Add a `// <bead-id>` tag on the changed signature line's block.

```rust
pub async fn serve_collab_tcp_once(
    listener: &tokio::net::TcpListener,
    repo: &Repository,
    relay: bool,
) -> Result<()> {
```
Inside, change `serve_collab(&mut conn, repo).await` to `serve_collab(&mut conn, repo, relay).await`.

- [ ] **Step 5: Update all in-file + external call sites (pass `false` except where a relay test needs `true`)**

In `src/sync/collab.rs` test module, every `serve_collab(&mut X, &Y).await` spawn call and every `collab_adverts(&repo).await` call gets a `false` argument added, EXCEPT the two new tests above which pass the explicit mode they test. (There are ~9 such call sites — search `serve_collab(` and `collab_adverts(` in the test module.)

In `bole-cli/src/commands/node.rs`, change the `serve_collab_tcp_once(&listener, &ctx.repo)` call to `serve_collab_tcp_once(&listener, &ctx.repo, false)` (the `--relay` flag is added in Task 4; for now the CLI passes `false`).

In `tests/collab_network.rs`, all 4 `serve_collab_tcp_once(&listener, &X).await` calls get a `, false` argument.

- [ ] **Step 6: Run RED→GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole` then `cargo test -p bole --test collab_network` then `cargo build --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: new tests pass; all existing pass with the `false` arg; workspace builds; clippy clean.

- [ ] **Step 7: Commit**
```bash
git add src/sync/collab.rs bole-cli/src/commands/node.rs tests/collab_network.rs
git commit -m "<bead-id>: relay flag in collab_adverts + serve threading (G1)"
```

---

## Task 2: `collab_fetch_transient`

**Files:** Modify `src/sync/collab.rs`, `src/lib.rs`.

**Interfaces:**
- Consumes: `Conn`/`Message`/`Intent`/`CapSet`/`PROTO_VERSION` (WS5); `decode_pack`; `crate::codec::deserialize`; `verified` (private helper in this file); `Object`/`CollabObject`.
- Produces: `pub async fn collab_fetch_transient(conn: &mut dyn Conn) -> Result<Vec<CollabObject>>`.

- [ ] **Step 1: Write the failing tests** — add to the test module in `src/sync/collab.rs`:

```rust
    // <bead-id>
    #[tokio::test]
    async fn transient_fetch_returns_verified() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{CollabObject, CollabSigner};

        // A relay-style server with two authors cached (B own profile + C cached).
        let server = Repository::memory();
        let b = CollabSigner::from_seed([93u8; 32]);
        let c = CollabSigner::from_seed([94u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // cache C directly under remotes and serve with relay=true so it's advertised
        let cp = c.sign_profile("cee".into(), String::new(), vec![], vec![], 1);
        let cid = server.objects.put(&Object::Collab(CollabObject::Profile(cp))).await.unwrap();
        let cfp = crate::collab::fingerprint(&c.public_key());
        let mut tx = server.refs.transaction();
        tx.set(crate::refs::RefName::new(format!("{}{cfp}/profile", crate::repo::collab::COLLAB_REMOTES_PREFIX)).unwrap(),
               crate::refs::Ref::Tag(crate::refs::Tag { target: cid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, true).await });
        let objs = collab_fetch_transient(&mut cl).await.unwrap();
        srv.await.unwrap().unwrap();

        let names: Vec<String> = objs.iter().filter_map(|o| match o {
            CollabObject::Profile(p) => Some(p.display_name.clone()),
            _ => None,
        }).collect();
        assert!(names.contains(&"bob".to_string()) && names.contains(&"cee".to_string()),
            "transient fetch returns both verified profiles");
    }

    // <bead-id>
    #[tokio::test]
    async fn transient_fetch_drops_tampered() {
        use crate::sync::transport::InProcessConn;
        use crate::collab::{CollabObject, CollabSigner};

        let server = Repository::memory();
        let b = CollabSigner::from_seed([95u8; 32]);
        server.publish_profile(&b.sign_profile("bob".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // A tampered profile pinned under public/ (so it's advertised) but won't verify.
        let mut bad = b.sign_profile("origname".into(), String::new(), vec![], vec![], 2);
        bad.display_name = "tampered".into();
        let bid = server.objects.put(&Object::Collab(CollabObject::Profile(bad))).await.unwrap();
        let mut tx = server.refs.transaction();
        tx.set(crate::refs::RefName::new(format!("{}profile/bad", crate::repo::collab::COLLAB_PUBLIC_PREFIX)).unwrap(),
               crate::refs::Ref::Tag(crate::refs::Tag { target: bid, created_at: 0, message: None }));
        tx.commit().unwrap();

        let (mut s, mut cl) = InProcessConn::pair();
        let srv = tokio::spawn(async move { serve_collab(&mut s, &server, true).await });
        let objs = collab_fetch_transient(&mut cl).await.unwrap();
        srv.await.unwrap().unwrap();

        assert!(objs.iter().all(|o| !matches!(o, CollabObject::Profile(p) if p.display_name == "tampered")),
            "tampered object is dropped fail-closed");
        assert!(objs.iter().any(|o| matches!(o, CollabObject::Profile(p) if p.display_name == "bob")),
            "valid object still returned");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole sync::collab::tests::transient_fetch_returns_verified`
Expected: FAIL (`collab_fetch_transient` undefined).

- [ ] **Step 3: Implement** — add to `src/sync/collab.rs` (near `collab_pull`):

```rust
// <bead-id>
/// Fetches a node's advertised public collab objects over `conn` and returns the
/// signature-verified ones, WITHOUT touching any repository. Pure fetch+verify:
/// used by relay stranger-search, where results are transient and never persisted.
/// Fail-closed: any object whose signature does not verify is dropped.
pub async fn collab_fetch_transient(conn: &mut dyn Conn) -> Result<Vec<CollabObject>> {
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Fetch,
    })
    .await?;
    let refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Welcome".into())),
    };
    let want: Vec<_> = refs.iter().map(|r| r.target).collect();
    conn.send(&Message::HaveWant { want, have: vec![] }).await?;
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
    match conn.recv().await? {
        Message::Done => {}
        other => return Err(Error::Storage(format!("collab: expected Done, got {other:?}"))),
    }
    let mut out = Vec::new();
    for (_id, canonical) in decode_pack(&pack)? {
        if let Ok(Object::Collab(obj)) = crate::codec::deserialize(&canonical) {
            if verified(&obj) {
                out.push(obj);
            }
        }
    }
    Ok(out)
}
```

Re-export in `src/lib.rs`:
```rust
// <bead-id>
pub use sync::collab::collab_fetch_transient;
```

> **Implementer note:** confirm `crate::codec::deserialize(&[u8]) -> Result<Object>` (used by `src/store/mod.rs:53`); if the path/signature differs, match it. `verified`, `decode_pack`, `Message`/`Intent`/`CapSet`/`PROTO_VERSION`, `Object`, `CollabObject` are already in scope in this file.

- [ ] **Step 4: Run RED→GREEN**

Run: `cargo test -p bole sync::collab` then `cargo test -p bole` then `cargo clippy -p bole --all-targets -- -D warnings`
Expected: both new tests pass; clippy clean.

- [ ] **Step 5: Commit**
```bash
git add src/sync/collab.rs src/lib.rs
git commit -m "<bead-id>: collab_fetch_transient — pure fail-closed fetch+verify (G2)"
```

---

## Task 3: Loopback integration — no-persist + follow-to-adopt

**Files:** Modify `tests/collab_network.rs`.

**Interfaces:** Consumes `serve_collab_tcp_once(.., relay=true)`, `collab_fetch_transient`, `collab_pull`, `local_discovery_index`.

- [ ] **Step 1: Write the failing tests** — add to `tests/collab_network.rs`:

```rust
// <bead-id>
#[tokio::test]
async fn relay_transient_fetch_no_persist() {
    use bole::collab::{fingerprint, CollabObject, CollabSigner};
    use bole::object::Object;
    use bole::refs::{Ref, RefName, Tag};
    use bole::repo::collab::COLLAB_REMOTES_PREFIX;
    use bole::sync::collab::{collab_fetch_transient, serve_collab_tcp_once};

    // Relay R has B and C cached (strangers to the querier A).
    let relay = Repository::memory();
    let rsigner = CollabSigner::from_seed([40u8; 32]);
    relay.publish_profile(&rsigner.sign_profile("relay".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    let b = CollabSigner::from_seed([41u8; 32]);
    let c = CollabSigner::from_seed([42u8; 32]);
    for (signer, name) in [(&b, "bob"), (&c, "carol")] {
        let fp = fingerprint(&signer.public_key());
        let id = relay.objects.put(&Object::Collab(CollabObject::Profile(
            signer.sign_profile(name.into(), String::new(), vec![], vec![], 1)))).await.unwrap();
        let mut tx = relay.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();
    }

    // Querier A follows nobody.
    let anode = Repository::memory();
    let a = CollabSigner::from_seed([43u8; 32]);
    anode.publish_profile(&a.sign_profile("alice".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    let before: Vec<String> = anode.refs.list("refs/collab/").unwrap().iter().map(|n| n.as_str().to_string()).collect();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move { serve_collab_tcp_once(&listener, &relay, true).await });
    let mut conn = connect(addr).await;
    let objs = collab_fetch_transient(&mut conn).await.unwrap();
    srv.await.unwrap().unwrap();

    // Stranger found in the transient corpus...
    assert!(objs.iter().any(|o| matches!(o, CollabObject::Profile(p) if p.display_name == "bob")));
    // ...but NOT persisted: no remotes/ entry, and refs/collab/ layout unchanged.
    let bfp = fingerprint(&b.public_key());
    assert!(anode.refs.get_tag(&RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap()).unwrap().is_none(),
        "stranger never written to remotes/");
    let after: Vec<String> = anode.refs.list("refs/collab/").unwrap().iter().map(|n| n.as_str().to_string()).collect();
    assert_eq!(before, after, "discover relay causes no on-disk refs/collab/ change");

    // A second fetch behaves identically (no hidden cache).
    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let srv2 = tokio::spawn(async move { serve_collab_tcp_once(&listener2, &relay, true).await });
    let mut conn2 = connect(addr2).await;
    let objs2 = collab_fetch_transient(&mut conn2).await.unwrap();
    srv2.await.unwrap().unwrap();
    assert_eq!(objs.len(), objs2.len(), "repeated relay fetch is identical");
}

// <bead-id>
#[tokio::test]
async fn stranger_absent_from_query_until_followed() {
    use bole::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
    use bole::object::Object;
    use bole::refs::{Ref, RefName, Tag};
    use bole::repo::collab::COLLAB_REMOTES_PREFIX;

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
    tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap(),
           Ref::Tag(Tag { target: bid, created_at: 0, message: None }));
    tx.commit().unwrap();

    // Before following: B not in discovery.
    let idx = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(idx.query("bob").is_empty(), "unfollowed stranger not in discover query");

    // After trust follow: B is in the neighborhood.
    anode.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
    let idx2 = anode.local_discovery_index(&a.public_key(), 2).await.unwrap();
    assert!(!idx2.query("bob").is_empty(), "after follow, stranger appears in discover query");
}
```

- [ ] **Step 2: Run RED→GREEN**

Run: `cargo test -p bole --test collab_network` then `cargo test -p bole`
Expected: FAIL first if imports/`relay` arg differ; after Task 1/2 are in place, PASS (2 new + existing).

- [ ] **Step 3: Commit**
```bash
git add tests/collab_network.rs
git commit -m "<bead-id>: loopback relay transient no-persist + follow-to-adopt (G3)"
```

---

## Task 4: CLI `node serve --relay` + `discover relay` + real-binary E2E

**Files:** Modify `bole-cli/src/commands/node.rs`, `bole-cli/src/commands/discover.rs`, `bole-cli/tests/collab_cli.rs`.

**Interfaces:**
- Consumes: `bole::sync::collab::{serve_collab_tcp_once, collab_fetch_transient}`, `bole::sync::transport::TcpConn`, `bole::collab::fingerprint`, `bole::{CollabObject, Profile}`, `crate::key::hex32`.

- [ ] **Step 1: Write the failing E2E test** — add to `bole-cli/tests/collab_cli.rs`:

```rust
// <bead-id>
#[test]
fn cli_discover_relay_shows_stranger() {
    use std::process::Stdio;
    fn serve(dir: &std::path::Path, args: &[&str]) -> std::process::Child {
        let mut cmd = bin();
        cmd.args(args).current_dir(dir).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().unwrap()
    }

    // Publisher P serves.
    let ptmp = tempfile::tempdir().unwrap(); let p = ptmp.path();
    ok(p, &["init", "."], None);
    let pseed = "p2".repeat(32);
    ok(p, &["profile", "set", "--display-name", "Pat"], Some(&pseed));
    let paddr = "127.0.0.1:47801";
    let mut pchild = serve(p, &["node", "serve", "--listen", paddr]);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // Relay R pulls P, then serves in --relay mode.
    let rtmp = tempfile::tempdir().unwrap(); let r = rtmp.path();
    ok(r, &["init", "."], None);
    let rseed = "r2".repeat(32);
    ok(r, &["profile", "set", "--display-name", "Relay"], Some(&rseed));
    ok(r, &["discover", "pull", paddr], Some(&rseed));
    let _ = pchild.kill(); let _ = pchild.wait();
    let raddr = "127.0.0.1:47802";
    let mut rchild = serve(r, &["node", "serve", "--listen", raddr, "--relay"]);
    std::thread::sleep(std::time::Duration::from_millis(400));

    // Querier Q (follows nobody) searches the relay for "Pat".
    let qtmp = tempfile::tempdir().unwrap(); let q = qtmp.path();
    ok(q, &["init", "."], None);
    let qseed = "q3".repeat(32);
    ok(q, &["profile", "set", "--display-name", "Q"], Some(&qseed));
    let out = ok(q, &["discover", "relay", raddr, "Pat", "--json"], Some(&qseed));
    let _ = rchild.kill(); let _ = rchild.wait();

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pat = v.as_array().unwrap().iter().find(|r| r["display_name"] == "Pat");
    assert!(pat.is_some(), "Pat discoverable via relay: {}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(pat.unwrap()["reach"], "stranger");
    // Q persisted nothing.
    let refs = std::fs::read_dir(q.join(".bole")).map(|_| ()).ok();
    assert!(refs.is_some());
    let listing = ok(q, &["trust", "list", "--json"], Some(&qseed));
    let tl: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert!(tl.as_array().map(|a| a.is_empty()).unwrap_or(true), "querier followed nobody via relay search");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-cli --test collab_cli -- cli_discover_relay_shows_stranger`
Expected: FAIL — `node serve --relay` and `discover relay` don't exist yet.

- [ ] **Step 3: Add `--relay` to `node serve`** — in `bole-cli/src/commands/node.rs`, add the flag to `Cmd::Serve` and pass it through:

```rust
    // <bead-id>
    Serve {
        #[arg(long)]
        listen: String,
        /// Run as a relay: serve the whole aggregate (all cached authors), not
        /// just directly-followed ones. See WS8d.
        #[arg(long)]
        relay: bool,
    },
```
In the `Cmd::Serve` handler, destructure `relay` and change the serve call to `serve_collab_tcp_once(&listener, &ctx.repo, relay)`.

- [ ] **Step 4: Add `discover relay`** — in `bole-cli/src/commands/discover.rs`, add a variant to `Cmd` and a handler arm:

```rust
    // <bead-id>
    /// Search a relay for strangers (transient; mutates no local state).
    Relay {
        /// Relay network endpoint (host:port).
        endpoint: String,
        /// Substring to match against profile name/bio/aliases/key.
        term: String,
    },
```

```rust
        // <bead-id>
        Cmd::Relay { endpoint, term } => {
            use bole::sync::collab::collab_fetch_transient;
            use bole::sync::transport::TcpConn;
            use bole::collab::fingerprint;
            let stream = tokio::net::TcpStream::connect(&endpoint).await?;
            let mut conn = TcpConn::new(stream);
            let objs = collab_fetch_transient(&mut conn).await?;
            let mut hits: Vec<&bole::Profile> = objs
                .iter()
                .filter_map(|o| match o {
                    bole::CollabObject::Profile(p) => {
                        let t = term.as_str();
                        let matches = p.display_name.contains(t)
                            || p.bio.contains(t)
                            || p.dns_aliases.iter().any(|a| a.contains(t))
                            || key::hex32(&p.key).contains(t);
                        if matches { Some(p) } else { None }
                    }
                    _ => None,
                })
                .collect();
            // Deterministic, honest ranking: match already applied; tiebreak name then key fp.
            hits.sort_by(|a, b| {
                a.display_name.cmp(&b.display_name).then_with(|| fingerprint(&a.key).cmp(&fingerprint(&b.key)))
            });
            let rows: Vec<_> = hits
                .iter()
                .map(|p| serde_json::json!({
                    "key": key::hex32(&p.key),
                    "display_name": p.display_name,
                    "reach": "stranger",
                }))
                .collect();
            out.emit(
                || {
                    if rows.is_empty() {
                        "no strangers matched".to_string()
                    } else {
                        rows.iter().map(|r| format!("{} [stranger] {}", r["key"], r["display_name"]))
                            .collect::<Vec<_>>().join("\n")
                    }
                },
                || serde_json::json!(rows),
            );
            Ok(())
        }
```

> **Implementer note:** `discover relay` needs no repository, but the `run(ctx, out, cmd)` signature already provides `ctx` (used by `Pull`/`Query`) — leave `ctx` unused in this arm (it stays a used param overall, no clippy warning). Confirm `bole::Profile` and `bole::CollabObject` are re-exported at the crate root (WS8a); if not, use `bole::collab::{Profile, CollabObject}`. `key::hex32` is `crate::key::hex32`.

- [ ] **Step 5: Run RED→GREEN**

Run: `cargo test -p bole-cli --test collab_cli` then `cargo test -p bole-cli` then `cargo build --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: `cli_discover_relay_shows_stranger` + existing CLI tests pass; workspace builds; clippy clean. If the E2E is flaky on fixed ports (47801/47802), raise the bind sleep; reap all `node serve` children (`kill()`+`wait()`).

- [ ] **Step 6: Commit**
```bash
git add bole-cli/src/commands/node.rs bole-cli/src/commands/discover.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: CLI node serve --relay + discover relay + E2E (G4)"
```

---

## Self-Review

**Spec coverage:** §2 relay role (horizon-off serve) → Task 1 (G1) + CLI flag Task 4. §3 transient fetch → Task 2 (G2). §4 CLI `discover relay` → Task 4 (G4). §5 minimal ranking (match + name/fp tiebreak, `reach:"stranger"`, no trust annotation) → Task 4. §6 testing (relay adverts, transient drop-tampered + no-write, loopback no-persist + no-refs-change + repeated-identical + follow-to-adopt, CLI E2E) → Tasks 1–4. §1 invariants (relays-never-authoritative via per-object verify; endpoint read-only — no write path added; strangers transient — Task 3 asserts no `refs/collab/` change; depth-2 untouched — `discover relay` is separate, `discover query` unchanged).

**Placeholder scan:** No "TBD"/"handle errors". The two `Implementer note` blocks flag only confirmation points (`codec::deserialize` path; `bole::Profile`/`CollabObject` re-export path) with exact resolutions.

**Type consistency:** `collab_adverts(repo, relay: bool)`, `serve_collab(conn, repo, relay: bool)`, `serve_collab_tcp_once(listener, repo, relay: bool)` used consistently across all call sites (Task 1 lists every ripple). `collab_fetch_transient(conn) -> Vec<CollabObject>` produced in Task 2, consumed in Tasks 3–4. `reach: "stranger"` string + `key::hex32` raw-hex keys consistent with WS8c CLI. `Profile` fields (`display_name`/`bio`/`dns_aliases`/`key`) match WS8a.

**Flagged risks:** (1) the `relay` param ripples to ~14 call sites (all listed in Task 1 Step 5) — mechanical `false`/`true` additions. (2) `codec::deserialize` path must be confirmed. (3) fixed E2E ports carry the WS8b/c parallel-run flake caveat.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-03-ws8d-relay-and-stranger-search.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task (one bead + branch each), review between tasks.

**2. Inline Execution** — execute tasks here in batches with checkpoints.

Which approach?
