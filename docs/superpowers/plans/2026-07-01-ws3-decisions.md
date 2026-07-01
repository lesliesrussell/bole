# WS3 — Locked Implementation Decisions (bole-9mz)

Resolves the seven open questions in
[specs/2026-06-29-ws3-secrets-env.md](../specs/2026-06-29-ws3-secrets-env.md).
Maintainer-approved 2026-07-01.

| OQ | Decision |
|----|----------|
| **Scope** | Build the **core + `env resolve` + `run` + `secret rekey`**; defer KMS. `KmsKeyProvider`/`KmsClient` (O7) tracked in **bole-vw9**. |
| **O1 — MK sourcing** | Env/file providers for dev/CI; KMS (deferred) for prod. On-demand rekey; no enforced cadence. |
| **O2 — secret label** | Reuse WS1's `LabelRule::Secret { name, label }` + `Accessor::can_read_secret` (already shipped in WS1). No new WS1 surface. |
| **O3 — AAD** | Bind `{version, label}`; reserve `secret_uid: [u8;16]` field for later per-identity binding. |
| **O4 — rekey refs** | Auto-repoint registry-named secrets (and overlays referencing them) to the new object id; leave old objects for WS4 GC. |
| **O5 — resolve/run** | `resolve_overlay` is **fail-closed** (uncleared secret → `Error::AccessDenied`, names the var never the value); `--skip-unauthorized` opt-in to omit. `bole run` defaults to `--inherit` (parent env + overlay); `--clean` opt-in. |
| **O6 — DK granularity** | Per-secret data key (blast-radius isolation). |
| **O7 — KMS** | Deferred to **bole-vw9**: `KmsClient` = encrypt/decrypt only, one reference adapter behind a feature flag. |

## Deviation from spec §4.1 / §5.3 — versioning mechanic

The spec sketches a single `Secret` enum `{V1, V2}` with an
`#[serde(untagged)]`-style probe so legacy objects keep their `ObjectId`.
**This cannot work here: the codec is `postcard`, which is not self-describing,
so `deserialize_any`/untagged is unsupported.** An explicit inner enum
discriminant would shift every legacy byte and change existing ids.

**Chosen mechanic (postcard-correct, id-preserving):**
- Keep the existing `Secret` struct **byte-identical** (it is "v1"); existing
  stored objects and their `ObjectId`s are unchanged, and all current secret
  tests pass untouched.
- Add a new **`Object::SecretV2(SecretV2)`** variant for envelope secrets — the
  same pattern WS1 used for `Object::Policy`. Version is carried by the `Object`
  discriminant, which postcard encodes natively. Reading old data is unaffected;
  new secrets are written as `SecretV2`.
- `Secret` (v1) and `SecretV2` decrypt via a `ProviderChain` that dispatches on
  the stored variant.

Consequence for §5.3: WS3 **does** add an `Object` variant (contrary to the
spec's stated preference), justified by postcard. WS4's pack codec rides the
single postcard path, so a new variant is free (as with `Object::Policy`).

## Store API compatibility

The existing raw-key `ObjectStore::put_secret(plaintext, &[u8;32])` /
`get_secret(id, &[u8;32])` are **kept** (they write/read v1) so all current
tests stay green. New enveloped methods are added alongside:
`put_secret_enveloped(plaintext, &dyn KeyProvider, aad)` and
`get_secret_resolved(id, &ProviderChain)`, plus `rewrap_secret`. New code and
the CLI use the enveloped path.

## Build order (TDD)

1. `crypto::key_provider` — `WrappedKey`, `KeyProvider` trait (async), local
   provider (MK + `key_ref`), `ProviderChain` (v2 providers + v1 legacy keys).
2. `object::secret` — `SecretV2` + `SecretAad`; `encrypt_envelope` / `decrypt`.
   Add `Object::SecretV2`; update all `Object` matches (incl. bole-cli).
3. `store` — `put_secret_enveloped`, `get_secret_resolved`, `rewrap_secret`.
4. `repo` — `resolve_overlay` (WS1-gated, fail-closed), `rekey` (auto-repoint).
5. CLI — `key.rs` builds a `ProviderChain`; `env resolve`, `run`, `secret rekey`.
6. Full `cargo test --workspace` green (verify exit code, not grep).
