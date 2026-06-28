## Permission lattice: core concepts

### Objects that participate in permissions

In the new system, permissions apply to **objects** in the graph, not just files: [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)

- Paths (logical files/directories, e.g. `src/app.zep`, `infra/terraform`, `secrets/db`)  
- Snapshots (entire states of the project)  
- Timelines (ordered sequences of snapshots, e.g. `main`, `leslie/exp-foo`, `release/2026-06`)  
- Tags (named pointers: `v1.2.0`, `prod`, `leslie/private-notes`)  
- Special nodes: `Secret`, `EnvOverlay`, `Policy`

Each object carries a **security label**; the lattice is the partial order over these labels that controls who can read or write what. [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)

Example labels:

- `public`  
- `team/<name>`  
- `role/<name>` (e.g. `role/dev`, `role/devops`)  
- `confidential`, `secret`, `restricted/*`  

You don’t have to over‑engineer the lattice at first; you can implement it as a small hierarchy and keep the comparison rules simple, e.g. `public < team/dev < confidential < secret`. [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)

***

## Permission lattice: structure and semantics

### The lattice itself

You can model the lattice as:

- A finite set of labels `L`.  
- A partial order `≤` on `L` (“can flow from lower to higher sensitivity”). [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)
- A user/agent context `C` (roles, groups) that maps to an **effective label** or set of labels they’re allowed to access.

Two key ideas:

1. **Read**: A user with context `C` can read object with label `l` if `l ≤ label(C)` or if `C` explicitly has capability for `l`.  
2. **Write**: A user can write object with label `l` if they can read it and their context permits `write(l)` capability.

You can also borrow from information‑flow control:

- Disallow flows from higher sensitivity to lower sensitivity (e.g. copying `secret` into `public`) unless a declassification policy node explicitly permits it. [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)
- Express declassification as a special object, e.g. `Policy` saying “mask fields X,Y when copying from `secret` to `public`.”

### Private files and branches as lattice instances

“Private file” = a path whose label is higher than the label of the timeline or tag that a user usually sees.

Example:

- Path `secrets/prod.key` carries label `secret/prod`.  
- Timeline `main` has label `public`.  
- User `leslie` has context `team/dev` plus `capability(secrets/dev)`, but not `secrets/prod`.

Effects:

- When `leslie` fetches `main` with default permissions, `secrets/prod.key` is simply absent from the view; it doesn’t show up in diffs or status. [auditbuffet](https://auditbuffet.com/patterns/ab-000989)
- A specialized “security admin” context can open a view where `main` + `secret/prod` paths are visible, but merges into a `public` timeline must respect declassification policies.

“Private branch” = a timeline/tag whose own label is more restrictive than `main`.

Example:

- Timeline `leslie/exp-foo` has label `team/dev`.  
- Timeline `main` has label `public`.  

You get three behaviors:

1. Within `leslie/exp-foo`, you can have files labeled `team/dev` and `public`.  
2. Trying to merge `leslie/exp-foo` into `main` triggers a policy check: only `public`‑labeled content may flow into `main` unless there is an explicit declassification rule.  
3. Another dev with `team/dev` context can see `leslie/exp-foo` if policy allows cross‑dev sharing; otherwise it’s effectively a truly private branch.

***

## Permission lattice: operations and policies

### Key operations

1. **View derivation**: Given a snapshot `S` and a context `C`, derive a visible tree `T(C, S)` by removing all paths whose label is not visible to `C`.  
2. **Merge with policies**: When merging `S_a` into `S_b`, compute visibility and sensitivity of paths; reject or transform operations that would violate the lattice (e.g., copy a `secret` into `public`). [cs.up.ac](https://www.cs.up.ac.za/cs/eloff/journals/3.pdf)
3. **Tag/timeline moves**: Moving `main`’s head can’t implicitly change labels; labels travel with the objects, not with the pointer.  
4. **Capability grants**: Adding an agent or human with limited access is done by updating the lattice mapping, not by moving files into separate repos.

### Example policies

- “No `secret/*` path may be visible on any timeline tagged `public`.”  
- “Agent `ai-formatter` may only modify paths labeled `public` under `src/**`.”  
- “Declassify `secret/config.yaml` into `public/config.yaml` by stripping sensitive keys and writing defaults.”

Policies are themselves versioned objects:

- You can have snapshots where a policy existed or didn’t exist and diff them.  
- You can roll back a policy change just like code, which is critical for reproducibility.

***

## Permission lattice: tests

A few concrete TDD tests, layered on top of what we already sketched:

- **PT1 – Hidden paths**:  
  - Create paths `src/app.zep` (public) and `secrets/prod.key` (secret/prod).  
  - As user with `team/dev` but no `secret/prod`, fetch view; assert only `src/app.zep` is present.  
  - As user with `secret/prod`, fetch view; assert both paths appear. [auditbuffet](https://auditbuffet.com/patterns/ab-000989)

- **PT2 – Merge rejection on policy violation**:  
  - Policy: “No `secret/*` under `main`.”  
  - Add `secrets/prod.key` to a dev timeline and attempt to merge into `main`.  
  - Assert the merge fails with a policy violation and suggests declassification or target change.

- **PT3 – Agent capability enforcement**:  
  - Agent context: `role/formatter` with capability `write(public, src/**)` only.  
  - Prepare edits touching `src/app.zep` and `secrets/prod.key`.  
  - Assert only `src/app.zep` change is accepted; the secret touch is rejected or ignored, with audit log. [blacksmith](https://www.blacksmith.sh/blog/best-practices-for-managing-secrets-in-github-actions)

- **PT4 – Declassification transform**:  
  - Policy: “When copying `secret/config.yaml` to `public/config.yaml`, drop field `password`, replace `api_key` with `***`.”  
  - Perform copy; assert target content matches transform and labels are `public`.

***

## Secrets/env overlay model: core ideas

The model pulls secrets and envs out of the file world and into typed, permissioned graph nodes: [dev](https://dev.to/armorbreak/the-env-file-is-not-a-security-strategy-12pc)

- `Secret`: a value (string, JSON, whatever) with its own encryption, label, and history.  
- `EnvOverlay`: a mapping from keys to either literals or references to `Secret` values (e.g. `DB_URL = ref(secret/db_url)`). [nodejs-security](https://www.nodejs-security.com/blog/do-not-use-secrets-in-environment-variables-and-here-is-how-to-do-it-better)
- `WorkspaceView`: for a given snapshot + env context, a derived environment that is applied at runtime, not committed as `.env` files.

Key properties:

- Secrets are never stored as plain blobs that can be diffed like code. [youtube](https://www.youtube.com/watch?v=z5N3zJuxv3M)
- You can attach different env overlays (dev, staging, prod) to the same code snapshot.  
- All env/secrets operations are auditable and policy‑checked.

***

## Secrets/env overlays: structure

### Secret object

Fields:

- `id`: content‑addressed identifier, but using encrypted payload.  
- `label`: e.g. `secret/prod`, `secret/dev`.  
- `value`: encrypted blob.  
- `metadata`: created_at, creator, rotation policy, etc. [keyenv](https://keyenv.dev/blog/env-file-security-risks/)

Operations:

- `create_secret(label, value)`  
- `rotate_secret(id, new_value)`  
- `revoke_secret(id)`  
- `grant_access(id, contexts)` / `revoke_access(id, contexts)`

### EnvOverlay object

Think of this as “configuration overlay”:

- `id`: identifier.  
- `label`: e.g. `env/dev`, `env/staging`, `env/prod`.  
- `entries`: list of `{key, source}` where `source` is either:  
  - literal (`value="sqlite://dev.db"`)  
  - secret reference (`secret_id=<id>`, maybe plus transform)  
  - computed expression (e.g. `join("postgres://", ref(secret/host))`) [keyway](https://keyway.sh/articles/env-files-not-safe-2026)

Env overlays are versioned:

- You can diff `env/dev@t1` and `env/dev@t2` safely without revealing actual secret values (only structure changes).

### Binding overlays to snapshots

You don’t commit `.env` files; instead, snapshots record:

- “Default env overlay: `env/dev`” (at dev timeline level).  
- “For release timeline, attach `env/prod`.” [blacksmith](https://www.blacksmith.sh/blog/best-practices-for-managing-secrets-in-github-actions)

When you materialize a workspace view:

- Choose a snapshot `S`.  
- Choose an env overlay `E` according to policy (user context, timeline, environment).  
- Compute `WorkspaceView(S, E, C)` where `C` is permission context.

Secrets appear only at runtime (process env, container env, config injection) and never as files tracked in the snapshot. [youtube](https://www.youtube.com/watch?v=z5N3zJuxv3M)

***

## Secrets/env overlays: flows and policies

### Secure flow patterns

- Git‑style disaster (accidentally committed `.env`):  
  - In this model, `.env` as a tracked file simply doesn’t exist; secrets and envs live in their own graph, governed by separate APIs. [envmaster](https://www.envmaster.dev/blog/why-env-files-are-a-security-risk)
- Rotation:  
  - Rotate a secret; all env overlays that reference it automatically see the new value when materialized, without changing code snapshots. [keyenv](https://keyenv.dev/blog/env-file-security-risks/)
- Least privilege:  
  - A dev context might only be allowed to use `env/dev`, not `env/prod`.  
  - CI in `prod` might require multi‑party approval to attach `env/prod` (similar to environment secrets in GitHub Actions). [blacksmith](https://www.blacksmith.sh/blog/best-practices-for-managing-secrets-in-github-actions)

### Policy examples

- “No workspace view for context `team/dev` may resolve `env/prod`.”  
- “All secrets must be referenced via overlays; direct secret access requires admin context.”  
- “Logging system must mask or drop all secret values when tracing env resolution.”

These are expressed as `Policy` objects in the graph that get evaluated at env resolution time.

***

## Secrets/env overlays: tests

- **ST1 – No `.env` in history**:  
  - Try to add a `.env` file with secrets to the repo; assert the system rejects it or automatically converts entries into `Secret` + `EnvOverlay`, leaving no plaintext secrets in snapshots. [envmaster](https://www.envmaster.dev/blog/why-env-files-are-a-security-risk)

- **ST2 – Overlay binding and separation**:  
  - Create a code snapshot `S` with no `.env` files.  
  - Create `EnvOverlay dev` and `EnvOverlay prod`.  
  - Materialize two workspace views; confirm code is identical, env differs, and snapshots’ stored content is unchanged.

- **ST3 – Secret rotation**:  
  - Create secret `db_password` with value `old`.  
  - Attach it to `EnvOverlay prod` and materialize workspace; confirm runtime sees `old`.  
  - Rotate secret to `new` and re‑materialize; runtime sees `new`, but snapshot ids and diffs remain exactly the same.

- **ST4 – Permission enforcement on envs**:  
  - Context `team/dev` can use `EnvOverlay dev` but not `EnvOverlay prod`.  
  - Attempt to materialize `env/prod` in dev context; assert failure with policy violation.  
  - In CI context with `role/release`, assert it can materialize `env/prod` only when a specific approval artifact exists. [blacksmith](https://www.blacksmith.sh/blog/best-practices-for-managing-secrets-in-github-actions)

***

## How the lattice and overlay model meet

The two models interlock:

- Secrets and env overlays themselves carry labels (`secret/prod`, `env/dev`, etc.), participating in the same lattice as files. [blacksmith](https://www.blacksmith.sh/blog/best-practices-for-managing-secrets-in-github-actions)
- Visibility derivation runs on both file paths and env/secret nodes; a dev may know a secret exists without seeing its value.  
- Policies can encode cross‑domain rules: “If a timeline is labeled `public`, it can never be bound to `env/prod`” or “only contexts at or above `team/devops` may see infra secrets.”

One illustration:

- Code timeline `main` at label `public`.  
- Secret store has `db_password_prod` at label `secret/prod`.  
- Overlay `env/prod` references `db_password_prod`.  
- A release pipeline context with `role/release` and `secret/prod` capability can bind `main + env/prod` to produce a prod deployment, but no one browsing `main` in dev tools sees the prod secrets or env. [keyway](https://keyway.sh/articles/env-files-not-safe-2026)
