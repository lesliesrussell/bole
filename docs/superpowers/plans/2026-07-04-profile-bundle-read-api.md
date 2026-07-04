# Profile-Bundle Read API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One transport-agnostic aggregated read — `Repository::profile_bundle(key)` + `bole profile bundle` — that returns a dev's verified identity, own trust out-edges, and (when local) the repo's timelines as a single stable JSON bundle.

**Architecture:** A read-only `Repository` method returns a typed `ProfileBundle` (no JSON dependency in the library). The CLI adds a `profile bundle` subcommand that renders the typed struct to the stable JSON contract with raw-hex keys. Every emitted profile/edge is verified fail-closed. First "bole as backend API" surface for the Grove frontend (separate repo, later).

**Tech Stack:** Rust, tokio, `serde_json` (CLI only), real-binary CLI tests.

## Global Constraints

- **Bead:** all work is under `bole-k93a` (the controller may split per-gate sub-beads at execution, but the branch(es) match the bead ID(s) exactly). Each contiguous added block gets a `// <bead-id>` comment (ID only).
- **Read-only.** `profile_bundle` takes `&self` and performs only reads; it persists nothing.
- **Fail-closed verification.** Every emitted `Profile`/`TrustEdge` is verified (`verify_profile`/`verify_edge`); anything failing is dropped (profile → `None`, edge omitted). `public_profiles`/`public_edges` do NOT verify on read — the bundle must.
- **Transport-agnostic.** The library returns typed data with **no `serde_json`**; the CLI owns hex-rendering + `--json` (mirrors `discover`/`trust`).
- **Out-edges only.** Trust slice = edges where `from_key == key`. An edge where `to_key == key` (someone else trusting the key) is excluded in v1.
- **Timelines local-only.** Included only when `is_local`; `[]` otherwise.
- **Keys raw hex** everywhere in output (`key::hex32`); seeds from env/file (`--key-env`/`--key-file`), never argv.
- **Local depth-2 / discovery untouched.** No change to `discover`/`follow_*`/relay code.
- **JSON null/empty conventions (exact):** `key` + `is_local` always present; `profile` is `null` (never omitted) when absent; `trust.edges` and `timelines` are `[]` (never `null`) when empty.

---

## File Structure

- **Modify** `src/repo/collab.rs` — add `TimelineView`, `ProfileBundle`, and `Repository::profile_bundle` (this file already holds the collab read API: `public_profiles`/`public_edges`/`tracked_collab`/`profile`).
- **Modify** `src/lib.rs` — re-export `ProfileBundle`, `TimelineView`.
- **Modify** `bole-cli/src/commands/profile.rs` — add the `Bundle` subcommand + rendering.
- **Modify** `bole-cli/tests/collab_cli.rs` — CLI E2E.

Gate order: G1 (library op + types + unit tests) → G2 (CLI subcommand + JSON contract + E2E).

---

## Gate 1 (bead: profile_bundle library) — `Repository::profile_bundle` + types

**Files:**
- Modify: `src/repo/collab.rs`, `src/lib.rs`
- Test: unit tests in `src/repo/collab.rs`

**Interfaces:**
- Consumes: `public_profiles()`, `public_edges()`, `tracked_collab()` (all in this file), `verify_profile`/`verify_edge` (`crate::collab`), `CollabObject`, `Profile`, `TrustEdge`, `Key`, `self.refs`/`self.objects` (pub fields), `crate::refs::Ref::Timeline`, `crate::refs::Timeline{head: ObjectId, …}`, `crate::object::{Object::Snapshot, ObjectId}`, `Snapshot{author: String, created_at: u64}`.
- Produces:
  - `pub struct TimelineView { pub name: String, pub head: ObjectId, pub author: String, pub created_at: u64 }` (derive `Debug, Clone, PartialEq, Eq`)
  - `pub struct ProfileBundle { pub key: Key, pub is_local: bool, pub profile: Option<Profile>, pub edges: Vec<TrustEdge>, pub timelines: Vec<TimelineView> }` (derive `Debug, Clone`)
  - `Repository::profile_bundle(&self, key: &Key) -> Result<ProfileBundle>`

- [ ] **Step 1: Write the failing tests** (in `src/repo/collab.rs` test module — reuse the `Repository::memory()` construction the existing tests in this file use, e.g. `relay_pin_crud_and_upsert`).

```rust
#[tokio::test]
async fn bundle_own_identity_full() {
    let repo = Repository::memory();
    let me = CollabSigner::from_seed([1u8; 32]);
    let x = CollabSigner::from_seed([2u8; 32]);
    // Publish own profile + an own out-edge (me -> x, follow).
    repo.publish_profile(&me.sign_profile("Me".into(), "hi".into(), vec![], vec![], 1)).await.unwrap();
    repo.publish_edge(&me.sign_edge(x.public_key(), TrustKind::Follow, Some("ex".into()), 1)).await.unwrap();
    // Create a timeline with a snapshot head.
    let snap_id = /* put a Snapshot object and create_timeline "main" -> it; see note */;
    let b = repo.profile_bundle(&me.public_key()).await.unwrap();
    assert!(b.is_local);
    assert_eq!(b.profile.as_ref().unwrap().display_name, "Me");
    assert_eq!(b.edges.len(), 1);
    assert_eq!(b.edges[0].to_key, x.public_key());
    assert_eq!(b.timelines.len(), 1);
    assert_eq!(b.timelines[0].name, "main");
    assert_eq!(b.timelines[0].head, snap_id);
}

#[tokio::test]
async fn bundle_peer_from_cache_no_timelines() {
    let repo = Repository::memory();
    let me = CollabSigner::from_seed([3u8; 32]);
    let peer = CollabSigner::from_seed([4u8; 32]);
    let y = CollabSigner::from_seed([5u8; 32]);
    // Track the peer's profile + an out-edge into the cache (remotes/), verified.
    /* cache peer.sign_profile("Peer",..,1) and peer.sign_edge(y, Follow, None, 1)
       using the same tracked-peer-caching helper the collab tests already use */
    let b = repo.profile_bundle(&peer.public_key()).await.unwrap();
    assert!(!b.is_local);
    assert_eq!(b.profile.as_ref().unwrap().display_name, "Peer");
    assert_eq!(b.edges.len(), 1);
    assert!(b.timelines.is_empty(), "peers get no timelines");
}

#[tokio::test]
async fn bundle_unknown_key_is_empty() {
    let repo = Repository::memory();
    let ghost = CollabSigner::from_seed([6u8; 32]);
    let b = repo.profile_bundle(&ghost.public_key()).await.unwrap();
    assert!(!b.is_local);
    assert!(b.profile.is_none());
    assert!(b.edges.is_empty());
    assert!(b.timelines.is_empty());
}

#[tokio::test]
async fn bundle_out_edges_only() {
    let repo = Repository::memory();
    let me = CollabSigner::from_seed([7u8; 32]);
    let other = CollabSigner::from_seed([8u8; 32]);
    repo.publish_profile(&me.sign_profile("Me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    // An edge OTHER -> me (to_key == me) must NOT appear in me's bundle.
    // Publish it as an own public edge signed by `other`? Own public_edges are
    // the repo's own; to test exclusion, cache an edge other->me and query me:
    /* cache other.sign_edge(me.public_key(), Follow, None, 1) into remotes/ */
    let b = repo.profile_bundle(&me.public_key()).await.unwrap();
    assert!(b.edges.iter().all(|e| e.from_key == me.public_key()), "only out-edges");
    // me authored none, so edges is empty here.
    assert!(b.edges.is_empty());
}
```

> **Notes for the implementer (resolve by reading this file's existing tests):**
> - Confirm the exact method names for publishing an own edge (`publish_edge`?) and for caching a tracked peer object into `remotes/` — grep `src/repo/collab.rs` for how its tests set up `public_edges` and `tracked_collab` fixtures, and reuse those helpers verbatim. Do NOT invent new persistence.
> - To create a timeline with a real snapshot head: build a `Snapshot { root, parents: vec![], author: "…".into(), created_at: <n> }`, `repo.objects.put(&Object::Snapshot(snap)).await?` → `snap_id`, then `repo.refs.create_timeline(RefName::new("main")?, snap_id, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)?`. Mirror how `src/repo/mod.rs` tests build snapshots + timelines (search `create_timeline` there).
> - For the fail-closed test below, tamper by mutating a signed object's field after signing (e.g. `p.seq = 999` or `e.seq = 999`) so `verify_*` returns false — the same technique the WS8e/collab tests use.

- [ ] **Step 2: Add the fail-closed test**

```rust
#[tokio::test]
async fn bundle_drops_unverifiable() {
    let repo = Repository::memory();
    let peer = CollabSigner::from_seed([9u8; 32]);
    let y = CollabSigner::from_seed([10u8; 32]);
    // Cache a TAMPERED peer profile (sig no longer valid) + a tampered out-edge.
    let mut bad_p = peer.sign_profile("Peer".into(), String::new(), vec![], vec![], 1);
    bad_p.seq = 999; // breaks the signature
    let mut bad_e = peer.sign_edge(y.public_key(), TrustKind::Follow, None, 1);
    bad_e.seq = 999;
    /* cache bad_p and bad_e into remotes/ (bypassing any verify-on-write helper if
       one exists — write the raw objects + refs the way the store is populated) */
    let b = repo.profile_bundle(&peer.public_key()).await.unwrap();
    assert!(b.profile.is_none(), "tampered profile dropped -> None");
    assert!(b.edges.is_empty(), "tampered edge dropped");
}
```

> If `tracked_collab()` itself already drops unverifiable objects (it re-verifies on read), this test still holds — the tampered objects never reach the bundle. If the cache-write helper verifies-on-write and refuses the tampered object, write the raw object+ref directly (grep how `tracked_collab`'s own test injects a tampered object — WS8d's `transient_fetch_drops_tampered` shows the tamper technique) so the bundle's own verify is what's under test.

- [ ] **Step 3: Run the tests, verify they fail** — `cargo test -p bole --lib repo::collab::tests::bundle` → FAIL (types/method absent).

- [ ] **Step 4: Add the types** near the top of `src/repo/collab.rs` (after the imports; add `use crate::object::ObjectId;` if not present):

```rust
// bole-k93a
/// The head-snapshot summary of one timeline in this repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineView {
    pub name: String,
    pub head: ObjectId,
    pub author: String,
    pub created_at: u64,
}

// bole-k93a
/// The locally-verifiable hub view of a developer key: identity + own trust
/// out-edges (+ this repo's timelines when `key` is the repo's own identity).
#[derive(Debug, Clone)]
pub struct ProfileBundle {
    pub key: Key,
    pub is_local: bool,
    pub profile: Option<Profile>,
    pub edges: Vec<TrustEdge>,
    pub timelines: Vec<TimelineView>,
}
```

- [ ] **Step 5: Implement `profile_bundle`** in the `impl Repository` block in `src/repo/collab.rs` (near `tracked_collab`):

```rust
// bole-k93a
/// Aggregate the locally-verifiable hub view of `key`. Read-only; every emitted
/// profile and edge is verified fail-closed (dropped if it does not verify).
pub async fn profile_bundle(&self, key: &Key) -> Result<ProfileBundle> {
    use crate::collab::{verify_edge, verify_profile};

    // is_local: this repo publishes a PUBLIC profile for `key`.
    let publics = self.public_profiles().await?;
    let is_local = publics.iter().any(|p| &p.key == key);

    // profile: own published (if local) else tracked peer; verified fail-closed.
    let profile = if is_local {
        publics.into_iter().find(|p| &p.key == key)
    } else {
        self.tracked_collab().await?.into_iter().find_map(|o| match o {
            CollabObject::Profile(p) if &p.key == key => Some(p),
            _ => None,
        })
    }
    .filter(|p| verify_profile(p));

    // edges: out-edges (from_key == key), verified fail-closed.
    let mut edges = Vec::new();
    if is_local {
        for e in self.public_edges().await? {
            if &e.from_key == key && verify_edge(&e) {
                edges.push(e);
            }
        }
    } else {
        for o in self.tracked_collab().await? {
            if let CollabObject::TrustEdge(e) = o {
                if &e.from_key == key && verify_edge(&e) {
                    edges.push(e);
                }
            }
        }
    }

    // timelines: this repo's timelines when local, else empty.
    let mut timelines = Vec::new();
    if is_local {
        for name in self.refs.list("")? {
            if let Some(crate::refs::Ref::Timeline(t)) = self.refs.get(&name)? {
                if let Some(crate::object::Object::Snapshot(s)) =
                    self.objects.get(&t.head).await?
                {
                    timelines.push(TimelineView {
                        name: name.as_str().to_string(),
                        head: t.head,
                        author: s.author,
                        created_at: s.created_at,
                    });
                }
            }
        }
    }

    Ok(ProfileBundle { key: *key, is_local, profile, edges, timelines })
}
```

> `self.refs.list("")` + `self.refs.get(&name)` matching `Ref::Timeline` is exactly the enumeration `bole-cli/src/commands/timeline.rs::list` uses. `Key` is `[u8; 32]` (`Copy`), so `*key` is fine. If `verify_profile`/`verify_edge` are re-exported at `crate::collab`, the `use` line resolves; otherwise use the path the existing collab methods use.

- [ ] **Step 6: Run the tests, verify they pass** — `cargo test -p bole --lib repo::collab::tests::bundle` → PASS (5 tests).

- [ ] **Step 7: Re-export + commit** — add `ProfileBundle, TimelineView` to the `src/lib.rs` re-export (next to the other `repo`/collab types).

```bash
cargo test -p bole --lib repo::collab
cargo clippy --workspace
git add src/repo/collab.rs src/lib.rs
git commit -m "<bead-id>: Repository::profile_bundle — verified identity + out-edges + local timelines"
```

---

## Gate 2 (bead: profile bundle CLI) — `bole profile bundle` + JSON contract

**Files:**
- Modify: `bole-cli/src/commands/profile.rs`
- Test: `bole-cli/tests/collab_cli.rs`

**Interfaces:**
- Consumes: `bole::{ProfileBundle, TimelineView}` (G1), `Repository::profile_bundle` via `ctx.repo`, `key::{hex32, parse_hex_32}`, `signer_from`, `bole::TrustKind`, `Output::emit`.
- Produces: `Cmd::Bundle { key: Option<String>, key_env: String, key_file: Option<PathBuf> }` + rendering.

- [ ] **Step 1: Write the failing E2E** in `bole-cli/tests/collab_cli.rs` (mirror the process/`ok`/`run` helpers the existing collab_cli tests use; pick a temp repo dir + valid-hex seed as they do):

```rust
// <bead-id>
#[test]
fn cli_profile_bundle_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "b7".repeat(32);
    // Own identity: set profile, create a timeline, follow a peer.
    ok(w, &["profile", "set", "--display-name", "Me", "--bio", "hi"], Some(&seed));
    // (create a timeline via the timeline CLI; follow an arbitrary peer key)
    let peer = "c1".repeat(32);
    ok(w, &["trust", "follow", &peer], Some(&seed));
    let out = ok(w, &["profile", "bundle", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["is_local"], true);
    assert_eq!(v["profile"]["display_name"], "Me");
    assert!(v["trust"]["edges"].as_array().unwrap().iter().any(|e| e["to"] == peer && e["kind"] == "follow"));
    assert!(v["timelines"].is_array());
    // Unknown key: null/empty shape.
    let ghost = "d2".repeat(32);
    let out2 = ok(w, &["profile", "bundle", &ghost, "--json"], Some(&seed));
    let v2: serde_json::Value = serde_json::from_slice(&out2.stdout).unwrap();
    assert_eq!(v2["is_local"], false);
    assert_eq!(v2["profile"], serde_json::Value::Null);
    assert!(v2["trust"]["edges"].as_array().unwrap().is_empty());
    assert!(v2["timelines"].as_array().unwrap().is_empty());
}
```

> Match the exact `profile set` flags to `Cmd::Set`'s clap fields (grep `profile.rs` — likely `--display-name`/`--bio`/`--endpoints`). For creating a timeline, use the existing `timeline create` command with whatever args it requires (grep `bole-cli/src/commands/timeline.rs`'s `Cmd::Create`). If a follow needs the peer discoverable first, follow the pattern the existing `trust`/`discover` E2Es use. The peer-shape assertion (`is_local:false`, `timelines:[]`) can be a third sub-case if you pull a peer profile into cache as the other collab_cli tests do; otherwise the unknown-key case above already exercises the null/empty contract.

- [ ] **Step 2: Run it, verify it fails** — `cargo test -p bole-cli --test collab_cli cli_profile_bundle_contract` → FAIL (`bundle` subcommand absent).

- [ ] **Step 3: Add the `Bundle` subcommand** to the `Cmd` enum in `bole-cli/src/commands/profile.rs` (after `Show`):

```rust
    // <bead-id>
    /// Aggregated hub bundle for a dev: profile + own trust out-edges + (own) timelines.
    Bundle {
        /// 64-hex public key to bundle (omit for own key).
        key: Option<String>,
        #[arg(long, default_value = "BOLE_COLLAB_KEY")]
        key_env: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
```

- [ ] **Step 4: Add the handler arm** in `run(...)`:

```rust
        // <bead-id>
        Cmd::Bundle { key, key_env, key_file } => {
            let k = match key {
                Some(h) => key::parse_hex_32(&h)?,
                None => signer_from(&key_env, key_file.as_deref())?.public_key(),
            };
            let b = ctx.repo.profile_bundle(&k).await?;
            let profile_json = match &b.profile {
                Some(p) => serde_json::json!({
                    "key": key::hex32(&p.key),
                    "display_name": p.display_name,
                    "bio": p.bio,
                    "endpoints": p.endpoints,
                    "dns_aliases": p.dns_aliases,
                    "seq": p.seq,
                }),
                None => serde_json::Value::Null,
            };
            let edges_json: Vec<_> = b.edges.iter().map(|e| serde_json::json!({
                "to": key::hex32(&e.to_key),
                "kind": match e.kind {
                    bole::TrustKind::Follow => "follow",
                    bole::TrustKind::Vouch => "vouch",
                    bole::TrustKind::Review => "review",
                },
                "petname": e.petname,
                "seq": e.seq,
            })).collect();
            let timelines_json: Vec<_> = b.timelines.iter().map(|t| serde_json::json!({
                "name": t.name,
                "head": t.head.to_string(),
                "author": t.author,
                "created_at": t.created_at,
            })).collect();
            let bundle_key = key::hex32(&b.key);
            let is_local = b.is_local;
            out.emit(
                || format!(
                    "{} [{}] {} edges, {} timelines",
                    bundle_key,
                    if is_local { "local" } else { "peer" },
                    edges_json.len(),
                    timelines_json.len(),
                ),
                || serde_json::json!({
                    "key": bundle_key,
                    "is_local": is_local,
                    "profile": profile_json,
                    "trust": { "edges": edges_json },
                    "timelines": timelines_json,
                }),
            );
            Ok(())
        }
```

> `e.petname` is `Option<String>` → serializes to a JSON string or `null` automatically. `t.head.to_string()` is the ObjectId hex (`ObjectId: Display`). Keys via `key::hex32`. Confirm `PathBuf` is imported at the top of `profile.rs` (it is — `Cmd::Show` uses it).

- [ ] **Step 5: Run it, verify it passes** — `cargo test -p bole-cli --test collab_cli cli_profile_bundle_contract` → PASS.

- [ ] **Step 6: Full build + suite + clippy + commit**

```bash
cargo build --workspace
cargo test -p bole-cli
cargo clippy --workspace
git add bole-cli/src/commands/profile.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: bole profile bundle — JSON contract for the Grove backend"
```

---

## Self-Review

**Spec coverage:**
- §1 scope (by-key layered availability, read-only, fail-closed, transport-agnostic, raw hex) → G1 `profile_bundle` + Global Constraints. ✅
- §2 identity resolution + `is_local` (public profile present; own-vs-peer-vs-none; verify) → G1 Step 5. ✅
- §3 trust slice (out-edges `from_key==key`, verified, local vs cache) → G1 Step 5. ✅
- §4 timelines (local-only, head snapshot summary) → G1 Step 5 (refs enumeration + Snapshot load). ✅
- §5 library API + CLI + JSON contract (typed struct; `profile bundle`; exact shape + null/empty) → G1 types + G2 handler. ✅
- §6 tests (own / peer / unknown / fail-closed / out-edges-only unit; CLI contract E2E) → G1 Steps 1-2 + G2 Step 1. ✅
- Invariants (read-only, fail-closed, transport-agnostic, out-edges-only, timelines-local, raw hex, seeds env/file, depth-2 untouched) → Global Constraints + carried. ✅

**Placeholder scan:** the G1 test bodies contain `/* … */` notes ONLY for fixture setup (publishing an own edge, caching a tracked peer, building a snapshot/timeline) because those must reuse the exact persistence helpers already in `src/repo/collab.rs` / `src/repo/mod.rs` tests — the assertions and the code under test are fully specified. Every implementation step carries complete code.

**Type consistency:** `ProfileBundle{key: Key, is_local: bool, profile: Option<Profile>, edges: Vec<TrustEdge>, timelines: Vec<TimelineView>}`, `TimelineView{name, head: ObjectId, author, created_at}`, `profile_bundle(&self, key: &Key) -> Result<ProfileBundle>`, CLI `Cmd::Bundle{key: Option<String>, key_env, key_file}` — consistent across gates and matching live types (`Profile`, `TrustEdge`, `TrustKind`, `Timeline.head`, `Snapshot.author/created_at`, `key::hex32`).

**Open verification items for implementers (named in-step):** the own-edge publish method name and the tracked-peer cache helper in `src/repo/collab.rs` tests (G1 S1); the snapshot/timeline build pattern in `src/repo/mod.rs` tests (G1 S1); the tamper-injection technique for the fail-closed test (G1 S2); the exact `profile set` / `timeline create` clap flags (G2 S1).
