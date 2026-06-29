# WS3 — Secrets / Env Completion

- **Bead:** `bole-9mz`
- **Depends on:** `bole-fo2` (WS1 — Hybrid Access / Policy Core). Secret and
  overlay resolution is access-checked through WS1's `Accessor` pipeline; this
  spec consumes that surface and does not re-derive it.
- **Status:** design spec (not an implementation plan)
- **Conforms to:** [`2026-06-29-roadmap-foundations.md`](./2026-06-29-roadmap-foundations.md).
  Shared vocabulary (Label, LabelLattice, label rule, Clearance, Accessor,
  PolicyHook, content-addressed object, atomic refs) is defined there and in
  [`2026-06-29-ws1-access-policy-core.md`](./2026-06-29-ws1-access-policy-core.md);
  it is referenced, not restated.

---

## 1. Goal

Today bole can *store* an encrypted `Secret` (ChaCha20-Poly1305, nonce +
ciphertext, `src/object/secret.rs`) and *reference* it from an `EnvOverlay`
(`EnvValue::Plain | Secret(ObjectId)`, `src/object/env.rs`), keyed off a single
raw 32-byte key resolved from `$BOLE_KEY` / `--key-file` (`bole-cli/src/key.rs`).
Two things are missing for the primitives to pay off:

1. **The last mile of env resolution and execution.** There is no way to turn an
   overlay into a concrete environment (decrypting its secret refs) and hand it
   to a process. WS3 adds `Repository::resolve_overlay(...)` and the
   `bole env resolve` / `bole run` CLI surface, with resolution gated by the WS1
   access model: an actor must be *cleared for a secret's label* to decrypt it.

2. **Key management done right.** The single-key model cannot rotate cheaply,
   cannot integrate an external KMS/HSM, and binds every value to one key
   forever. WS3 introduces a `KeyProvider` trait and **envelope encryption**: a
   random per-secret **data key (DK)** encrypts the value; the DK is **wrapped**
   by a **master key (MK)** obtained from the provider. `secret rekey` then
   rotates the MK by re-wrapping DKs (cheap, no plaintext touched) instead of
   re-encrypting every value.

**Non-goals.** WS3 does **not** implement a KMS — it *integrates* with one
through a wrap/unwrap hook slot. It does not own deep CLI ergonomics (WS7) or the
sync/authority story for policy objects (WS5). It does not change the WS1
evaluation rules; it only labels secrets and calls `Accessor`.

**Backward compatibility (foundations §3).** Existing repos, `secrets.json` /
`envs.json` registries, the `--key-env` / `--key-file` flags, and the current
tests must keep working. The single-key `Secret` becomes a tagged **v1** format
that still decrypts; new secrets are written as **v2** envelope secrets.

---

## 2. Architecture

```
                         KeyProvider (master-key authority)
                ┌────────────────────────────────────────────────┐
                │ EnvKeyProvider | FileKeyProvider | KmsKeyProvider│
                │   wrap_dk(dk, aad) -> WrappedKey                 │
                │   unwrap_dk(wrapped, aad) -> dk                  │
                └───────────────┬───────────────┬─────────────────┘
                                │ wrap          │ unwrap
            put_secret          ▼               ▼          get_secret
   plaintext ──► gen DK ──► AEAD(value, DK) ──► SecretV2{wrapped_dk, nonce, ct, aad}
                                                        │
                                                        ▼ (read path)
   SecretV2 ──► unwrap_dk(wrapped_dk, aad=secret-id) ──► DK ──► AEAD⁻¹ ──► plaintext

   resolve_overlay(overlay, key_provider, accessor):
     for (var, value) in overlay.entries:
        Plain(s)           -> insert s
        Secret(id)         -> accessor.can_read_path(secret_label_path(id|name))?  ── WS1 gate
                              get_secret(id, key_provider) -> insert plaintext
     -> BTreeMap<String,String>
```

Module layout (additive; nothing deleted):

| Module | Responsibility | Status |
|--------|----------------|--------|
| `crypto::key_provider` | `KeyProvider` trait + `Env`/`File`/`Kms` impls, `WrappedKey` | new |
| `object::secret` | `Secret` enum: `V1` (legacy) + `V2` (envelope) | extended |
| `store::mod` (`ObjectStore`) | `put_secret` / `get_secret` take a `&dyn KeyProvider`; add `rewrap_secret` | rewritten signatures + compat shims |
| `repo::mod` (`Repository`) | `resolve_overlay`, `rekey` | new methods |
| `bole-cli` `env resolve`, `run`, `secret rekey` | CLI surface | new |
| `bole-cli` `key.rs` | builds a `KeyProvider` from `--key-env` / `--key-file` / `--kms` | extended |

---

## 3. Key management — the `KeyProvider` trait

### 3.1 Trait signature

The provider is the **authority for the master key only**. It never sees secret
values or data keys in plaintext outside the wrap/unwrap boundary — which is
exactly the boundary a KMS/HSM exposes (`Encrypt`/`Decrypt` of a small blob).

```rust
/// Opaque wrapped data key: the bytes a provider returns from `wrap_dk` and
/// consumes in `unwrap_dk`. For local providers this is AEAD(dk) under the MK;
/// for a KMS it is the provider's ciphertext blob (and may embed a key id /
/// version). bole treats it as opaque and stores it verbatim in SecretV2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    /// Identifies which master key / KMS key + version produced this wrap, so a
    /// reader can route to the right provider and `rekey` can detect staleness.
    pub key_ref: String,
    pub bytes: Vec<u8>,
}

/// Source of the master key and the wrap/unwrap boundary. Implementations:
/// env var, file, external KMS/HSM. bole INTEGRATES KMS here; it does not
/// implement one — `KmsKeyProvider` calls out to the operator's KMS client.
#[async_trait]
pub trait KeyProvider: Send + Sync {
    /// Stable identity of the active master key (for WrappedKey.key_ref and
    /// rekey's "is this wrap current?" check).
    fn active_key_ref(&self) -> &str;

    /// Wrap a freshly generated 32-byte data key under the active master key.
    /// `aad` binds the wrap to the secret it protects (see §4.3 / O3).
    async fn wrap_dk(&self, dk: &[u8; 32], aad: &[u8]) -> Result<WrappedKey>;

    /// Unwrap a previously wrapped data key. Must succeed for any `key_ref` the
    /// provider still recognises (current OR prior MK versions), so reads keep
    /// working across a master rotation until `rekey` re-wraps.
    async fn unwrap_dk(&self, wrapped: &WrappedKey, aad: &[u8]) -> Result<[u8; 32]>;
}
```

### 3.2 Built-in implementations

- **`EnvKeyProvider`** — master key is the 64-hex value from `$BOLE_KEY`
  (default) or `--key-env <VAR>`. `wrap_dk` = ChaCha20-Poly1305 of the DK under
  the MK with a fresh nonce; `wrapped.bytes = nonce || ct`. `active_key_ref` is a
  short fingerprint of the MK (e.g. `blake3(MK)[..8]` hex, prefixed `env:`), so
  rotating to a new MK changes the ref without ever exposing the key.
- **`FileKeyProvider`** — identical, MK read from `--key-file`; `key_ref`
  prefixed `file:`.
- **`KmsKeyProvider`** — the integration slot. `wrap_dk` / `unwrap_dk` delegate
  to an injected `KmsClient` (operator-provided trait object: AWS KMS, Vault
  Transit, PKCS#11 HSM, age plugin). bole ships the adapter shape and one
  reference client behind a feature flag; **it does not embed a KMS**. The DK
  never leaves bole in plaintext on the wrap path beyond the single KMS
  `Encrypt` call; on read, KMS `Decrypt` returns the DK and bole does the value
  decryption locally. `key_ref` carries the KMS key ARN + version.

> A single read may need to try more than one provider when secrets predate a
> provider switch. The CLI builds a **resolver chain** (active provider first,
> then any `--key-fallback` providers) and `get_secret` walks it by matching
> `WrappedKey.key_ref` / falling through `unwrap_dk` errors. The *legacy v1*
> single-key path is one such fallback (§5.1).

---

## 4. The envelope `Secret` format

### 4.1 Versioned object layout

`Secret` becomes a version-tagged enum. The old struct is preserved verbatim as
the `V1` variant so existing stored objects deserialize and decrypt unchanged;
their `ObjectId` is unaffected (postcard encodes the same bytes for an
unchanged-shape variant only if the discriminant matches — see §5.1 for the
migration mechanic that preserves old ids).

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Secret {
    /// Legacy single-key format (today's struct). Read-only going forward:
    /// decrypts with a raw 32-byte key, never written by new code.
    V1(SecretV1),
    /// Envelope format: per-secret DK wraps the value; MK wraps the DK.
    V2(SecretV2),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretV1 { pub nonce: [u8; 12], pub ciphertext: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretV2 {
    /// The data key wrapped under a master key (provider-opaque).
    pub wrapped_dk: WrappedKey,
    /// AEAD nonce for the value encryption (DK + this nonce).
    pub nonce: [u8; 12],
    /// AEAD ciphertext of the value (plaintext len + 16-byte tag).
    pub ciphertext: Vec<u8>,
    /// AAD bound into BOTH the value AEAD and the DK wrap (§4.3).
    pub aad: SecretAad,
}

/// Additional authenticated data binding the ciphertext + wrap to context.
/// Serialized deterministically and passed as AEAD `aad`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretAad {
    pub version: u8,            // = 2; algorithm/format binding
    pub label: Option<Label>,   // the secret's confidentiality label (WS1)
}
```

### 4.2 Encrypt / decrypt

```rust
impl Secret {
    /// Envelope-encrypt: gen random DK, AEAD the value under (DK, nonce, aad),
    /// then wrap DK via the provider with the SAME aad. Always writes V2.
    pub async fn encrypt_envelope(
        plaintext: &[u8],
        provider: &dyn KeyProvider,
        aad: SecretAad,
    ) -> Result<Self>;

    /// Decrypt either variant. V2: unwrap DK via provider, AEAD-open the value.
    /// V1: caller supplies the legacy raw key through a LegacyKey provider
    /// shim (§5.1). Wrong key / tampering → Error::DecryptionFailed.
    pub async fn decrypt(&self, providers: &ProviderChain) -> Result<Vec<u8>>;
}
```

### 4.3 AEAD AAD binding (recommended; see O3)

The AAD ties a ciphertext and its wrapped DK to their context so a wrapped DK
cannot be lifted from one secret and replayed against another, and so a label
downgrade is detectable on decrypt. The minimal binding is `version`; the
recommended binding additionally includes the secret's **label** (so a stored
secret cannot be silently relabelled to a weaker label and still decrypt). The
**secret id** cannot be bound directly (it is the hash of the encoded object,
which contains the ciphertext — circular); instead the registry name or a random
per-secret `secret_uid` is the stable handle. Recommendation: bind
`{version, label}` now and reserve a `secret_uid: [u8;16]` field for id-binding;
finalize in O3.

---

## 5. Backward compatibility & migration

### 5.1 Reading legacy v1 secrets

- The current on-disk `Secret { nonce, ciphertext }` is reframed as
  `Secret::V1(SecretV1)`. To keep **existing `ObjectId`s stable**, the codec
  uses a serde representation where the historical layout deserializes into
  `V1` without a discriminant byte shift: implemented as a custom
  `Deserialize` that first attempts the legacy struct, falling back to the
  tagged enum (or, simpler and preferred, an `#[serde(untagged)]`-style probe in
  the codec layer). The chosen mechanic is verified by a golden-bytes test
  (§7): a v1 object encoded by today's code must `get` to the same `ObjectId`
  and decrypt with the legacy key.
- A **`LegacyKeyProvider`** wraps the old raw 32-byte key and only participates
  on the v1 decrypt path; it has no `wrap_dk` (returns `Unsupported`). The CLI
  injects it automatically when `--key-env` / `--key-file` resolve to a raw key,
  so `bole secret reveal` of an old secret keeps working with no flag change.
- **Lazy upgrade:** `secret rotate` and `secret rekey` (§6) re-emit any v1
  secret they touch as v2 under the active provider. There is no forced bulk
  rewrite; v1 and v2 coexist indefinitely.

### 5.2 Registries and flags

- `secrets.json` (name → object id) and `envs.json` (name → overlay id) are
  **unchanged on disk**. Envelope encryption changes only the *object the id
  points at*, not the registry shape.
- `--key-env` (default `BOLE_KEY`) and `--key-file` keep their meaning. New
  optional flags are additive: `--kms <uri>` selects `KmsKeyProvider`,
  `--key-fallback <...>` adds resolver-chain entries for reads across rotations.
- `bole secret put` now writes v2 by default; behaviour and output are otherwise
  identical. `EnvValue::Secret(ObjectId)` references resolve the same way
  regardless of v1/v2.

### 5.3 New `Object` interaction

No new `Object` enum variant is required — `Secret` already is an `Object`
variant; only its internal shape gains a version tag. (Contrast WS1, which adds
`Object::Policy`.) This keeps the WS4 pack codec untouched.

---

## 6. Env resolution, execution, and rekey

### 6.1 `resolve_overlay`

```rust
impl Repository {
    /// Resolve an overlay to a concrete environment, decrypting Secret refs.
    /// Access-checked per WS1: for each Secret entry, the secret's effective
    /// label must be readable by `accessor` (no-read-up). Plain entries are
    /// always included.
    pub async fn resolve_overlay(
        &self,
        overlay_id: &ObjectId,
        key_provider: &ProviderChain,
        accessor: &Accessor,
    ) -> Result<BTreeMap<String, String>>;
}
```

Semantics:

1. Load the `EnvOverlay`. For each `(var, value)`:
   - `Plain(s)` → insert `s`.
   - `Secret(id)` → compute the secret's **effective label** via the WS1
     `LabelRuleSet` (a secret is labelled by a rule keyed on its registry
     name/path — see O2), then call `accessor.can_read(label)`. If the actor is
     not cleared, **fail closed** (default) with `Error::AccessDenied`, naming
     the variable but **never** the value. (`--skip-unauthorized` may downgrade
     to "omit the var" — O5.)
   - If cleared, `get_secret(id, key_provider)` → decrypt → insert plaintext.
2. Non-UTF-8 secret bytes → `Error::Codec` (env values must be strings); raw
   binary secrets are out of scope for env injection.
3. Returns the fully resolved map; the caller decides whether to display
   (redacted/`--reveal`) or inject (`bole run`).

The access check **reuses WS1's `Accessor`** exactly — no parallel permission
logic. The label that gates a secret is the same lattice/rules object WS1 syncs,
so "who can resolve this overlay" is one model with path/timeline access.

### 6.2 `bole env resolve <name>` (CLI — high level; WS7 owns ergonomics)

```
bole env resolve <name> [--reveal] [--format env|json|dotenv] [key flags]
```

- Default: print `VAR=<redacted>` for secret-backed vars, plaintext for plain
  vars — safe to paste into logs.
- `--reveal`: print resolved plaintext values (requires the actor be cleared for
  every secret; uncleared → error, not silent omission, unless `--skip-...`).
- Builds the `accessor` from the caller's clearance grant (WS1) and a
  `ProviderChain` from the key flags, then calls `resolve_overlay`.

### 6.3 `bole run --env <name> -- <cmd> ...`

```
bole run --env <name> [--inherit|--clean] [key flags] -- <cmd> [args...]
```

- Resolves the overlay (access-checked), builds the child process environment,
  spawns `<cmd>`, forwards stdio, and exits with the child's status.
- `--clean` starts from an empty environment + resolved overlay (default may be
  `--inherit`: parent env overlaid with resolved vars — O5).
- Secrets exist only in the child's environment block; nothing is written to
  disk and values are never logged. The redaction default of `env resolve` does
  not apply here (the child legitimately needs the real values), but bole's own
  output (errors, `--verbose`) must redact.

### 6.4 `bole secret rekey` — master-key rotation

```
bole secret rekey [--all | <name>...] --from <old key flags> --to <new key flags>
```

- For each targeted secret: load the object; if v2, `unwrap_dk` with the **old**
  provider to recover the DK, then `wrap_dk` with the **new** provider, and
  store a new `SecretV2` with the same `nonce`/`ciphertext`/`aad` but a new
  `wrapped_dk`. The **value AEAD is never touched** — only the small DK wrap is
  recomputed.
- v1 secrets encountered by `rekey --all` are upgraded to v2 (DK generated, value
  re-encrypted once) — the one case that does touch the value, and only for
  legacy upgrade.
- **What rekey rotates:** the master key (the DK wraps). **What it does NOT
  rotate:** the data keys, the value ciphertext, or the plaintext — those are
  unchanged, which is the whole point (O(secrets) cheap wraps, not O(bytes)
  re-encryption).
- Because a rekeyed secret is a *new object* (new `wrapped_dk` ⇒ new
  `ObjectId`), the registry entry (`secrets.json`) is repointed, and overlays
  referencing the old id must be repointed too. Two options (O4): (a) rekey
  rewrites referencing overlays in-place; (b) keep the old object readable via
  the fallback chain until GC. Recommendation: (a) for registry-named secrets,
  with old objects left collectible by WS4 GC.

> **Compromised-key caveat:** rekey defends against *master*-key rotation
> hygiene. If a DK or plaintext was exposed, rekey does **not** help — the value
> ciphertext is unchanged; you must `secret rotate` (new value, new DK). This is
> stated explicitly so the operator does not over-trust rekey.

---

## 7. Testing strategy

- **Round-trip (v2):** `encrypt_envelope` then `decrypt` returns the plaintext,
  for each provider (env, file, and a fake KMS client).
- **Wrong key fails:** unwrap with a provider holding a different MK →
  `Error::DecryptionFailed`; mirrors the existing
  `get_secret_wrong_key_returns_err` test.
- **Rekey preserves plaintext:** encrypt under MK-A, `rekey` to MK-B, decrypt
  under MK-B → identical plaintext; assert the `nonce` and `ciphertext` bytes are
  **unchanged** and only `wrapped_dk` differs; assert MK-A can no longer unwrap
  the new object.
- **AAD binding:** lifting a `wrapped_dk` from secret X into secret Y's object
  fails to decrypt; relabelling a secret (changing `SecretAad.label`) fails to
  decrypt (locks the O3 decision once made).
- **Legacy compat (golden bytes):** a `Secret` object serialized by today's code
  loads as `V1`, keeps the **same `ObjectId`**, and decrypts via
  `LegacyKeyProvider`; `secret rotate`/`rekey` upgrades it to v2.
- **resolve_overlay access checks:** an accessor cleared for a secret's label
  gets the plaintext; an uncleared accessor gets `Error::AccessDenied` and the
  value never appears in the error or output. Plain vars always present.
- **Provider chain:** a repo with mixed v1 + v2-under-MK-A + v2-under-MK-B
  resolves fully when the chain holds all three.
- **`bole run`:** child process observes injected vars (`--clean` excludes
  parent env; `--inherit` includes it); child exit status is propagated; bole's
  own logs contain no secret values.
- **`env resolve` redaction:** default redacts secret-backed vars; `--reveal`
  shows them only when cleared.
- **Property:** N random secrets, rekey across K master keys in sequence, every
  decrypt still yields the original plaintext.

---

## 8. Open questions (need the maintainer's call)

- **O1 — Master key sourcing & rotation cadence.** Is the MK ever held by bole
  (env/file providers) in production, or is KMS the only sanctioned path for
  real deployments with env/file reserved for dev/CI? And is there a *required*
  rotation cadence (e.g. rekey on a timer) or purely on-demand?
  *Recommendation: env/file for dev, KMS for prod; on-demand rekey, document a
  suggested cadence, do not enforce.*

- **O2 — How a secret gets its WS1 label.** A secret object is content-addressed
  and has no path. Options: (a) label by **registry name** via a
  `LabelRule::Secret { pattern, label }` (a new rule kind in WS1 — needs WS1
  buy-in); (b) label by the overlay/var that references it; (c) carry the label
  inside `SecretAad` (self-labelling, but then the label is set at encrypt time
  and rules can't relabel). *Recommendation: (a) add a secret-name rule kind to
  WS1's `LabelRuleSet`; mirror it into `SecretAad.label` for AAD binding.*

- **O3 — AAD contents / DK-to-secret-id binding.** Bind `{version}` only,
  `{version, label}` (recommended), or add a stable random `secret_uid` to bind
  the DK to a specific secret identity (defeating wrap-replay even within the
  same label)? *Recommendation: `{version, label}` now, reserve `secret_uid`.*

- **O4 — Rekey and overlay references.** Rekey changes a secret's `ObjectId`, so
  overlays/registry entries pointing at it go stale. Auto-rewrite referencing
  overlays, or keep old objects readable via fallback until GC?
  *Recommendation: auto-repoint registry-named secrets and their overlays; leave
  old objects for WS4 GC.* (Alternatively, make the registry name — not the id —
  the stable handle inside overlays, decoupling references from rekey; larger
  change.)

- **O5 — `resolve_overlay` failure & `run` env baseline.** On an uncleared
  secret, fail-closed (default, recommended) vs omit-the-var with
  `--skip-unauthorized`? And does `bole run` default to `--inherit` (parent env +
  overlay) or `--clean`? *Recommendation: fail-closed; `--inherit` default.*

- **O6 — DK granularity: per-secret vs per-repo.** Per-secret DK (chosen) gives
  blast-radius isolation and lets `rotate` re-key one value, at the cost of one
  wrap per secret. A per-repo DK would make rekey a single wrap but couple all
  secrets to one DK (a DK leak exposes everything). *Recommendation: per-secret
  DK; revisit only if wrap cost on huge secret sets proves material.*

- **O7 — KMS client surface.** What is the minimal `KmsClient` trait bole ships
  (just `encrypt`/`decrypt` of ≤4KB blobs?), and which one reference backend
  (AWS KMS, Vault Transit, PKCS#11) ships behind a feature flag vs is left to
  third parties? *Recommendation: `encrypt`/`decrypt` only; ship one reference
  adapter behind a feature, document the trait for others.*
