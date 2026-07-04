# bole — Session Handoff (2026-07-04)

> Read this first when resuming. It is self-contained: with `STATUS.md`, `SCOPE-AUDIT.md`, and `bd list`
> you need nothing else to pick up. **Paused deliberately for a scope/direction rethink** — the code is
> healthy; the question on the table is *what bole should be*, not *is it broken*.

## TL;DR

We shipped the entire discovery/relay stack (WS8a→f-c1, 7 pushed tags), then stepped back because the
project felt like it was sprawling. Two assessments followed: a **STATUS map** (what's built vs. the
missing product-facing API) and a **SCOPE-AUDIT** (is it a Frankenstein? — no: clean, disciplined,
over-provisioned substrate). Mission got sharpened: **bole is the headless backend/API; Grove is the
user-facing frontend, built later in a separate repo.** A first backend-API operation (profile-bundle
read) was designed + planned but **not built** — paused here to think about scope.

## Repo state

- **Pushed to `origin/master`:** everything through tag **`ws8f-c1-relay-search-cost-bounds`** (commit the
  tag points at). All code + tests are pushed and green.
- **Local commits NOT pushed (5, all docs):** STATUS map + backlog, backend-API reframe, profile-bundle
  **spec**, scope audit, profile-bundle **plan**. HEAD = `c57f053`+1. **If resuming on another machine,
  `git push origin master` first** (these are just docs; safe to push).
- **Working tree:** clean. Tags: `ws8b`→`ws8c`→`ws8d`→`ws8e`→`ws8f-a`→`ws8f-b`→`ws8f-c1` (all pushed).
- Build: `cargo test --workspace` green (~350 tests), `cargo clippy --workspace` clean.

## Mission (the crisp version)

- **bole (this repo)** = content-addressed distributed VCS + the **backend/API** for Grove. Exposes hub
  operations (profiles, repos, timelines, discovery, trust, access, and eventually PRs/discussions) as a
  clean, verified, JSON-clean API. **No UI here.**
- **Grove** = the frontend hub ("better GitHub than GitHub"). **Separate repo, later. Out of scope.**
- Pillars: secure · distributed · discoverable · + product-API surfaces (PR/board/profile — as API ops,
  not UI).

## What's shipped vs. missing

See `docs/superpowers/STATUS.md` (pillar scorecard). Short version:
- **Deep & done:** VCS core, access-control lattice, secrets, git interop, distributed sync, and the
  *entire* discovery/relay stack (10 slices).
- **Thin & missing:** the product-facing **API surface** (PR/board/profile-bundle) and the **transport**
  a non-Rust Grove calls. This is the real gap.

## Scope-audit conclusion (why we paused)

See `docs/superpowers/SCOPE-AUDIT.md`. Verdict: **not a monster** — clean acyclic layering, 13 lean deps,
zero rot markers, every subsystem load-bearing. The issue is **scope depth**, not quality:
- **Freeze (shipped is enough):** discovery tail (WS8f-c2/c3/c4/d, DNS, reputation), further ACL/IFC depth,
  git round-trip gaps. *Stop deepening the substrate.*
- **Consolidate:** `MultiRecipientSecret` — built + persistable but **no consuming feature** (bead
  `bole-oea4`). Wire it or remove it; collapse secrets to `Secret`+`SecretV2`.
- **Build (the real mission):** the Grove hub **API** — start with profile-bundle read, then PR/board.
- **Optional cleanups:** split `repo/mod.rs` (2183 lines); surface-or-document `sync/http.rs` (built,
  tested, not CLI-wired).

## In-flight work (designed, NOT built)

**Profile-bundle read API** — bead **`bole-k93a`**, the first backend-API operation.
- **Spec (committed):** `docs/superpowers/specs/2026-07-04-profile-bundle-read-api-design.md`
- **Plan (committed, NOT executed):** `docs/superpowers/plans/2026-07-04-profile-bundle-read-api.md`
- **What it is:** `Repository::profile_bundle(key)` → verified identity + own trust out-edges + (local)
  repo timelines, as one stable JSON contract; CLI `bole profile bundle [<key-hex>] [--json]`. 2 gates,
  full code in the plan, grounded in live APIs.
- **To resume building it:** invoke `superpowers:subagent-driven-development` on the plan (branch =
  `bole-k93a`, per-gate sub-beads OK), same flow as every WS8f slice. **But decide direction first**
  (below) — this build is only right if "build the API surface now" is the chosen path.

## Open decisions to sit with

1. **Scope posture:** accept the audit's freeze recommendation (stop deepening substrate), or keep
   hardening discovery? The audit argues freeze.
2. **`MultiRecipientSecret`** (`bole-oea4`): wire to a real per-recipient-secret workflow, or remove +
   consolidate secrets to two schemes?
3. **First API surface:** profile-bundle read (designed, cheapest) → then PR system? Or a different first
   cut once you've thought about what Grove most needs from the backend?
4. **Transport** (`bole-yd56`): defer the HTTP/JSON-RPC hub-API server until Grove's stack is known
   (recommended), or stand up a minimal one early to pin the contract?
5. **Bigger frame:** is bole trying to be too many things? The audit says the substrate is over-built for
   the mission — worth deciding whether some pillars (deep IFC, deep relays) are frozen indefinitely.

## Backlog

Lives in **beads** (`bd list` = source of truth; `bd ready` for actionable). ~19 open, labeled by track:
`track:product` (PR/board/landing-API), `track:security`, `track:discovery`, `track:distribution`,
`track:vcs`, `track:cleanup`. Narrative view in `STATUS.md`; freeze/prune rationale in `SCOPE-AUDIT.md`.
Key beads: `bole-k93a` (profile-bundle, designed), `bole-j899` (PR system, epic), `bole-yd56` (API
transport, decision), `bole-oea4` (MultiRecipientSecret decision).

## How to resume (entry points)

1. Read this file → `STATUS.md` → `SCOPE-AUDIT.md`.
2. `bd list` / `bd ready` for the tracked backlog.
3. If building: pick a bead, `bd update <id> --claim`, branch = bead ID, brainstorm→spec→plan→
   subagent-driven-development (specs/plans already exist for `bole-k93a`).
4. Git norms this project used: **commit-no-push per slice; push + `git tag` only on explicit go-ahead at
   a workstream boundary.** Don't push without being asked.

## Process notes (so a fresh session isn't surprised)

- **Reviewer flakiness:** 3 subagent code-reviews died on infra this session (connection-closed /
  watchdog). Mitigation used: controller-side review on small diffs (per the WS8e precedent). If it
  recurs on a large diff, split the diff or retry on a mid-tier model rather than lean on controller
  review.
- **Execution flow:** every slice ran brainstorming → writing-plans → subagent-driven-development, with a
  per-gate bead+branch, two-stage review, and a final whole-branch review before push+tag. Ledgers live
  in `.superpowers/sdd/*-progress.md` (git-ignored scratch).
- **Env noise (harmless):** `bd` prints "beads: repair: … dolt remote" on most calls; the sandbox shell
  prints `cd:2: no such file or directory` / `sort: No such file or directory` on compound commands.
  Neither indicates failure.
- **`.beads/*.jsonl`** are passive exports (dolt DB is source of truth); they show as modified often.

## Bottom line for whoever resumes

The substrate is solid and pushed. The pause is a *thinking* pause about scope and direction, not a
recovery from breakage. Start from the five open decisions above — the most consequential is #1/#5 (is the
substrate done enough; redirect to the thin API layer?). Nothing needs fixing to move; it needs deciding.
