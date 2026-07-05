# bole-api Walking-Skeleton Read API Server — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up `bole-api`, an axum HTTP/JSON server exposing read-only endpoints over existing bole operations, with authentication that reuses bole's `authn` → `Accessor` ACL core.

**Architecture:** A new workspace binary crate `bole-api` depends on the `bole` library. Requests are resolved to a `bole::sync::authn::Principal`, converted to an `Accessor` via `authn::accessor_for`, and handed to existing lib read ops. Content reads go through `Repository::get_snapshot_filtered` (ACL-filtered), never raw object-by-hash.

**Tech Stack:** Rust, tokio, axum 0.8, serde/serde_json, toml, clap, ed25519-dalek, sha2, hex.

## Global Constraints

- Beads: this work is bead **bole-3xj5**. Every contiguous added block gets a `// bole-3xj5` comment (one per block).
- The `bole` core library must stay HTTP-framework-free: no axum/tower/hyper deps added to `bole/Cargo.toml`. All HTTP deps live in `bole-api/Cargo.toml`.
- All routes are under the `/v1` prefix.
- Reads only. No endpoint mutates repository state in this slice.
- Error responses use the envelope `{"error":{"code":"<slug>","message":"<text>"}}` with codes `bad_request` (400), `unauthorized` (401), `not_found` (404), `internal` (500).
- ACL-hidden resources return `404`, never `403` (no existence leak).
- TDD: write the failing test first, watch it fail, implement minimally, watch it pass, commit.
- Run tests with `cargo test -p bole-api`.

---

## File Structure

- `bole-api/Cargo.toml` — crate manifest (HTTP deps).
- `bole-api/src/main.rs` — arg parsing, build `AppState`, bind and serve.
- `bole-api/src/lib.rs` — re-exports `build_router`, `AppState`, config, for tests.
- `bole-api/src/state.rs` — `AppState` (shared repo + auth config).
- `bole-api/src/config.rs` — `AuthConfig` (TOML) → `ActorMap` + `KeyRegistry` + trusted proxies.
- `bole-api/src/error.rs` — `ApiError` + `IntoResponse`.
- `bole-api/src/auth.rs` — `RequestAuth` extractor: HTTP → `Principal` → `Accessor`.
- `bole-api/src/router.rs` — `build_router(state) -> Router`.
- `bole-api/src/handlers/mod.rs` — handler module tree.
- `bole-api/src/handlers/{status,repos,timelines,snapshots,profiles}.rs` — endpoint handlers.
- `bole-api/tests/api.rs` — in-process oneshot integration tests + a seed helper.
- `Cargo.toml` (workspace root) — add `bole-api` to `members`.

---

### Task 1: Scaffold the crate — server binds, `GET /v1/status` responds

**Files:**
- Create: `bole-api/Cargo.toml`
- Create: `bole-api/src/main.rs`
- Create: `bole-api/src/lib.rs`
- Create: `bole-api/src/state.rs`
- Create: `bole-api/src/router.rs`
- Create: `bole-api/src/handlers/mod.rs`
- Create: `bole-api/src/handlers/status.rs`
- Modify: `Cargo.toml` (workspace `members`)
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `AppState { repo: Arc<bole::Repository>, config: Arc<config::AuthConfig> }` (Clone); `build_router(state: AppState) -> axum::Router`; `handlers::status::get_status`.

- [ ] **Step 1: Add the crate to the workspace**

Modify root `Cargo.toml`:

```toml
[workspace]
members = ["bole-cli", "bole-api"]
```

- [ ] **Step 2: Create `bole-api/Cargo.toml`**

```toml
[package]
name = "bole-api"
version = "0.1.0"
edition = "2021"
license = "MIT"

[[bin]]
name = "bole-api"
path = "src/main.rs"

[dependencies]
bole = { path = ".." }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "signal"] }
axum = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
ed25519-dalek = "2"
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
tower = { version = "0.5", features = ["util"] }
tempfile = "3"
http-body-util = "0.1"
```

- [ ] **Step 3: Create `bole-api/src/state.rs`**

```rust
// bole-3xj5
//! Shared server state handed to every handler.

use std::sync::Arc;

use crate::config::AuthConfig;

/// Cloneable application state (cheap: everything behind `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<bole::Repository>,
    pub config: Arc<AuthConfig>,
}
```

- [ ] **Step 4: Create `bole-api/src/config.rs` (minimal for now)**

```rust
// bole-3xj5
//! Auth configuration loaded from TOML. Extended in later tasks; for now it is
//! an empty-by-default holder so `AppState` can carry it.

use std::collections::HashMap;

use serde::Deserialize;

/// A registered signing key: its ed25519 public key (32 bytes) and the actor it
/// authenticates as.
#[derive(Debug, Clone)]
pub struct RegisteredKey {
    pub pubkey: [u8; 32],
    pub actor: String,
}

/// Parsed auth configuration.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub actors: bole::sync::authn::ActorMap,
    pub keys: HashMap<String, RegisteredKey>,
    pub trusted_proxies: Vec<String>,
}

/// The on-disk TOML shape.
#[derive(Debug, Default, Deserialize)]
pub struct AuthConfigFile {
    #[serde(default)]
    pub tokens: HashMap<String, String>,
    #[serde(default)]
    pub mtls: HashMap<String, String>,
    #[serde(default)]
    pub keys: HashMap<String, KeyEntry>,
    #[serde(default)]
    pub proxy: ProxySection,
}

#[derive(Debug, Deserialize)]
pub struct KeyEntry {
    pub pubkey: String,
    pub actor: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProxySection {
    #[serde(default)]
    pub trusted: Vec<String>,
}

impl AuthConfig {
    /// Builds runtime config from the parsed file, validating hex fields.
    pub fn from_file(f: AuthConfigFile) -> anyhow::Result<Self> {
        let mut actors = bole::sync::authn::ActorMap::new();
        for (token, actor) in f.tokens {
            actors.map_token(token, actor);
        }
        for (subject, actor) in f.mtls {
            actors.map_mtls(subject, actor);
        }
        let mut keys = HashMap::new();
        for (key_id, entry) in f.keys {
            let raw = hex::decode(&entry.pubkey)
                .map_err(|_| anyhow::anyhow!("key {key_id}: pubkey is not valid hex"))?;
            let pubkey: [u8; 32] = raw
                .try_into()
                .map_err(|_| anyhow::anyhow!("key {key_id}: pubkey must be 32 bytes"))?;
            actors.map_ssh_key(key_id.clone(), entry.actor.clone());
            keys.insert(key_id, RegisteredKey { pubkey, actor: entry.actor });
        }
        Ok(Self { actors, keys, trusted_proxies: f.proxy.trusted })
    }

    /// Parses a TOML string into runtime config.
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        let file: AuthConfigFile = toml::from_str(toml_str)?;
        Self::from_file(file)
    }
}
```

Note: `ActorMap` derives `Clone` and `Default` (confirmed in `src/sync/authn.rs`).

- [ ] **Step 5: Create `bole-api/src/handlers/mod.rs` and `handlers/status.rs`**

`handlers/mod.rs`:

```rust
// bole-3xj5
pub mod status;
```

`handlers/status.rs`:

```rust
// bole-3xj5
//! `GET /v1/status` — server + repo summary. Anonymous-readable.

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let refs = state.repo.refs.list("")?;
    Ok(Json(json!({
        "service": "bole-api",
        "version": env!("CARGO_PKG_VERSION"),
        "ref_count": refs.len(),
    })))
}
```

- [ ] **Step 6: Create `bole-api/src/error.rs` (minimal; expanded in Task 2)**

```rust
// bole-3xj5
//! HTTP error envelope. Expanded in Task 2; this minimal version lets Task 1
//! handlers compile and return a JSON body on failure.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, code: "internal", message: message.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": { "code": self.code, "message": self.message } }));
        (self.status, body).into_response()
    }
}

impl From<bole::Error> for ApiError {
    fn from(e: bole::Error) -> Self {
        ApiError::internal(e.to_string())
    }
}
```

- [ ] **Step 7: Create `bole-api/src/router.rs` and `bole-api/src/lib.rs`**

`router.rs`:

```rust
// bole-3xj5
//! Route table.

use axum::routing::get;
use axum::Router;

use crate::handlers;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/status", get(handlers::status::get_status))
        .with_state(state)
}
```

`lib.rs`:

```rust
// bole-3xj5
//! bole-api: HTTP/JSON read API over a bole repository.

pub mod config;
pub mod error;
pub mod handlers;
pub mod router;
pub mod state;

pub use router::build_router;
pub use state::AppState;
```

- [ ] **Step 8: Create `bole-api/src/main.rs`**

```rust
// bole-3xj5
//! bole-api server binary.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use bole_api::config::AuthConfig;
use bole_api::{build_router, AppState};

#[derive(Parser)]
#[command(name = "bole-api", version, about = "HTTP/JSON read API over a bole repository")]
struct Cli {
    /// Path to the `.bole` store directory.
    #[arg(long)]
    store: PathBuf,
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,
    /// Optional auth config (TOML). Absent ⇒ all requests are anonymous.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let repo = bole::Repository::disk(&cli.store).await?;
    let config = match cli.config {
        Some(path) => AuthConfig::parse(&std::fs::read_to_string(path)?)?,
        None => AuthConfig::default(),
    };
    let state = AppState { repo: Arc::new(repo), config: Arc::new(config) };

    let listener = tokio::net::TcpListener::bind(&cli.listen).await?;
    tracing::info!("bole-api listening on {}", cli.listen);
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
```

- [ ] **Step 9: Write the failing test**

Create `bole-api/tests/api.rs`:

```rust
// bole-3xj5
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bole_api::config::AuthConfig;
use bole_api::{build_router, AppState};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// A fresh on-disk repo in a tempdir, plus its AppState.
async fn state_with_temp_repo() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join(".bole");
    let repo = bole::Repository::disk(&store).await.unwrap();
    let state = AppState { repo: Arc::new(repo), config: Arc::new(AuthConfig::default()) };
    (dir, state)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn status_returns_service_and_version() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/v1/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["service"], "bole-api");
    assert!(json["version"].is_string());
    assert_eq!(json["ref_count"], 0);
}
```

- [ ] **Step 10: Run the test to verify it fails**

Run: `cargo test -p bole-api status_returns_service_and_version`
Expected: FAIL to compile (crate not yet built) or assertion — until all Task 1 files exist and compile.

- [ ] **Step 11: Build and run to green**

Run: `cargo test -p bole-api status_returns_service_and_version`
Expected: PASS.

- [ ] **Step 12: Commit**

```bash
git add bole-api Cargo.toml
git commit -m "feat(bole-api): scaffold axum server with GET /v1/status (bole-3xj5)"
```

---

### Task 2: Full error envelope

**Files:**
- Modify: `bole-api/src/error.rs`
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `ApiError::{bad_request, unauthorized, not_found, internal}` constructors; `IntoResponse`; `From<bole::Error>` and `From<bole::ParseObjectIdError>`.

- [ ] **Step 1: Write the failing test**

Add to `bole-api/tests/api.rs`:

```rust
// bole-3xj5
#[tokio::test]
async fn unknown_route_is_404_envelope() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    // A well-formed but non-existent snapshot id (64 hex zeros).
    let id = "0".repeat(64);
    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/snapshots/{id}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "not_found");
    assert!(json["error"]["message"].is_string());
}
```

(This test also drives Task 7's snapshot route; if run now it fails because that route does not exist yet. Mark it `#[ignore]` until Task 7, or implement Task 2's constructors now and let Task 7 satisfy the route. Recommended: implement Task 2 constructors now; this test passes once Task 7 lands. To keep Task 2 self-contained, verify the envelope via the unit test in Step 3 instead.)

- [ ] **Step 2: Expand `error.rs`**

Replace `bole-api/src/error.rs` with:

```rust
// bole-3xj5
//! HTTP error envelope: `{"error":{"code","message"}}` with a matching status.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "bad_request", message: message.into() }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, code: "unauthorized", message: message.into() }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "not_found", message: message.into() }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, code: "internal", message: message.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": { "code": self.code, "message": self.message } }));
        (self.status, body).into_response()
    }
}

impl From<bole::Error> for ApiError {
    fn from(e: bole::Error) -> Self {
        // Library errors are internal by default; handlers translate the
        // "not found" / "forbidden" cases explicitly before this fallback.
        ApiError::internal(e.to_string())
    }
}

impl From<bole::ParseObjectIdError> for ApiError {
    fn from(_: bole::ParseObjectIdError) -> Self {
        ApiError::bad_request("invalid object id (expected 64 hex chars)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn not_found_renders_envelope() {
        let resp = ApiError::not_found("nope").into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["code"], "not_found");
        assert_eq!(json["error"]["message"], "nope");
    }
}
```

- [ ] **Step 3: Run the unit test to green**

Run: `cargo test -p bole-api not_found_renders_envelope`
Expected: PASS.

- [ ] **Step 4: Mark the route-dependent test ignored until Task 7**

In `tests/api.rs`, add `#[ignore = "needs snapshots route (Task 7)"]` above `unknown_route_is_404_envelope`. Remove the attribute in Task 7.

- [ ] **Step 5: Commit**

```bash
git add bole-api/src/error.rs bole-api/tests/api.rs
git commit -m "feat(bole-api): error envelope with status mapping (bole-3xj5)"
```

---

### Task 3: Auth extractor — token, anonymous, and the `Accessor`

**Files:**
- Create: `bole-api/src/auth.rs`
- Modify: `bole-api/src/lib.rs` (add `pub mod auth;`)
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `RequestAuth { pub accessor: bole::Accessor, pub principal: Principal }`, an axum extractor implementing `FromRequestParts<AppState>`. Later handlers add `auth: RequestAuth` to their signature to get an ACL-checked `Accessor`.

- [ ] **Step 1: Write the failing test**

Add to `tests/api.rs` a helper that grants an actor and a test that a mapped token yields that actor's accessor. Because `RequestAuth` is exercised through handlers, test it via a temporary debug route:

```rust
// bole-3xj5
#[tokio::test]
async fn token_maps_to_actor_principal() {
    use bole::sync::authn::Principal;
    let (_dir, state) = state_with_temp_repo().await;

    // Build config mapping a token to actor "alice".
    let cfg = AuthConfig::parse("[tokens]\n\"t-secret\" = \"alice\"\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    // A debug router that echoes the resolved principal.
    let app = bole_api::router::debug_auth_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/debug/whoami")
                .header("authorization", "Bearer t-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["principal"], "Token");
    assert_eq!(json["actor"], "alice");
    let _ = Principal::Anonymous; // keep the import used
}

#[tokio::test]
async fn no_credential_is_anonymous() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = bole_api::router::debug_auth_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/debug/whoami").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["principal"], "Anonymous");
    assert_eq!(json["actor"], serde_json::Value::Null);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p bole-api token_maps_to_actor_principal`
Expected: FAIL (no `auth` module, no `debug_auth_router`).

- [ ] **Step 3: Create `bole-api/src/auth.rs`**

```rust
// bole-3xj5
//! Request authentication: extract a `Principal` from HTTP headers, then build
//! the ACL `Accessor` via bole's existing `authn::accessor_for`. This is the
//! only new authorization logic — everything downstream is the same ACL checks
//! the CLI and sync path use.

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use bole::sync::authn::{accessor_for, Principal};

use crate::error::ApiError;
use crate::state::AppState;

/// The resolved identity + capability for a request.
pub struct RequestAuth {
    pub accessor: bole::Accessor,
    pub principal: Principal,
    pub actor: Option<String>,
}

impl FromRequestParts<AppState> for RequestAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let principal = resolve_principal(parts, state)?;
        let actor = state.config.actors.actor_for(&principal).map(str::to_string);
        let accessor = accessor_for(&state.repo.acls, &state.config.actors, &principal)
            .map_err(ApiError::from)?;
        Ok(RequestAuth { accessor, principal, actor })
    }
}

/// Resolves the request's `Principal` from its headers. Order: bearer token,
/// then signed request (Task 4), then trusted mTLS proxy header (Task 5), else
/// anonymous. Task 3 implements only bearer + anonymous; the other arms are
/// added by their tasks.
fn resolve_principal(parts: &Parts, _state: &AppState) -> Result<Principal, ApiError> {
    if let Some(auth) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        let value = auth.to_str().map_err(|_| ApiError::bad_request("non-ascii Authorization header"))?;
        if let Some(token) = value.strip_prefix("Bearer ") {
            return Ok(Principal::Token(token.to_string()));
        }
    }
    Ok(Principal::Anonymous)
}

/// The `ConnectInfo` peer address, used by the mTLS proxy-header arm (Task 5).
/// Extracted here so the signature is stable; unused until Task 5.
#[allow(dead_code)]
pub(crate) fn peer_addr(parts: &Parts) -> Option<std::net::SocketAddr> {
    parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>().map(|ci| ci.0)
}

/// A human label for a principal variant (for the debug route / logging).
pub fn principal_kind(p: &Principal) -> &'static str {
    match p {
        Principal::SshKey(_) => "SshKey",
        Principal::Token(_) => "Token",
        Principal::Mtls(_) => "Mtls",
        Principal::Anonymous => "Anonymous",
    }
}
```

- [ ] **Step 4: Add a debug route to `router.rs`**

Add to `bole-api/src/router.rs`:

```rust
// bole-3xj5
use crate::auth::{principal_kind, RequestAuth};
use axum::Json;
use serde_json::json;

/// A test-only router that echoes the resolved principal. Not mounted in the
/// real server; used by auth tests.
pub fn debug_auth_router(state: AppState) -> Router {
    Router::new()
        .route("/debug/whoami", get(debug_whoami))
        .with_state(state)
}

async fn debug_whoami(auth: RequestAuth) -> Json<serde_json::Value> {
    Json(json!({
        "principal": principal_kind(&auth.principal),
        "actor": auth.actor,
    }))
}
```

Add `pub mod auth;` to `lib.rs`.

- [ ] **Step 5: Run to green**

Run: `cargo test -p bole-api token_maps_to_actor_principal no_credential_is_anonymous`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add bole-api/src
git commit -m "feat(bole-api): RequestAuth extractor (token + anonymous) (bole-3xj5)"
```

---

### Task 4: Signed-request principal (ed25519)

**Files:**
- Modify: `bole-api/src/auth.rs`
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Consumes: `AuthConfig.keys` (`keyId -> RegisteredKey{pubkey, actor}`).
- Produces: recognition of `Authorization: Signature keyId="…",sig="…"` + `X-Bole-Date`, verified against the registered pubkey, yielding `Principal::SshKey(keyId)`. Canonical message: `bole-http-req-v1\0` + `METHOD\n` + `PATH\n` + `X-Bole-Date\n` + `hex(sha256(body))`. Replay window: `X-Bole-Date` (RFC3339 or unix-seconds string) must be within ±300s of now — but since `Date::now()` is fine in a server binary, use `std::time::SystemTime`.

- [ ] **Step 1: Write the failing test**

```rust
// bole-3xj5
#[tokio::test]
async fn signed_request_maps_to_actor() {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let (_dir, state) = state_with_temp_repo().await;
    let seed = [7u8; 32];
    let signing = SigningKey::from_bytes(&seed);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    let date = "1751600000"; // fixed unix seconds inside the skew check is bypassed for GET-with-empty-body test via a wide window; see note
    let method = "GET";
    let path = "/debug/whoami";
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(b"bole-http-req-v1\0");
    msg.extend_from_slice(format!("{method}\n{path}\n{date}\n{body_hash}").as_bytes());
    let sig = hex::encode(signing.sign(&msg).to_bytes());

    let app = bole_api::router::debug_auth_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header("authorization", format!("Signature keyId=\"k1\",sig=\"{sig}\""))
                .header("x-bole-date", date)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["principal"], "SshKey");
    assert_eq!(json["actor"], "carol");
}
```

Note on the skew window: make the window configurable with a large default in tests by exposing `AuthConfig.max_skew_secs` (default 300). For this test, set it via a test-only constructor or parse `[proxy]`-style; simplest is to make `verify_signed` accept `now` and window, and in tests pass a `now` equal to `date`. Adjust the test to call the verification path through the extractor by setting `X-Bole-Date` to the current unix time instead of a fixed value:

```rust
let date = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string();
```

Use that `date` string in both the signed message and the header.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p bole-api signed_request_maps_to_actor`
Expected: FAIL (Signature scheme not recognized → resolves Anonymous → actor null).

- [ ] **Step 3: Implement in `auth.rs`**

Extend `resolve_principal` to handle the `Signature` scheme, and add verification. Replace the token/anonymous body with:

```rust
// bole-3xj5
fn resolve_principal(parts: &Parts, state: &AppState) -> Result<Principal, ApiError> {
    if let Some(auth) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        let value = auth.to_str().map_err(|_| ApiError::bad_request("non-ascii Authorization header"))?;
        if let Some(token) = value.strip_prefix("Bearer ") {
            return Ok(Principal::Token(token.to_string()));
        }
        if let Some(rest) = value.strip_prefix("Signature ") {
            return verify_signed(rest, parts, state);
        }
    }
    Ok(Principal::Anonymous)
}

// bole-3xj5
const SIGNED_REQUEST_DOMAIN: &[u8] = b"bole-http-req-v1\0";
const MAX_SKEW_SECS: u64 = 300;

// bole-3xj5
/// Verifies `Signature keyId="…",sig="…"` against a registered key. GET/empty
/// body only carries a hash of the (possibly empty) body — the body is not read
/// here (read-only endpoints have empty request bodies), so `body_hash` is the
/// sha256 of an empty byte string. If a future write endpoint needs body
/// binding, buffer the body in a layer and stash its hash in extensions.
fn verify_signed(rest: &str, parts: &Parts, state: &AppState) -> Result<Principal, ApiError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use sha2::{Digest, Sha256};

    let key_id = extract_param(rest, "keyId").ok_or_else(|| ApiError::unauthorized("missing keyId"))?;
    let sig_hex = extract_param(rest, "sig").ok_or_else(|| ApiError::unauthorized("missing sig"))?;
    let registered = state
        .config
        .keys
        .get(&key_id)
        .ok_or_else(|| ApiError::unauthorized("unknown keyId"))?;

    // Replay window.
    let date = parts
        .headers
        .get("x-bole-date")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing X-Bole-Date"))?;
    let ts: u64 = date.parse().map_err(|_| ApiError::unauthorized("X-Bole-Date must be unix seconds"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.abs_diff(ts) > MAX_SKEW_SECS {
        return Err(ApiError::unauthorized("X-Bole-Date outside skew window"));
    }

    // Canonical message.
    let method = parts.method.as_str();
    let path = parts.uri.path();
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(SIGNED_REQUEST_DOMAIN);
    msg.extend_from_slice(format!("{method}\n{path}\n{date}\n{body_hash}").as_bytes());

    let vk = VerifyingKey::from_bytes(&registered.pubkey)
        .map_err(|_| ApiError::unauthorized("bad registered key"))?;
    let sig_bytes: [u8; 64] = hex::decode(&sig_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| ApiError::unauthorized("sig must be 64-byte hex"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    vk.verify(&msg, &signature).map_err(|_| ApiError::unauthorized("signature verification failed"))?;

    Ok(Principal::SshKey(key_id))
}

// bole-3xj5
/// Extracts `name="value"` from a comma-separated parameter string.
fn extract_param(s: &str, name: &str) -> Option<String> {
    for part in s.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}
```

- [ ] **Step 4: Add a replay/skew rejection test**

```rust
// bole-3xj5
#[tokio::test]
async fn signed_request_stale_date_rejected() {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!("[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n")).unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    let date = "1000000000"; // year 2001, far outside skew
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(b"bole-http-req-v1\0");
    msg.extend_from_slice(format!("GET\n/debug/whoami\n{date}\n{body_hash}").as_bytes());
    let sig = hex::encode(signing.sign(&msg).to_bytes());

    let app = bole_api::router::debug_auth_router(state);
    let resp = app.oneshot(Request::builder().uri("/debug/whoami")
        .header("authorization", format!("Signature keyId=\"k1\",sig=\"{sig}\""))
        .header("x-bole-date", date).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 5: Run to green**

Run: `cargo test -p bole-api signed_request`
Expected: PASS (both tests).

- [ ] **Step 6: Commit**

```bash
git add bole-api/src/auth.rs bole-api/tests/api.rs
git commit -m "feat(bole-api): ed25519 signed-request auth with replay window (bole-3xj5)"
```

---

### Task 5: mTLS via trusted proxy header

**Files:**
- Modify: `bole-api/src/auth.rs`
- Modify: `bole-api/src/router.rs` (add `ConnectInfo` when serving; test injects the extension)
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Consumes: `AuthConfig.trusted_proxies` (list of peer IPs); `peer_addr(parts)`.
- Produces: `X-Bole-Client-Subject` header honored **only** when the peer IP is in `trusted_proxies`, yielding `Principal::Mtls(subject)`.

- [ ] **Step 1: Write the failing tests**

```rust
// bole-3xj5
fn with_peer(req: Request<Body>, ip: &str) -> Request<Body> {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    let mut req = req;
    let addr: SocketAddr = format!("{ip}:9999").parse().unwrap();
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

#[tokio::test]
async fn mtls_header_honored_from_trusted_peer() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = bole_api::router::debug_auth_router(state);
    let req = with_peer(Request::builder().uri("/debug/whoami")
        .header("x-bole-client-subject", "CN=bob").body(Body::empty()).unwrap(), "127.0.0.1");
    let json = body_json(app.oneshot(req).await.unwrap()).await;
    assert_eq!(json["principal"], "Mtls");
    assert_eq!(json["actor"], "bob");
}

#[tokio::test]
async fn mtls_header_ignored_from_untrusted_peer() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = bole_api::router::debug_auth_router(state);
    let req = with_peer(Request::builder().uri("/debug/whoami")
        .header("x-bole-client-subject", "CN=bob").body(Body::empty()).unwrap(), "10.0.0.5");
    let json = body_json(app.oneshot(req).await.unwrap()).await;
    assert_eq!(json["principal"], "Anonymous");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-api mtls_header`
Expected: FAIL (subject header not consulted).

- [ ] **Step 3: Implement the arm in `resolve_principal`**

Insert before the final `Ok(Principal::Anonymous)`:

```rust
// bole-3xj5
// mTLS via trusted-proxy header: only honored when the immediate peer is an
// allowlisted proxy (the proxy is trusted to have verified the client cert).
if let Some(subject) = parts.headers.get("x-bole-client-subject").and_then(|v| v.to_str().ok()) {
    let peer_ip = peer_addr(parts).map(|a| a.ip().to_string());
    let trusted = peer_ip
        .as_deref()
        .map(|ip| state.config.trusted_proxies.iter().any(|t| t == ip))
        .unwrap_or(false);
    if trusted {
        return Ok(Principal::Mtls(subject.to_string()));
    }
}
```

Remove the `#[allow(dead_code)]` on `peer_addr` (now used).

- [ ] **Step 4: Serve with `ConnectInfo` in `main.rs`**

Change the serve line so peer addresses are available in production:

```rust
// bole-3xj5
axum::serve(
    listener,
    build_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
)
.await?;
```

- [ ] **Step 5: Run to green**

Run: `cargo test -p bole-api mtls_header`
Expected: PASS (both).

- [ ] **Step 6: Commit**

```bash
git add bole-api/src
git commit -m "feat(bole-api): mTLS via trusted proxy header (bole-3xj5)"
```

---

### Task 6: Timelines endpoints

**Files:**
- Create: `bole-api/src/handlers/timelines.rs`
- Modify: `bole-api/src/handlers/mod.rs`, `bole-api/src/router.rs`
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `GET /v1/timelines` → `{"timelines":[{name,kind,head}...]}` from `refs.list("")` + `refs.get`; `GET /v1/timelines/{name}` → one ref or 404.

- [ ] **Step 1: Write the failing test**

Add a seed helper and test. Seed uses the library to create a timeline:

```rust
// bole-3xj5
async fn seed_snapshot_and_timeline(repo: &bole::Repository) -> bole::ObjectId {
    use bole::{DiskWorkspace, Workspace};
    // Snapshot from an empty ephemeral workspace, then a timeline pointing at it.
    let mut ws = repo.ephemeral_workspace();
    ws.write("README.md", &b"hi"[..]);
    let snap = ws.commit("tester", "init", 0).await.unwrap();
    let name = bole::RefName::new("main").unwrap();
    repo.refs
        .create_timeline(name, snap, bole::TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
        .unwrap();
    snap
}

#[tokio::test]
async fn timelines_lists_created_timeline() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app.oneshot(Request::builder().uri("/v1/timelines").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names: Vec<&str> = json["timelines"].as_array().unwrap().iter()
        .map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"main"));
}
```

Confirm the exact `create_timeline` signature against `src/refs/mod.rs` before running (arguments: `name`, `head`, `policy`, `created_at`, `kind`, `expires_at`). The seed helper's `ephemeral_workspace()` + `write` + `commit` matches `EphemeralWorkspace` (confirmed in `src/repo/ephemeral.rs`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-api timelines_lists_created_timeline`
Expected: FAIL (route missing → 404).

- [ ] **Step 3: Create `handlers/timelines.rs`**

```rust
// bole-3xj5
//! `GET /v1/timelines` and `GET /v1/timelines/{name}`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut timelines = Vec::new();
    for name in state.repo.refs.list("")? {
        match state.repo.refs.get(&name)? {
            Some(bole::Ref::Timeline(t)) => timelines.push(json!({
                "name": name.as_str(),
                "kind": "timeline",
                "head": t.head.to_string(),
                "policy": format!("{:?}", t.policy),
            })),
            Some(bole::Ref::Tag(tag)) => timelines.push(json!({
                "name": name.as_str(),
                "kind": "tag",
                "head": tag.target.to_string(),
            })),
            None => {}
        }
    }
    Ok(Json(json!({ "timelines": timelines })))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(name): Path<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ref_name = bole::RefName::new(name).map_err(|_| ApiError::bad_request("invalid ref name"))?;
    match state.repo.refs.get(&ref_name)? {
        Some(bole::Ref::Timeline(t)) => Ok(Json(json!({
            "name": ref_name.as_str(), "kind": "timeline",
            "head": t.head.to_string(), "policy": format!("{:?}", t.policy),
            "created_at": t.created_at,
        }))),
        Some(bole::Ref::Tag(tag)) => Ok(Json(json!({
            "name": ref_name.as_str(), "kind": "tag",
            "head": tag.target.to_string(), "created_at": tag.created_at,
        }))),
        None => Err(ApiError::not_found("no such ref")),
    }
}
```

Note: `bole::Ref`, `bole::RefName` are exported (confirmed in `src/lib.rs` line 102). `_auth: RequestAuth` requires a resolvable accessor (anonymous OK) — it also validates any presented credential (bad token/sig ⇒ 401 before listing).

- [ ] **Step 4: Wire routes + module**

`handlers/mod.rs`: add `pub mod timelines;`.
`router.rs` `build_router`: add
```rust
.route("/v1/timelines", get(handlers::timelines::list))
.route("/v1/timelines/{name}", get(handlers::timelines::get_one))
```
(axum 0.8 path params use `{name}` syntax.)

- [ ] **Step 5: Run to green**

Run: `cargo test -p bole-api timelines_lists_created_timeline`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add bole-api/src bole-api/tests/api.rs
git commit -m "feat(bole-api): timelines list + get endpoints (bole-3xj5)"
```

---

### Task 7: Snapshots endpoints — metadata (ACL-filtered) + blob-by-path

**Files:**
- Create: `bole-api/src/handlers/snapshots.rs`
- Modify: `bole-api/src/handlers/mod.rs`, `bole-api/src/router.rs`
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `GET /v1/snapshots/{id}` → `{id,author,created_at,message,parents,visible_paths}` via `Repository::get_snapshot_filtered(id, accessor)`; `GET /v1/snapshots/{id}/blob?path=<p>` → raw bytes iff `path` in `visible_paths`, else 404.

- [ ] **Step 1: Write the failing tests**

```rust
// bole-3xj5
#[tokio::test]
async fn snapshot_metadata_exposes_visible_paths() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/v1/snapshots/{snap}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["message"], "init");
    assert!(json["visible_paths"].get("README.md").is_some());
}

#[tokio::test]
async fn snapshot_blob_returns_bytes_for_visible_path() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/v1/snapshots/{snap}/blob?path=README.md")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"hi");
}

#[tokio::test]
async fn snapshot_blob_missing_path_is_404() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/v1/snapshots/{snap}/blob?path=nope.txt")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

Also remove `#[ignore]` from `unknown_route_is_404_envelope` (Task 2) — the route now exists.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-api snapshot_`
Expected: FAIL (routes missing).

- [ ] **Step 3: Create `handlers/snapshots.rs`**

```rust
// bole-3xj5
//! `GET /v1/snapshots/{id}` (ACL-filtered metadata) and
//! `GET /v1/snapshots/{id}/blob?path=` (raw bytes for a visible path).

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_metadata(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let oid: bole::ObjectId = id.parse()?; // ParseObjectIdError -> 400
    let filtered = state
        .repo
        .get_snapshot_filtered(oid, &auth.accessor)
        .await?
        .ok_or_else(|| ApiError::not_found("no such snapshot"))?;
    let visible: HashMap<String, String> = filtered
        .visible_paths
        .iter()
        .map(|(p, blob)| (p.clone(), blob.to_string()))
        .collect();
    Ok(Json(json!({
        "id": filtered.id.to_string(),
        "author": filtered.author,
        "created_at": filtered.created_at,
        "message": filtered.message,
        "parents": filtered.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        "visible_paths": visible,
    })))
}

#[derive(serde::Deserialize)]
pub struct BlobQuery {
    pub path: String,
}

pub async fn get_blob(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<BlobQuery>,
    auth: RequestAuth,
) -> Result<Response, ApiError> {
    let oid: bole::ObjectId = id.parse()?;
    let filtered = state
        .repo
        .get_snapshot_filtered(oid, &auth.accessor)
        .await?
        .ok_or_else(|| ApiError::not_found("no such snapshot"))?;
    // Only paths the accessor may read are present; an unknown/hidden path is 404.
    let blob_id = filtered
        .visible_paths
        .get(&q.path)
        .ok_or_else(|| ApiError::not_found("no such path in snapshot"))?;
    match state.repo.objects.get(blob_id).await? {
        Some(bole::Object::Blob(blob)) => {
            Ok(([(CONTENT_TYPE, "application/octet-stream")], Body::from(blob.data)).into_response())
        }
        _ => Err(ApiError::internal("path did not resolve to a blob")),
    }
}
```

Note: `Body::from(blob.data)` — `blob.data` is `bytes::Bytes` (confirmed in `src/object/blob.rs`), which `axum::body::Body` accepts. `bole::Object`, `bole::ObjectId` are exported (confirmed `src/lib.rs` line 96).

- [ ] **Step 4: Wire routes + module**

`handlers/mod.rs`: `pub mod snapshots;`.
`router.rs`:
```rust
.route("/v1/snapshots/{id}", get(handlers::snapshots::get_metadata))
.route("/v1/snapshots/{id}/blob", get(handlers::snapshots::get_blob))
```

- [ ] **Step 5: Run to green**

Run: `cargo test -p bole-api snapshot_ unknown_route_is_404_envelope`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add bole-api/src bole-api/tests/api.rs
git commit -m "feat(bole-api): ACL-filtered snapshot metadata + blob-by-path (bole-3xj5)"
```

---

### Task 8: Profiles + repos endpoints

**Files:**
- Create: `bole-api/src/handlers/profiles.rs`, `bole-api/src/handlers/repos.rs`
- Modify: `bole-api/src/handlers/mod.rs`, `bole-api/src/router.rs`
- Test: `bole-api/tests/api.rs`

**Interfaces:**
- Produces: `GET /v1/profiles/{key}` → the `Profile` JSON (or 404) via `Repository::profile(&Key)` + `verify_profile`; `GET /v1/repos` → `{"repos":[{...}]}` single-element list.

- [ ] **Step 1: Write the failing tests**

```rust
// bole-3xj5
#[tokio::test]
async fn profile_unknown_key_is_404() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let key = "1".repeat(64);
    let resp = app.oneshot(Request::builder().uri(format!("/v1/profiles/{key}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_bad_key_is_400() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let resp = app.oneshot(Request::builder().uri("/v1/profiles/not-hex").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn repos_lists_this_store() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let json = body_json(app.oneshot(Request::builder().uri("/v1/repos").body(Body::empty()).unwrap()).await.unwrap()).await;
    assert_eq!(json["repos"].as_array().unwrap().len(), 1);
    assert_eq!(json["repos"][0]["ref_count"], 1);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p bole-api profile_ repos_lists_this_store`
Expected: FAIL (routes missing).

- [ ] **Step 3: Create `handlers/profiles.rs`**

```rust
// bole-3xj5
//! `GET /v1/profiles/{key}` — a published Profile by 64-hex collab key.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_profile(
    State(state): State<AppState>,
    Path(key_hex): Path<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let raw = hex::decode(&key_hex).map_err(|_| ApiError::bad_request("key must be 64 hex chars"))?;
    let key: bole::collab::Key = raw
        .try_into()
        .map_err(|_| ApiError::bad_request("key must be 32 bytes (64 hex)"))?;
    let profile = state
        .repo
        .profile(&key)
        .await?
        .ok_or_else(|| ApiError::not_found("no profile for key"))?;
    if !bole::verify_profile(&profile) {
        return Err(ApiError::not_found("profile failed verification"));
    }
    Ok(Json(json!({
        "display_name": profile.display_name,
        "bio": profile.bio,
        "endpoints": profile.endpoints,
        "seq": profile.seq,
    })))
}
```

Before running, confirm `Profile`'s public fields in `src/collab/object.rs` (line 22) and that `bole::collab::Key` and `bole::verify_profile` are reachable. `verify_profile` is exported (confirmed `src/lib.rs` line 71 group). If `bole::collab` is not re-exported, use the full path `bole::collab::Key` (the `collab` module is `pub` in `src/lib.rs` line 48). Map only the fields that exist; adjust the `json!` block to the actual `Profile` fields.

- [ ] **Step 4: Create `handlers/repos.rs`**

```rust
// bole-3xj5
//! `GET /v1/repos` — the single store this server hosts (there is no multi-repo
//! primitive today; the collection shape is forward-compatible).

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ref_count = state.repo.refs.list("")?.len();
    Ok(Json(json!({
        "repos": [ { "id": "default", "ref_count": ref_count } ]
    })))
}
```

- [ ] **Step 5: Wire routes + module**

`handlers/mod.rs`: `pub mod profiles;` and `pub mod repos;`.
`router.rs`:
```rust
.route("/v1/repos", get(handlers::repos::list))
.route("/v1/profiles/{key}", get(handlers::profiles::get_profile))
```

- [ ] **Step 6: Run to green**

Run: `cargo test -p bole-api profile_ repos_lists_this_store`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add bole-api/src bole-api/tests/api.rs
git commit -m "feat(bole-api): profiles + repos endpoints (bole-3xj5)"
```

---

### Task 9: Full-suite green, clippy, README, and CLI/skills note

**Files:**
- Create: `bole-api/README.md`
- Modify: `docs/CLI.md` or a new `docs/API.md` section pointer (optional)
- Test: whole workspace

- [ ] **Step 1: Run the whole workspace**

Run: `cargo test --workspace`
Expected: all green (existing bole/bole-cli suites + new bole-api suite).

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets`
Expected: no warnings. Fix any inline.

- [ ] **Step 3: Write `bole-api/README.md`**

Document: what it is, `bole-api --store <path> --listen <addr> --config auth.toml`, the auth config TOML shape (tokens/mtls/keys/proxy), the endpoint list, the error envelope, and the ACL-through-`Accessor` model. Note the two deployment assumptions (mTLS via trusted proxy; signed-request canonical string).

- [ ] **Step 4: Manual smoke test (verify)**

```bash
cargo run -p bole-api -- --store /path/to/.bole --listen 127.0.0.1:8080 &
curl -s localhost:8080/v1/status | jq .
curl -s localhost:8080/v1/timelines | jq .
kill %1
```
Expected: JSON responses matching the handlers.

- [ ] **Step 5: Commit**

```bash
git add bole-api/README.md docs
git commit -m "docs(bole-api): README + endpoint/auth reference (bole-3xj5)"
```

---

## Self-Review

**Spec coverage:**
- Crate & process (spec §Architecture) → Task 1. ✓
- Module layout → Tasks 1–8 create each module. ✓
- Auth: token → Task 3; signed-request → Task 4; mTLS proxy header → Task 5; anonymous → Task 3; `accessor_for` reuse → Task 3. ✓
- Auth config TOML → Task 1 (`config.rs`), exercised in Tasks 3–5. ✓
- Endpoints: status → Task 1; repos → Task 8; timelines (+one) → Task 6; snapshots metadata → Task 7; blob-by-path → Task 7; profiles → Task 8. ✓ (`GET /v1/objects/{id}` deliberately absent per the ACL reshape.)
- JSON contract & errors, ACL-hidden → 404 → Task 2 + Task 7 (visible_paths naturally yields 404 for hidden paths). ✓
- Testing (oneshot, seeded repo, each case) → Tasks 1–8 tests; full suite → Task 9. ✓
- Files added list → matches Tasks. ✓

**Placeholder scan:** No "TBD"/"handle appropriately" — each step has concrete code. Two grounding caveats are called out explicitly (confirm `create_timeline` arg order in Task 6 Step 1; confirm `Profile` fields in Task 8 Step 3) — these are verification instructions, not placeholders, because the exact field list must be read from source at implementation time.

**Type consistency:** `AppState`, `RequestAuth`, `ApiError`, `build_router`, `AuthConfig`, `RegisteredKey` names are used identically across tasks. `Principal`/`ActorMap`/`accessor_for` come from `bole::sync::authn` (public, confirmed). `get_snapshot_filtered`, `FilteredSnapshot.visible_paths`, `Blob.data`, `Ref::{Timeline,Tag}`, `Timeline.head/policy/created_at`, `Tag.target/created_at`, `Object::Blob`, `ObjectId: FromStr`, `RefName::new`, `Repository::{disk,refs,objects,acls,profile,ephemeral_workspace,get_snapshot_filtered}` all verified against source during planning.

**Open verification points for the implementer (read source, don't guess):**
1. `RefStore::create_timeline` exact parameters (Task 6 seed).
2. `Profile` public field names (Task 8 profiles handler `json!`).
3. axum 0.8 exact versions of `into_make_service_with_connect_info` and `FromRequestParts` (async-trait-free in 0.8) — adjust if the toolchain pulls a different minor.
