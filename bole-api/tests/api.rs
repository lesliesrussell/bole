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
