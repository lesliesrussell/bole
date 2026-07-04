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

// bole-3xj5
#[tokio::test]
#[ignore = "needs snapshots route (Task 7)"]
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
