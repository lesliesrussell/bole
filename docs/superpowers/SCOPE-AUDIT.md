# bole — Scope Audit (freeze / prune map)

> Feature-by-feature verdict against the mission **"bole = backend API for Grove"** (Grove frontend is a
> separate later repo). Answers the "did we build a monster / build ahead of need?" question with
> integration evidence, not opinion. Companion to `STATUS.md`. Audited 2026-07-04.

## Headline

**The code is healthy, not a monster** — clean acyclic layering, 13 lean deps, zero `unimplemented!`/`TODO`,
~350 green tests, every subsystem load-bearing. What's off is not *quality* but *scope depth*: effort
concentrated in a few pillars (discovery, access-control) well past what a not-yet-existent frontend pulls
on, plus one genuinely unused capability. Nothing here argues for a rewrite; a few things argue for a
**freeze** and one **consolidation**.

## Verdict legend

- **CORE** — required for a hub backend; keep and maintain.
- **PLAUSIBLE** — reasonably needed, but not urgent for the backend-API mission; keep, don't deepen.
- **AHEAD** — built past current need / not yet pulled on by any consumer; freeze or consolidate.

## Feature-by-feature

| Subsystem | ~LOC | Integration | Verdict | Note |
|---|---|---|---|---|
| **VCS core** — object store, trees, snapshots, refs, timelines, packs/GC (`object` 830, `store` 1958, `refs` 1359, `repo` 5420) | ~8.5k | foundation; used by everyone | **CORE** | The spine. Every hub op sits on this. `repo/mod.rs` (2183 lines, ~half tests) is the one split candidate. |
| **Distributed sync** — pack-delta wire, TCP + HTTP transports, AUTHN/AUTHZ (`sync` 3422) | 3.4k | top layer; enforces `acl` | **CORE** | The "distributed" pillar. HTTP transport (`sync/http.rs`) is peer fetch/push, **not** a hub API, and is **not CLI-wired** — surface it or note it as library-only. |
| **Access control** — label lattice, clearances, IFC (confined/no-write-down), policy hooks (`acl` 3346) | 3.3k | used by `object`/`repo`/`sync` | **CORE pillar, AHEAD depth** | The "secure" pillar, genuinely load-bearing (sync authz uses it). But it's as large as all of sync, and the IFC sophistication (information-flow no-write-down) exceeds what a v1 GitHub-like hub needs. Keep (foundational, hard to retrofit); **freeze further ACL depth.** |
| **Discovery / relays** — profiles, trust edges, trust-paths, relays, multi-relay auth, server-side search, cost bounds (`collab` 1691 + WS8 sync) | ~2.5k | used by `repo`/`sync`/CLI | **CORE pillar, AHEAD depth** | The "discoverable" pillar — 10 shipped slices (WS8a→f-c1). Excellent, but **very deep for a frontend that doesn't exist yet**. Keep shipped; **freeze the tail (WS8f-c2/c3/c4/d, DNS, reputation)** until Grove pulls on it. |
| **Secrets / env** — `Secret` (raw-key), `SecretV2` (master-key envelope + KMS), `EnvOverlay` (`object/secret` 470, `crypto` 393) | ~0.9k | `Secret`/`SecretV2` wired into env/clearance/session/KMS | **PLAUSIBLE** | CI/Actions-style secrets — reasonable for a hub. `Secret`+`SecretV2` are load-bearing. Keep. |
| **`MultiRecipientSecret`** — per-recipient DK wrapping | (in secret 470) | `bole secret share` + `grant-actor`/`revoke-actor`/`reveal` | **WIRED** | Resolved (bole-oea4): `secret share` creates it for N recipients up front; `grant-actor` upgrades a plain secret to it; `reveal` reads it. Load-bearing, not ahead-of-need. |
| **Git import/export** — `gix`-backed round-trip (`git_import` 812, `git_projection` 578) | ~1.4k | `repo` | **PLAUSIBLE** | Migration bridge from git. Useful, not core to the hub product. Keep; **freeze round-trip gap-filling** (submodules, annotated-tag export, `--prune`). |
| **Workspaces / virtual repos / multi-actor** — `Workspace` trait, disk+ephemeral, actors/approvers | (in repo/refs) | `repo` | **CORE** | VCS infra + the "agents & humans as actors" pillar. Keep. |

## The actual gap (from STATUS.md)

Everything above is *backend substrate*. The mission — **expose the hub operations a frontend needs** — is
barely started: the **product-facing API** (profile-bundle read [designed], PR system, message board) and
the **transport** a non-Rust Grove calls (`bole-yd56`). The imbalance is real: deep substrate, thin API.

## Freeze / prune / build map

**Consolidate (built ahead of need, low pull):**
- **`MultiRecipientSecret`** — RESOLVED (bole-oea4): wired to `bole secret share` (creates a multi-recipient secret for N recipients), consumed by `grant-actor`/`reveal`. No longer a consolidation target.
  to `Secret` + `SecretV2`. Decide deliberately; don't leave a third scheme no one drives. (bead: file one)

**Freeze (shipped is enough; stop deepening until a consumer pulls):**
- Discovery tail: WS8f-c2/c3/c4/d, DNS alias verify, Profile timestamp, search index.
- Further ACL/IFC depth beyond what's shipped.
- Git round-trip gap-filling (submodules, annotated-tag export, `--prune`, thin-pack deltas).

**Build (the real mission — where new effort should go):**
- The Grove-backend **API surface**: profile-bundle read (designed, `bole-k93a`), then PR/board read+write
  ops — transport-agnostic, JSON-clean.
- The **transport decision** (`bole-yd56`) when Grove's stack is known.

**Cleanups (optional, non-urgent):**
- Split `repo/mod.rs` (2183 lines) along a clear seam.
- Wire or document `sync/http.rs` (built, tested, unsurfaced).

## Bottom line

Not a monster — a disciplined, over-provisioned substrate. The correction isn't to tear down; it's to
**stop deepening the substrate, consolidate the one unused capability, and redirect effort to the thin
part: the hub API Grove will consume.**
