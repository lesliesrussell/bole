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

// bole-3xj5
async fn seed_snapshot_and_timeline(repo: &bole::Repository) -> bole::ObjectId {
    // Snapshot from an empty ephemeral workspace, then a timeline pointing at it.
    // `write`/`commit` resolve to EphemeralWorkspace's inherent methods, so the
    // `Workspace` trait doesn't need to be in scope here.
    let mut ws = repo.ephemeral_workspace();
    ws.write("README.md", &b"hi"[..]);
    let snap = ws.commit("tester", "init", 0).await.unwrap();
    let name = bole::RefName::new("main").unwrap();
    repo.refs
        .create_timeline(name, snap, bole::TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
        .unwrap();
    snap
}

// bole-3xj5
#[tokio::test]
async fn timelines_lists_created_timeline() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/v1/timelines").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names: Vec<&str> = json["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main"));
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

// bole-3xj5
#[tokio::test]
async fn token_maps_to_actor_principal() {
    // bole-3xj5
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
}

// bole-261x
/// Contract: a request that PRESENTS a bearer token which maps to no actor is
/// 401, not silently anonymous — a stale or typo'd token must surface as an
/// auth failure, never as a quiet capability downgrade.
#[tokio::test]
async fn unknown_bearer_token_is_401() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[tokens]\n\"t-secret\" = \"alice\"\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/timelines")
                .header("authorization", "Bearer t-wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "unauthorized");
}

// bole-261x
/// Same contract for the trusted-proxy mTLS header: a subject the actor map
/// does not know is 401, not anonymous.
#[tokio::test]
async fn unknown_mtls_subject_from_trusted_proxy_is_401() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = build_router(state);
    let req = with_peer(
        Request::builder()
            .uri("/v1/timelines")
            .header("x-bole-client-subject", "CN=mallory")
            .body(Body::empty())
            .unwrap(),
        "127.0.0.1",
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

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

    let date = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
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

// bole-e333
/// Signs a GET request and returns (date, sig) for the given request target
/// (path, optionally with `?query`), binding the full target into the canonical
/// message exactly as the server does.
fn sign_get(signing: &ed25519_dalek::SigningKey, target: &str) -> (String, String) {
    use ed25519_dalek::Signer;
    use sha2::{Digest, Sha256};
    let date = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(b"bole-http-req-v1\0");
    msg.extend_from_slice(format!("GET\n{target}\n{date}\n{body_hash}").as_bytes());
    (date, hex::encode(signing.sign(&msg).to_bytes()))
}

// bole-e333
/// A signature over a request target that includes a query string must verify
/// when the request is sent to that exact target — proving the query is part of
/// the signed canonical message.
#[tokio::test]
async fn signed_request_with_query_accepted_when_matching() {
    use ed25519_dalek::SigningKey;
    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    let target = "/debug/whoami?scope=public";
    let (date, sig) = sign_get(&signing, target);

    let app = bole_api::router::debug_auth_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(target)
                .header("authorization", format!("Signature keyId=\"k1\",sig=\"{sig}\""))
                .header("x-bole-date", date)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["actor"], "carol");
}

// bole-e333
/// A signature over one query must NOT verify a request whose query was altered
/// in transit — the query is bound, so tampering breaks the signature.
#[tokio::test]
async fn signed_request_tampered_query_rejected() {
    use ed25519_dalek::SigningKey;
    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    // Sign for scope=public, but send scope=admin.
    let (date, sig) = sign_get(&signing, "/debug/whoami?scope=public");

    let app = bole_api::router::debug_auth_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/debug/whoami?scope=admin")
                .header("authorization", format!("Signature keyId=\"k1\",sig=\"{sig}\""))
                .header("x-bole-date", date)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// bole-3xj5
#[tokio::test]
async fn signed_request_stale_date_rejected() {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    let date = "1000000000"; // year 2001, far outside skew
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(b"bole-http-req-v1\0");
    msg.extend_from_slice(format!("GET\n/debug/whoami\n{date}\n{body_hash}").as_bytes());
    let sig = hex::encode(signing.sign(&msg).to_bytes());

    let app = bole_api::router::debug_auth_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/debug/whoami")
                .header("authorization", format!("Signature keyId=\"k1\",sig=\"{sig}\""))
                .header("x-bole-date", date)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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

// bole-3xj5
fn with_peer(req: Request<Body>, ip: &str) -> Request<Body> {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    let mut req = req;
    let addr: SocketAddr = format!("{ip}:9999").parse().unwrap();
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

// bole-3xj5
#[tokio::test]
async fn mtls_header_honored_from_trusted_peer() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = bole_api::router::debug_auth_router(state);
    let req = with_peer(
        Request::builder()
            .uri("/debug/whoami")
            .header("x-bole-client-subject", "CN=bob")
            .body(Body::empty())
            .unwrap(),
        "127.0.0.1",
    );
    let json = body_json(app.oneshot(req).await.unwrap()).await;
    assert_eq!(json["principal"], "Mtls");
    assert_eq!(json["actor"], "bob");
}

// bole-3xj5
#[tokio::test]
async fn mtls_header_ignored_from_untrusted_peer() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = bole_api::router::debug_auth_router(state);
    let req = with_peer(
        Request::builder()
            .uri("/debug/whoami")
            .header("x-bole-client-subject", "CN=bob")
            .body(Body::empty())
            .unwrap(),
        "10.0.0.5",
    );
    let json = body_json(app.oneshot(req).await.unwrap()).await;
    assert_eq!(json["principal"], "Anonymous");
}

// bole-3xj5
#[tokio::test]
async fn snapshot_metadata_exposes_visible_paths() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/snapshots/{snap}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
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
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/snapshots/{snap}/blob?path=README.md"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"hi");
}

#[tokio::test]
async fn snapshot_blob_missing_path_is_404() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/snapshots/{snap}/blob?path=nope.txt"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// bole-3xj5
#[tokio::test]
async fn profile_unknown_key_is_404() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let key = "1".repeat(64);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/profiles/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// bole-3xj5
#[tokio::test]
async fn profile_bad_key_is_400() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/profiles/not-hex")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// bole-3xj5
/// C1 regression: an ACL-protected timeline must be invisible to an anonymous
/// caller in both `list` and `get_one` — and `get_one` must answer 404, never
/// 403, so a hidden timeline is indistinguishable from a nonexistent one.
#[tokio::test]
async fn acl_protected_timeline_is_hidden_from_anonymous() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await; // public timeline "main"

    state
        .repo
        .acls
        .set_timeline_acl(bole::TimelineAcl { pattern: "leslie/private/**".into() })
        .unwrap();
    let hidden_head = bole::ObjectId::new([9u8; 32]);
    state
        .repo
        .refs
        .create_timeline(
            bole::RefName::new("leslie/private/exp").unwrap(),
            hidden_head,
            bole::TimelinePolicy::Unrestricted,
            0,
            "persistent".into(),
            None,
        )
        .unwrap();

    let app = build_router(state);

    // Anonymous list: public timeline present, protected timeline absent.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/v1/timelines").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names: Vec<&str> = json["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main"));
    assert!(!names.contains(&"leslie/private/exp"));

    // Anonymous get_one on the protected timeline: 404, not 403.
    let resp2 = app
        .oneshot(
            Request::builder()
                .uri("/v1/timelines/leslie/private/exp")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
}

// bole-3xj5
/// I3 regression: hierarchical (slash-containing) timeline names must resolve
/// through `GET /v1/timelines/{*name}`.
#[tokio::test]
async fn hierarchical_timeline_name_resolves() {
    let (_dir, state) = state_with_temp_repo().await;
    let mut ws = state.repo.ephemeral_workspace();
    ws.write("README.md", &b"hi"[..]);
    let snap = ws.commit("tester", "init", 0).await.unwrap();
    state
        .repo
        .refs
        .create_timeline(
            bole::RefName::new("team/foo").unwrap(),
            snap,
            bole::TimelinePolicy::Unrestricted,
            0,
            "persistent".into(),
            None,
        )
        .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/v1/timelines/team/foo").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "team/foo");
    assert_eq!(json["kind"], "timeline");
}

// bole-3xj5
/// I2 (signed-request arm): a tampered-but-well-formed signature over a fresh,
/// in-window `X-Bole-Date` must be rejected with 401.
#[tokio::test]
async fn signed_request_tampered_signature_rejected() {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };

    let date = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let method = "GET";
    let path = "/debug/whoami";
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(b"bole-http-req-v1\0");
    msg.extend_from_slice(format!("{method}\n{path}\n{date}\n{body_hash}").as_bytes());
    let mut sig = hex::encode(signing.sign(&msg).to_bytes());
    // Valid hex, wrong signature: flip the last hex digit.
    let last = sig.pop().unwrap();
    sig.push(if last == '0' { '1' } else { '0' });

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
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// bole-3xj5
#[tokio::test]
async fn repos_lists_this_store() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);
    let json = body_json(
        app.oneshot(Request::builder().uri("/v1/repos").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(json["repos"].as_array().unwrap().len(), 1);
    assert_eq!(json["repos"][0]["ref_count"], 1);
}
