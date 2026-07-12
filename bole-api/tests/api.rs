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

// bole-rvyl
/// Every error path speaks the JSON envelope — including axum's own defaults:
/// unmatched routes, wrong methods, and extractor rejections must not return
/// bare text/empty bodies a JSON client can't parse.
#[tokio::test]
async fn unmatched_route_and_extractor_errors_use_envelope() {
    let (_dir, state) = state_with_temp_repo().await;
    let snap = seed_snapshot_and_timeline(&state.repo).await;
    let app = build_router(state);

    // Unmatched route → 404 envelope.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "not_found");

    // Wrong method on a matched route → 405 envelope.
    let resp = app
        .clone()
        .oneshot(Request::builder().method("POST").uri("/v1/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "method_not_allowed");

    // Missing required query param (Query extractor rejection) → 400 envelope.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/snapshots/{snap}/blob"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "bad_request");
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
/// The scheme parse is part of the same contract: an Authorization header with
/// an unrecognized scheme (or no scheme separator) is a presented credential
/// and must 401, and scheme names are case-insensitive per RFC 7235 — a
/// spec-compliant `bearer` client must reach the bearer arm, not silently
/// downgrade to anonymous.
#[tokio::test]
async fn unrecognized_or_case_variant_auth_scheme_handled() {
    let (_dir, state) = state_with_temp_repo().await;
    let cfg = AuthConfig::parse("[tokens]\n\"t-secret\" = \"alice\"\n").unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = build_router(state);

    // Unknown scheme → 401 (previously: silent anonymous 200).
    for header in ["Basic dXNlcjpwdw==", "Bearer"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/timelines")
                    .header("authorization", header)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "header {header:?} must 401");
    }

    // Lowercase scheme + unknown token → 401 proves it entered the bearer arm
    // (a silent-anonymous fallthrough would have returned 200).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/timelines")
                .header("authorization", "bearer t-wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Lowercase scheme + known token → authenticated request succeeds.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/timelines")
                .header("authorization", "bearer t-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
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

// bole-6lzk
/// The signed arm must not be a keyId enumeration oracle: unknown keyId, bad
/// signature, and stale date must all produce the SAME generic 401 body.
#[tokio::test]
async fn signed_auth_failures_are_indistinguishable() {
    use ed25519_dalek::SigningKey;
    let (_dir, state) = state_with_temp_repo().await;
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pubkey_hex = hex::encode(signing.verifying_key().to_bytes());
    let cfg = AuthConfig::parse(&format!(
        "[keys]\n\"k1\" = {{ pubkey = \"{pubkey_hex}\", actor = \"carol\" }}\n"
    ))
    .unwrap();
    let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
    let app = bole_api::router::debug_auth_router(state);

    let (date, sig) = sign_get(&signing, "/debug/whoami");
    let cases = [
        // Unknown keyId, otherwise-valid signature and date.
        (format!("Signature keyId=\"nope\",sig=\"{sig}\""), date.clone()),
        // Known keyId, corrupted signature.
        (format!("Signature keyId=\"k1\",sig=\"{}\"", "0".repeat(128)), date.clone()),
        // Known keyId, valid-format signature, stale date.
        (format!("Signature keyId=\"k1\",sig=\"{sig}\""), "1000000000".to_string()),
    ];
    let mut bodies = Vec::new();
    for (auth_header, d) in cases {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/debug/whoami")
                    .header("authorization", auth_header)
                    .header("x-bole-date", d)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        bodies.push(body_json(resp).await);
    }
    assert_eq!(bodies[0], bodies[1], "unknown-keyId vs bad-sig bodies must match");
    assert_eq!(bodies[1], bodies[2], "bad-sig vs stale-date bodies must match");
}

// bole-6lzk
/// A dual-stack listener reports IPv4 peers as IPv4-mapped IPv6
/// (::ffff:a.b.c.d); the trusted-proxy check must compare normalized IPs, not
/// strings, or the mTLS arm silently stops working behind such a listener.
#[tokio::test]
async fn mtls_trusted_proxy_matches_ipv4_mapped_ipv6_peer() {
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
        "[::ffff:127.0.0.1]",
    );
    let json = body_json(app.oneshot(req).await.unwrap()).await;
    assert_eq!(json["principal"], "Mtls");
    assert_eq!(json["actor"], "bob");
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

// bole-e78l
/// M2: `refs/collab/scoped/**` is not general-serve material — its names and
/// target ids must not enumerate through the timelines endpoints for ANY
/// caller (unlabeled refs default to the lattice bottom, so without a
/// structural gate an anonymous caller would see them).
#[tokio::test]
async fn scoped_collab_refs_hidden_from_timelines_endpoints() {
    let (_dir, state) = state_with_temp_repo().await;
    seed_snapshot_and_timeline(&state.repo).await;
    // Pin a scoped collab tag (a future capability-scoped object).
    let id = state.repo.objects.put_blob(axum::body::Bytes::from("scoped")).await.unwrap();
    let scoped = bole::RefName::new("refs/collab/scoped/profile/x").unwrap();
    let mut tx = state.repo.refs.transaction();
    tx.set(scoped, bole::Ref::Tag(bole::refs::Tag { target: id, created_at: 0, message: None }));
    tx.commit().unwrap();

    let app = build_router(state);

    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/v1/timelines").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let json = body_json(resp).await;
    let names: Vec<&str> = json["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.iter().all(|n| !n.starts_with("refs/collab/scoped/")),
        "scoped collab refs leaked via /v1/timelines: {names:?}"
    );

    // Point lookup: hidden means 404, indistinguishable from absent.
    let resp2 = app
        .oneshot(
            Request::builder()
                .uri("/v1/timelines/refs/collab/scoped/profile/x")
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

// bole-i8zl
/// Extractor rejections must preserve their real HTTP status, not flatten to
/// 400. A Path arity mismatch (MissingPathParams) is a 500-class server error;
/// a Query deserialize failure is a genuine 400. Both must use the envelope,
/// and neither may leak serde-internal detail in the message.
mod extractor_status {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use bole_api::extract::{ApiPath, ApiQuery};

    #[derive(serde::Deserialize)]
    struct Q {
        #[allow(dead_code)]
        path: String,
    }

    async fn wants_path(ApiPath(_): ApiPath<String>) -> &'static str {
        "ok"
    }
    async fn wants_query(ApiQuery(_): ApiQuery<Q>) -> &'static str {
        "ok"
    }

    #[tokio::test]
    async fn path_arity_mismatch_is_500_envelope() {
        // Route has no path param, but the handler asks for one → a 500-class
        // Path rejection (WrongNumberOfParameters / MissingPathParams).
        let app: Router = Router::new().route("/noparam", get(wants_path));
        let resp = app
            .oneshot(Request::builder().uri("/noparam").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(resp).await;
        assert_eq!(json["error"]["code"], "internal");
        // Generic message — no axum/serde internal detail leaked.
        let msg = json["error"]["message"].as_str().unwrap();
        assert!(!msg.contains("MissingPathParams") && !msg.to_lowercase().contains("deserialize"), "leaked detail: {msg}");
    }

    #[tokio::test]
    async fn query_deserialize_failure_is_400_envelope() {
        let app: Router = Router::new().route("/q", get(wants_query));
        // Missing required `path` query param → Query rejection (400).
        let resp = app
            .oneshot(Request::builder().uri("/q").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = body_json(resp).await;
        assert_eq!(json["error"]["code"], "bad_request");
        let msg = json["error"]["message"].as_str().unwrap();
        assert!(!msg.to_lowercase().contains("deserialize") && !msg.contains("missing field"), "leaked detail: {msg}");
    }
}

// bole-wyx7
/// Characterization: credential resolution is strict-precedence, not
/// fall-through. The first presented credential class decides — an
/// Authorization header is resolved (or rejected) on its own and never falls
/// through to the x-bole-client-subject mTLS arm.
mod auth_precedence {
    use super::*;

    /// A mapped bearer token wins even when a trusted-proxy mTLS subject is
    /// also present.
    #[tokio::test]
    async fn authorization_wins_over_trusted_proxy_mtls() {
        let (_dir, state) = state_with_temp_repo().await;
        let cfg = AuthConfig::parse(
            "[tokens]\n\"t-secret\" = \"alice\"\n[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n",
        )
        .unwrap();
        let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
        let app = bole_api::router::debug_auth_router(state);
        let req = with_peer(
            Request::builder()
                .uri("/debug/whoami")
                .header("authorization", "Bearer t-secret")
                .header("x-bole-client-subject", "CN=bob")
                .body(Body::empty())
                .unwrap(),
            "127.0.0.1",
        );
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        assert_eq!(json["principal"], "Token");
        assert_eq!(json["actor"], "alice");
    }

    /// An unrecognized Authorization scheme is a 401 and does NOT fall through
    /// to a valid mTLS subject — strict precedence, no silent downgrade.
    #[tokio::test]
    async fn bad_authorization_does_not_fall_through_to_mtls() {
        let (_dir, state) = state_with_temp_repo().await;
        let cfg = AuthConfig::parse(
            "[mtls]\n\"CN=bob\" = \"bob\"\n[proxy]\ntrusted = [\"127.0.0.1\"]\n",
        )
        .unwrap();
        let state = AppState { repo: state.repo.clone(), config: Arc::new(cfg) };
        let app = bole_api::router::debug_auth_router(state);
        let req = with_peer(
            Request::builder()
                .uri("/debug/whoami")
                .header("authorization", "Basic dXNlcjpwdw==")
                .header("x-bole-client-subject", "CN=bob")
                .body(Body::empty())
                .unwrap(),
            "127.0.0.1",
        );
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

// bole-4cnv
#[tokio::test]
async fn proposals_list_and_get_with_comments() {
    use bole::pr::ProposalSigner;
    let (_dir, state) = state_with_temp_repo().await;
    let signer = ProposalSigner::from_seed([42u8; 32]);
    let pid = state
        .repo
        .publish_proposal(&signer.sign_proposal("feature/x", "release/1.0", "Add x", 7))
        .await
        .unwrap();
    state
        .repo
        .add_comment(&signer.sign_comment(pid, "looks good", true, 8))
        .await
        .unwrap();
    let app = build_router(state);

    // List.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/v1/proposals").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let rows = json["proposals"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], pid.to_string());
    assert_eq!(rows[0]["from"], "feature/x");
    assert_eq!(rows[0]["into"], "release/1.0");

    // Get one, with its comment thread.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(format!("/v1/proposals/{pid}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["title"], "Add x");
    let comments = json["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["body"], "looks good");
    assert_eq!(comments[0]["resolves"], true);

    // Unknown id -> 404 envelope.
    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/proposals/{}", "00".repeat(32))).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], "not_found");
}

// bole-x23l
#[tokio::test]
async fn profile_bundle_endpoint_lists_repos() {
    use bole::{CollabSigner, reporecord::RepoSigner};
    let (_dir, state) = state_with_temp_repo().await;
    let seed = [77u8; 32];
    let me = CollabSigner::from_seed(seed);
    let repo_signer = RepoSigner::from_seed(seed);
    let key_hex = bole::key_hex(&me.public_key());
    state.repo.publish_profile(&me.sign_profile("Me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
    state.repo.publish_repo(&repo_signer.sign_repo("grove", "the hub", 1)).await.unwrap();
    state.repo.publish_repo(&repo_signer.sign_repo("dotfiles", "config", 1)).await.unwrap();
    let app = build_router(state);

    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/profiles/{key_hex}/bundle")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let repos: Vec<&str> = json["repos"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(repos, vec!["dotfiles", "grove"], "bundle lists the owner's repos");
    assert_eq!(json["repos"][0]["description"], "config");
}

// bole-p0lo
#[tokio::test]
async fn board_endpoint_returns_threaded_posts() {
    use bole::board::BoardSigner;
    let (_dir, state) = state_with_temp_repo().await;
    let signer = BoardSigner::from_seed([55u8; 32]);
    let root = state.repo.publish_post(&signer.sign_post("general", "hi", None, 1)).await.unwrap();
    state.repo.publish_post(&signer.sign_post("general", "reply", Some(root), 2)).await.unwrap();
    let app = build_router(state);

    let resp = app
        .oneshot(Request::builder().uri("/v1/boards/general").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["board"], "general");
    let posts = json["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    assert!(posts.iter().any(|p| p["body"] == "hi" && p["parent"].is_null()));
    assert!(posts.iter().any(|p| p["body"] == "reply" && p["parent"] == root.to_string()));
}

// bole-jgjt
#[tokio::test]
async fn profile_bundle_endpoint_contract_and_acl() {
    use bole::CollabSigner;
    let (_dir, state) = state_with_temp_repo().await;
    let me = CollabSigner::from_seed([71u8; 32]);
    let peer = CollabSigner::from_seed([72u8; 32]);
    let key_hex = bole::key_hex(&me.public_key());

    // Own identity: profile + a follow out-edge + a public and a protected timeline.
    state.repo.publish_profile(&me.sign_profile("Me".into(), "hi".into(), vec![], vec![], 1)).await.unwrap();
    state.repo.publish_edge(&me.sign_edge(peer.public_key(), bole::TrustKind::Follow, None, 1)).await.unwrap();
    let snap = seed_snapshot_and_timeline(&state.repo).await; // public "main"
    state.repo.acls.set_timeline_acl(bole::TimelineAcl { pattern: "secret/**".into() }).unwrap();
    state.repo.refs.create_timeline(
        bole::RefName::new("secret/x").unwrap(), snap, bole::TimelinePolicy::Unrestricted, 0, "persistent".into(), None,
    ).unwrap();

    let app = build_router(state);

    // Anonymous caller: bundle present; protected timeline filtered out.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(format!("/v1/profiles/{key_hex}/bundle")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["is_local"], true);
    assert_eq!(json["profile"]["display_name"], "Me");
    assert!(json["trust"]["edges"].as_array().unwrap().iter().any(|e| e["kind"] == "follow"));
    let names: Vec<&str> = json["timelines"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"main"), "public timeline present: {names:?}");
    assert!(!names.contains(&"secret/x"), "protected timeline must not leak to an anonymous caller: {names:?}");

    // Unknown key: null/empty contract, is_local:false.
    let ghost = "d3".repeat(32);
    let resp2 = app
        .oneshot(Request::builder().uri(format!("/v1/profiles/{ghost}/bundle")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let j2 = body_json(resp2).await;
    assert_eq!(j2["is_local"], false);
    assert_eq!(j2["profile"], serde_json::Value::Null);
    assert!(j2["trust"]["edges"].as_array().unwrap().is_empty());
    assert!(j2["timelines"].as_array().unwrap().is_empty());
}

// bole-wy0f
#[tokio::test]
async fn users_repos_endpoint_lists_owner_repos() {
    use bole::reporecord::RepoSigner;
    let (_dir, state) = state_with_temp_repo().await;
    let signer = RepoSigner::from_seed([88u8; 32]);
    let key_hex = bole::key_hex(&signer.public_key());
    state.repo.publish_repo(&signer.sign_repo("grove", "the hub", 1)).await.unwrap();
    state.repo.publish_repo(&signer.sign_repo("dotfiles", "config", 1)).await.unwrap();
    let app = build_router(state);

    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/users/{key_hex}/repos")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let names: Vec<&str> = json["repos"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["dotfiles", "grove"], "lists the owner's repos, sorted");
    assert_eq!(json["repos"][0]["description"], "config");
    assert_eq!(json["repos"][0]["owner"], key_hex);
    assert_eq!(json["owner"], key_hex);
}

// bole-wy0f
#[tokio::test]
async fn users_repos_endpoint_empty_for_unknown_key() {
    let (_dir, state) = state_with_temp_repo().await;
    let key_hex = "ab".repeat(32);
    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/users/{key_hex}/repos")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["repos"].as_array().unwrap().is_empty(), "no repos for an unknown key");
}

// bole-wy0f
#[tokio::test]
async fn repo_endpoint_returns_one_record_or_404() {
    use bole::reporecord::RepoSigner;
    let (_dir, state) = state_with_temp_repo().await;
    let signer = RepoSigner::from_seed([89u8; 32]);
    let key_hex = bole::key_hex(&signer.public_key());
    state.repo.publish_repo(&signer.sign_repo("grove", "the hub", 3)).await.unwrap();
    let app = build_router(state);

    let ok = app
        .clone()
        .oneshot(Request::builder().uri(format!("/v1/repos/{key_hex}/grove")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let json = body_json(ok).await;
    assert_eq!(json["name"], "grove");
    assert_eq!(json["description"], "the hub");
    assert_eq!(json["owner"], key_hex);
    assert_eq!(json["seq"], 3);

    let missing = app
        .oneshot(Request::builder().uri(format!("/v1/repos/{key_hex}/nope")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND, "unknown repo is a 404");
}

// bole-wy0f
#[tokio::test]
async fn repo_endpoint_rejects_bad_key() {
    let (_dir, state) = state_with_temp_repo().await;
    let app = build_router(state);
    let resp = app
        .oneshot(Request::builder().uri("/v1/repos/not-hex/grove").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "a non-hex owner is a 400");
}
