<!-- planning sketch -->
# Grove Hub — Multi-User Repo Hosting (design sketch)

> **Status: sketch.** Grove is not a viewer of one node — it is a **hub** where
> many bole users push their repos, each user's profile lists all *their* repos,
> and anyone can pull a pushed repo. This is the GitHub-shaped model. It needs a
> repo-ownership + server-namespacing layer bole does not have yet. Drives a
> bole epic + slices.

## The model

- **A hub is one bole store** (the "hub node"). It holds every user's repos in
  one ref space, namespaced by owner:
  `refs/users/<owner-key-fp>/<repo>/<timeline>`.
- **A repo is a first-class, owned, named thing** — represented by a signed
  `RepoRecord` object `{ owner: Key, name, description, seq, sig }`, published
  under the owner's public collab prefix
  (`refs/collab/public/repo/<owner-fp>/<name>`), discoverable like a `Profile`.
- **A user's profile lists all their repos** — `profile_bundle` (and the
  landing page) enumerates the owner's `RepoRecord`s.
- **Push is authenticated and owner-scoped** — a user signs their push; the hub
  maps the connection to the owner key and ACL-refuses any write outside
  `refs/users/<owner-fp>/**`. You can only push your own repos.
- **Anyone can pull** a specific repo (`bole fetch`/clone the owner's
  `refs/users/<owner-fp>/<repo>/…`, public read).

## Why this shape

It reuses everything already built: signed collab objects (`RepoRecord` mirrors
`Profile`/`Post`), the native sync (`bole serve`/`push`/`fetch`), the label-ACL
(scope a writer to `refs/users/<key>/**`), and authn (`sync::authn`
Principal→actor). The only genuinely new mechanism is **authenticated,
owner-scoped push** — carrying a signed owner identity on the wire and building
a namespace-scoped accessor server-side.

## Surfaces (bole-api + Grove)

| Grove page | Data | bole-api |
|---|---|---|
| Profile → repos | owner's `RepoRecord`s | `GET /v1/profiles/{key}/bundle` (add `repos[]`) |
| Repo page | a repo's timelines + how to pull | `GET /v1/repos/{owner}/{name}` (new) |
| (existing) snapshot/file, PR, board | unchanged | live |

## Decisions taken

- **Hub store:** one shared store, owner-namespaced refs (not store-per-repo).
- **Ownership:** authenticated — only the owner-key holder may write their
  namespace.
- Naming: the object is `RepoRecord` in the library (distinct from the existing
  `Repository` store handle); user-facing it is "repo".

## Slices (tracer bullets)

1. **`RepoRecord` object + publish/list** (library) — signed object, published
   under `refs/collab/public/repo/<owner-fp>/<name>`, verified fail-closed;
   `publish_repo` / `list_repos(owner)`. Mirrors `Post`.
2. **profile → repos** — `profile_bundle` includes `repos[]`; bole-api bundle +
   Grove profile page list them; a repo page `/repo/{owner}/{name}` shows the
   `RepoRecord`.
3. **Owner-namespaced authenticated push** (the crux) — the pusher signs an
   identity on the wire; the hub verifies it, builds an accessor scoped to
   `refs/users/<owner-fp>/**`, and lands content there. `apply_push_ops` already
   refuses out-of-scope writes via ACL; this slice wires the auth + scoping. May
   split into (3a wire+authn, 3b CLI `bole push --as`).
4. **Pull a repo** — `bole clone/fetch` a specific owner/repo; Grove shows the
   pull command per repo.
5. **bole-api hub endpoints** — `GET /v1/users/{key}/repos`,
   `GET /v1/repos/{owner}/{name}` for Grove; Grove repo pages consume them.

Slice 3 is the load-bearing security slice (authenticated ownership); 1–2 and
4–5 are the same object→API→Grove pattern already used for PR/board.
