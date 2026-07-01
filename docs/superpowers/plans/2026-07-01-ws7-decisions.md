# WS7 — Locked Implementation Decisions (bole-3hj)

Resolves the open questions in
[specs/2026-06-29-ws7-hardening-repositioning.md](../specs/2026-06-29-ws7-hardening-repositioning.md).
Maintainer-approved 2026-07-01.

| OQ | Decision |
|----|----------|
| **Scope** | Part A (hardening) **in full** + targeted Part B: README positioning + honest Status/roadmap table marking WS1–6 realized. Full narrative rewrite of `docs/CLI.md`, `docs/API.md`, `bole-cli/README.md`, and the three skill files → **bole-9c0**. |
| **OQ1 — prune exit code** | git-like: `0` on completion (incl. 0 pruned), non-zero only on error (`git worktree prune` convention). |
| **OQ2 — repair auto-fix R1** | Auto-fix store-moved (R1) entries idempotently; `--dry-run` previews. No `--yes` gate (rewrites are safe + reported). |
| **OQ3 — list --check** | Exit `1` when any linked worktree is stale, `0` when clean (`git diff --exit-code` convention). |
| **OQ4 — adopt missing metadata** | Write `CliState::default()` (unbound); the user re-binds with `workspace open`. |
| **OQ5 — orphan-metadata scan** | Deferred (harmless; auto-scanning widens blast radius). |
| **OQ6/7/8 — doc-sync CI, skill parity, roadmap markers** | Convention first; CI gates deferred. |

## Part A shape

`bole-cli/src/worktrees.rs`: `Status` enum (`Ok` / `MissingDirectory` /
`MissingPointer` / `BadPointer` / `WrongId` / `WrongStore`) + pure
`classify(repo_dir, id, &Entry)` reading `<path>/.bole` via `std::fs`.
`commands/workspace.rs`: `prune` (remove metadata dir + only the `.bole` pointer
for bad/wrong-id, never user files), `repair` R1 (rewrite pointers) / R2
(`--moved-to <path> <id>`) / R3 (`--adopt <path>`), `list` status field +
`[STALE: …]` annotation + `--check`. 8 integration tests in
`bole-cli/tests/workspace_hardening.rs`.

## Part B (targeted)

README leads with the access-control differentiator (positioning statement),
reframes features as capabilities, and carries the canonical Status/roadmap table
— WS1–6 realized this session, with networked transports / signed policy
verification / KMS / CLI verbs marked roadmap. Honesty rule preserved:
present-tense claims run today; roadmap items are labeled.
