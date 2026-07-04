// bole-3xj5
//! Route table.

use axum::routing::get;
use axum::Router;

use crate::handlers;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/status", get(handlers::status::get_status))
        // bole-3xj5
        .route("/v1/timelines", get(handlers::timelines::list))
        .route("/v1/timelines/{name}", get(handlers::timelines::get_one))
        // bole-3xj5
        .route("/v1/snapshots/{id}", get(handlers::snapshots::get_metadata))
        .route("/v1/snapshots/{id}/blob", get(handlers::snapshots::get_blob))
        .with_state(state)
}

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
