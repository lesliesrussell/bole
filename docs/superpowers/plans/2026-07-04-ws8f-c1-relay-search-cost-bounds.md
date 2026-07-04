# WS8f-c1 — Relay-Side Search Query-Cost Bounds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cap the two attacker-controlled amplifiers in the WS8f-b `Search` verb — clamp `max_hops` to a ceiling and reject a too-short term before any work — so relay cost stays ~O(corpus) instead of O(corpus²).

**Architecture:** Two fixed public constants live beside `search_ball`. The relay enforces them as serve-side policy in `serve_collab`'s existing `Search` arm (reject too-short term fail-fast; clamp `max_hops` before `search_ball`). The pure `search_ball` and the wire `Message::Search` are untouched. The client maps a relay `Error` to `Err` (existing skip-and-continue), and the CLI rejects a too-short term locally before connecting.

**Tech Stack:** Rust, tokio, loopback `TcpConn` + real-binary CLI tests, `anyhow` (CLI).

## Global Constraints

- **ZERO code without a bead.** Each Gate is one bead; branch = bead ID exactly; each contiguous added block gets a `// <bead-id>` comment (ID only, one per contiguous block). Use `bd` for tracking.
- **`MAX_SEARCH_HOPS: u8 = 6`** — the relay clamps any incoming `max_hops` down to this (`max_hops.min(MAX_SEARCH_HOPS)`). Six hops is the maximum we consider meaningful for a trust path.
- **`MIN_SEARCH_TERM_LEN: usize = 3`** — a `Search` whose `term.len() < MIN_SEARCH_TERM_LEN` (bytes) is rejected. Three characters is the minimum we consider meaningful for a search term.
- **Relays never authoritative over objects.** A bound only *withholds or limits work* (reject a request, clamp a depth). It never forges, mutates, re-attributes, or changes *which signed objects are trusted*. The client still verifies every returned object fail-closed.
- **Bounds degrade availability, never soundness.** A rejected or clamped Search reduces what a relay returns; correctness is never affected.
- **Endpoint read-only; no new wire verbs.** `Message::Search` keeps its exact shape. `search_ball` is unchanged.
- **Fail-fast reject.** A too-short term must be rejected BEFORE the corpus load (`for a in &refs { objects.get }`) and before any BFS — zero corpus work.
- **Local depth-2 query + whole-aggregate HaveWant path untouched.** `discover query`/`follow_*` unchanged; the `HaveWant` path carries no `term`/`max_hops` and is not bounded here.
- **Transient query, no local mutation. Keys raw hex. No new CLI flag or command** — the CLI change is stricter input validation only.

---

## File Structure

- **Modify** `src/collab/search.rs` — add `pub const MAX_SEARCH_HOPS` + `pub const MIN_SEARCH_TERM_LEN` beside `search_ball`.
- **Modify** `src/collab/mod.rs` — re-export the two constants alongside `search_ball`.
- **Modify** `src/lib.rs` — re-export the two constants at crate root.
- **Modify** `src/sync/collab.rs` — enforce the bounds in `serve_collab`'s `Search` arm; add an explicit `Message::Error` arm in `search_or_fallback`.
- **Modify** `tests/collab_network.rs` — serve-arm + loopback tests.
- **Modify** `bole-cli/src/commands/discover.rs` — CLI local pre-check at the `Cmd::Relay` handler top.
- **Modify** `bole-cli/tests/collab_cli.rs` — CLI E2E.

Gate order: G1 (constants + relay enforcement + client Error arm + library tests) → G2 (CLI pre-check + E2E). Two beads.

---

## Gate 1 (bead: relay-side bounds) — constants + serve enforcement + client Error mapping

**Files:**
- Modify: `src/collab/search.rs`, `src/collab/mod.rs`, `src/lib.rs`, `src/sync/collab.rs`
- Test: `tests/collab_network.rs`

**Interfaces:**
- Consumes: `search_ball` (WS8f-b), `serve_collab` Search arm (WS8f-b), `Message::{Search, Error, Pack, Done}`, `search_or_fallback` (WS8f-b), `Error::Storage`.
- Produces:
  - `pub const MAX_SEARCH_HOPS: u8 = 6;`
  - `pub const MIN_SEARCH_TERM_LEN: usize = 3;`
  - serve `Search` arm: reject `term.len() < MIN_SEARCH_TERM_LEN` with `Message::Error("search term too short")` before any corpus work; clamp `max_hops` to `MAX_SEARCH_HOPS` before `search_ball`.
  - `search_or_fallback` maps a `Message::Error(e)` reply to `Err(Error::Storage(e))`.

- [ ] **Step 1: Add the constants** in `src/collab/search.rs` (near the top, beside `search_ball`):

```rust
// <bead-id>
/// Six hops is the maximum we consider meaningful for a trust path; a relay
/// clamps any larger `max_hops` in a Search request down to this.
pub const MAX_SEARCH_HOPS: u8 = 6;

// <bead-id>
/// Three characters is the minimum we consider meaningful for a search term.
/// Shorter terms match (nearly) every profile, so a relay rejects them before
/// doing any corpus work.
pub const MIN_SEARCH_TERM_LEN: usize = 3;
```

- [ ] **Step 2: Re-export the constants** — in `src/collab/mod.rs`, extend the search re-export:

```rust
// <bead-id>
pub use search::{search_ball, MAX_SEARCH_HOPS, MIN_SEARCH_TERM_LEN};
```

And in `src/lib.rs`, add `MAX_SEARCH_HOPS, MIN_SEARCH_TERM_LEN` to the collab re-export line (next to `search_ball`).

- [ ] **Step 3: Write the failing serve tests** in `tests/collab_network.rs`. Reuse the harness of the existing WS8f-b test `relay_search_returns_only_matches_and_ball` (find it and copy its relay-repo setup + client-drive shape). Add:

```rust
// <bead-id>
// A Search with max_hops far above the ceiling is CLAMPED to MAX_SEARCH_HOPS:
// a chain longer than 6 hops to a match returns exactly the depth-6 ball, not
// the depth-255 ball (the deepest edge is absent).
#[tokio::test]
async fn relay_search_clamps_max_hops() {
    // Relay corpus: a 7-edge Follow chain n0->n1->...->n7, where n7's profile
    // matches the term "target". Serve relay=true with a signer.
    // Client: Hello{caps: CAP_SEARCH} -> Welcome (assert caps.contains) ->
    //   Search{term:"target", max_hops: 255} -> Pack -> Done.
    // Decode the pack. Assert the returned edge set equals what
    // bole::search_ball(&corpus, "target", bole::MAX_SEARCH_HOPS) returns —
    // specifically: the edge n0->n1 (reverse depth 7) is ABSENT, and the six
    // nearer edges (n1->n2 ... n6->n7) are PRESENT. This proves the clamp:
    // an unclamped 255 search would have included n0->n1.
}

// <bead-id>
// A Search with a term shorter than MIN_SEARCH_TERM_LEN is rejected with an
// Error and does ZERO corpus work (the client receives Error, never a Pack).
#[tokio::test]
async fn relay_search_rejects_short_term() {
    // Relay corpus: any matching profile "Pat" reachable. Serve relay=true.
    // Client: Hello{caps: CAP_SEARCH} -> Welcome -> Search{term:"ab", max_hops:4}.
    // Assert the next message is Message::Error (containing "too short"), and
    // that NO Message::Pack is ever received on this connection.
}
```

> Build the 7-edge chain and the matching profile exactly as the sibling test builds its relay corpus (sign edges with `CollabSigner::from_seed`, `sign_edge(to, TrustKind::Follow, None, 1)`, cache them into the relay's `remotes/` the same way). Drive the raw client exchange with the same `TcpConn` + `Message` sends/receives the sibling uses; decode the pack via the same helper (`bole::store::pack::decode_pack` or the file's local helper). `bole::MAX_SEARCH_HOPS` / `bole::MIN_SEARCH_TERM_LEN` are the re-exported constants from Steps 1-2.

- [ ] **Step 4: Run them, verify they fail** — `cargo test -p bole --test collab_network relay_search_clamps relay_search_rejects` → FAIL (clamp not applied; short term currently served, not rejected).

- [ ] **Step 5: Enforce the bounds in the serve `Search` arm** (`src/sync/collab.rs`). The current arm is:

```rust
        Message::Search { term, max_hops } => {
            let mut corpus = Vec::new();
            for a in &refs {
                if let Some(Object::Collab(o)) = repo.objects.get(&a.target).await? {
                    corpus.push(o);
                }
            }
            let selected = crate::collab::search_ball(&corpus, &term, max_hops);
            // ... map to ids, build_pack, Pack, Done ...
        }
```

Change it to reject-then-clamp at the top of the arm, before any corpus work:

```rust
        Message::Search { term, max_hops } => {
            // <bead-id>
            // Fail-fast: a too-short term matches (nearly) everything; reject it
            // before loading any corpus or computing any ball.
            if term.len() < crate::collab::MIN_SEARCH_TERM_LEN {
                conn.send(&Message::Error("search term too short".into())).await?;
                return Ok(());
            }
            // <bead-id>
            // Clamp the search depth to the relay's ceiling.
            let max_hops = max_hops.min(crate::collab::MAX_SEARCH_HOPS);
            let mut corpus = Vec::new();
            for a in &refs {
                if let Some(Object::Collab(o)) = repo.objects.get(&a.target).await? {
                    corpus.push(o);
                }
            }
            let selected = crate::collab::search_ball(&corpus, &term, max_hops);
            // ... existing map-to-ids / build_pack / Pack / Done, unchanged ...
        }
```

> Keep the existing id-mapping/`build_pack`/`Pack`/`Done` tail exactly as it was; only prepend the reject + clamp. `return Ok(())` on reject ends the serve cleanly (the connection closes after; the client already treats a non-`Pack` reply as an error). Do NOT modify `search_ball` or `Message::Search`.

- [ ] **Step 6: Add the explicit client `Error` arm** in `search_or_fallback` (`src/sync/collab.rs`) so a relay's rejection surfaces its message. Change the pack-recv match:

```rust
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        // <bead-id>
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("collab: expected Pack".into())),
    };
```

> This mirrors how the Welcome step already handles `Message::Error(e) => return Err(Error::Storage(e))`. It changes only the error *message* (a `_`-matched Error already returned `Err`), so `query_relay_set`'s skip-and-continue behavior is unchanged — an erroring relay is still dropped, now with a clearer reason.

- [ ] **Step 7: Run the serve tests, verify pass** — `cargo test -p bole --test collab_network relay_search_clamps relay_search_rejects` → PASS.

- [ ] **Step 8: Write + run the loopback client tests** (`tests/collab_network.rs`):

```rust
// <bead-id>
// The client's collab_search surfaces a relay's too-short-term rejection as Err.
#[tokio::test]
async fn client_search_short_term_is_err() {
    // CAP_SEARCH relay with any corpus; client calls bole::collab_search(conn, "ab", 4).
    // Assert it returns Err.
}

// <bead-id>
// A max_hops above the ceiling still returns the correct, clamped result: the
// stranger is found and its trust_path computed over the depth-6-bounded ball.
#[tokio::test]
async fn client_search_above_ceiling_returns_clamped_result() {
    // CAP_SEARCH relay caching a chain to stranger "Pat" within 6 hops of the
    // querier's frontier; client calls collab_search(conn, "Pat", 255), feeds
    // the result + own_edges to rank_strangers_multi; assert Pat is found with
    // Some(trust_path). (The clamp does not lose paths that fit within 6 hops.)
}

// <bead-id>
// query_relay_set stays robust: a valid term over two relays returns merged
// hits; a too-short term makes every relay reject, and the set query returns
// empty (all skipped, no crash) rather than erroring out.
#[tokio::test]
async fn query_relay_set_handles_short_term_via_skip() {
    // Two CAP_SEARCH relays each serving stranger "Pat". Pin both.
    // valid: query_relay_set(self, own, relays, "Pat", 4) -> Pat present.
    // short: query_relay_set(self, own, relays, "ab", 4) -> empty Vec (both
    //        relays reject "ab"; skip-and-continue yields no hits, no panic).
}
```

> Reuse the WS8f-b loopback helpers (`collab_search`, `query_relay_set`, `rank_strangers_multi`, the relay-serve listener setup with `Some(&signer)`, own-edge construction from the `multi_relay_*` tests). Run `cargo test -p bole --test collab_network` → PASS. `cargo clippy --workspace` clean.

- [ ] **Step 9: Commit**

```bash
cargo test -p bole --test collab_network
cargo test -p bole --lib collab::search sync::
cargo clippy --workspace
git add src/collab/search.rs src/collab/mod.rs src/lib.rs src/sync/collab.rs tests/collab_network.rs
git commit -m "<bead-id>: relay-side Search cost bounds — clamp max_hops, reject short term"
```

---

## Gate 2 (bead: CLI pre-check) — local too-short-term rejection + E2E

**Files:**
- Modify: `bole-cli/src/commands/discover.rs`
- Test: `bole-cli/tests/collab_cli.rs`

**Interfaces:**
- Consumes: `bole::MIN_SEARCH_TERM_LEN` (Gate 1), the `Cmd::Relay` handler.
- Produces: `discover relay <term>` rejects `term.len() < MIN_SEARCH_TERM_LEN` locally, before connecting, with a clear error — for BOTH the ad-hoc `--endpoint` and the pinned-set paths.

- [ ] **Step 1: Write the failing E2E** in `bole-cli/tests/collab_cli.rs` (mirror `cli_discover_relay_search_transparent`'s structure for the positive case):

```rust
// <bead-id>
#[test]
fn cli_discover_relay_rejects_short_term() {
    // No relay needed — the check is local and must fire BEFORE any connection.
    // Run `discover relay "ab" --json` (2 chars) with a valid key env; assert the
    // command FAILS (non-zero exit) and stderr/err output contains "at least 3
    // characters". Use a bogus/unused --endpoint or none — the point is it must
    // not require a reachable relay because it never connects.
    // Then assert a valid `discover relay "Pat" ...` against a real relay still
    // returns the stranger (reuse cli_discover_relay_search_transparent's setup
    // for the positive half, OR keep this test negative-only and rely on the
    // existing transparent test for the positive path).
}
```

> Use the crate's existing failing-command helper (grep `collab_cli.rs` for how other tests assert a non-zero exit / error output — e.g. a `run(...)`/`fail(...)` helper rather than `ok(...)`). If only `ok(...)` exists, invoke the binary directly with `assert_cmd`/`Command` the way the file already spawns processes and assert `!status.success()` and the stderr substring. Keep the positive path covered by the existing `cli_discover_relay_search_transparent` test — this new test is the negative case.

- [ ] **Step 2: Run it, verify it fails** — `cargo test -p bole-cli --test collab_cli cli_discover_relay_rejects_short_term` → FAIL (short term currently connects and returns "no strangers matched" instead of erroring locally).

- [ ] **Step 3: Add the CLI pre-check** at the very top of the `Cmd::Relay` handler in `bole-cli/src/commands/discover.rs`, before `signer_from`/any connection:

```rust
        Cmd::Relay { term, endpoint, max_hops, key_env, key_file } => {
            // <bead-id>
            // Reject a too-short term locally, before connecting: the relay would
            // reject it anyway, and an empty/short search matches everything.
            if term.len() < bole::MIN_SEARCH_TERM_LEN {
                anyhow::bail!("search term must be at least {} characters", bole::MIN_SEARCH_TERM_LEN);
            }
            let self_key = signer_from(&key_env, key_file.as_deref())?.public_key();
            // ... rest of the handler unchanged ...
```

> The handler returns `anyhow::Result` (`use anyhow::Result;` at the top of the file), so `anyhow::bail!` is the idiomatic early return. Placing the check before `signer_from` guarantees no key resolution, no connection, and no work for a too-short term, on BOTH the `Some(endpoint)` and `None` (pinned-set) paths (they are reached later in the same handler).

- [ ] **Step 4: Run it, verify it passes** — `cargo test -p bole-cli --test collab_cli cli_discover_relay_rejects_short_term` → PASS.

- [ ] **Step 5: Full CLI suite + build + clippy**

```bash
cargo build --workspace
cargo test -p bole-cli
cargo clippy --workspace
```

Expected: all `collab_cli` tests pass, including the existing `cli_discover_relay_search_transparent` and `cli_discover_relay_trust_path` (a valid term like "Pat" is ≥3 chars, so those are unaffected); clippy clean.

- [ ] **Step 6: Commit**

```bash
git add bole-cli/src/commands/discover.rs bole-cli/tests/collab_cli.rs
git commit -m "<bead-id>: discover relay rejects too-short term locally before connecting"
```

---

## Self-Review

**Spec coverage:**
- §2 constants `MAX_SEARCH_HOPS = 6`, `MIN_SEARCH_TERM_LEN = 3` → G1 Steps 1-2. ✅
- §3 relay-side enforcement (reject too-short term before corpus work; clamp `max_hops` before `search_ball`; `search_ball`/wire untouched) → G1 Step 5. ✅
- §4 client `Error`→`Err` mapping (skip-and-continue) → G1 Step 6; CLI local pre-check (both paths, no surface change) → G2 Step 3. ✅
- §5 tests: clamp (255 → depth-6 ball), reject (Error + zero work) → G1 Steps 3/8; loopback (client Err, clamped result, query_relay_set robustness) → G1 Step 8; CLI E2E (short term fails locally / never connects; valid term works) → G2 Step 1. ✅
- Invariants (relays not authoritative, availability-not-soundness, endpoint read-only/no new verbs, depth-2 + HaveWant untouched, keys raw hex, transient no-mutation, no CLI surface change) → Global Constraints + carried per gate. ✅

**Deviation from spec test wording (flagged):** the spec §5 loopback item "a relay that errors on a too-short term is skipped while a healthy relay's hits still appear" cannot be literally realized — the min-term bound is *uniform*, so a too-short term errors on ALL relays. G1 Step 8's `query_relay_set_handles_short_term_via_skip` implements the faithful intent: a valid term returns merged hits, and a too-short term makes every relay reject → the set query returns empty via skip-and-continue (no crash). This preserves the spec's purpose (skip-and-continue robustness) without a scenario the uniform bound forbids.

**Placeholder scan:** the G1 Step 3/8 and G2 Step 1 test bodies specify assertions in prose plus the exact functions/constants under test, because they must reuse the existing loopback/process harness in those files rather than reinvent it; the concrete assertions (which edge absent, Error not Pack, Err return, exit + substring) are fully specified.

**Type consistency:** `MAX_SEARCH_HOPS: u8`, `MIN_SEARCH_TERM_LEN: usize`, `term.len()` (bytes) `< MIN_SEARCH_TERM_LEN`, `max_hops.min(MAX_SEARCH_HOPS)`, `Message::Error(String)`, `anyhow::bail!` — consistent across gates and matching the live signatures (`serve_collab` Search arm, `search_or_fallback`, `Cmd::Relay`).

**Open verification items for implementers (named in-step):** the exact sibling loopback helper/`decode_pack` path in collab_network.rs (G1 S3); whether a failing-command test helper exists in collab_cli.rs or the binary must be spawned directly (G2 S1).
