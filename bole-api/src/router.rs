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
        // bole-3xj5: catch-all so hierarchical timeline names (e.g. `leslie/private/exp`)
        // resolve — bole ref names are slash-delimited, not single-segment.
        .route("/v1/timelines/{*name}", get(handlers::timelines::get_one))
        // bole-3xj5
        .route("/v1/snapshots/{id}", get(handlers::snapshots::get_metadata))
        .route("/v1/snapshots/{id}/blob", get(handlers::snapshots::get_blob))
        // bole-3xj5
        .route("/v1/repos", get(handlers::repos::list))
        // bole-wy0f
        .route("/v1/users/{key}/repos", get(handlers::repos::list_for_owner))
        .route("/v1/repos/{owner}/{name}", get(handlers::repos::get_one))
        .route("/v1/profiles/{key}", get(handlers::profiles::get_profile))
        // bole-jgjt
        .route("/v1/profiles/{key}/bundle", get(handlers::profiles::get_bundle))
        // bole-4cnv
        .route("/v1/proposals", get(handlers::proposals::list))
        .route("/v1/proposals/{id}", get(handlers::proposals::get_one))
        // bole-p0lo
        .route("/v1/boards/{board}", get(handlers::boards::get_board))
        // bole-rvyl: axum's defaults for unmatched routes (bare 404) and wrong
        // methods (bare 405) are the only non-JSON error surfaces; every error
        // must speak the envelope.
        .fallback(unmatched)
        .method_not_allowed_fallback(wrong_method)
        .with_state(state)
}

// bole-rvyl
async fn unmatched() -> crate::error::ApiError {
    crate::error::ApiError::not_found("no such route")
}

// bole-rvyl
async fn wrong_method() -> crate::error::ApiError {
    crate::error::ApiError::method_not_allowed("method not allowed on this route")
}

// bole-3xj5
// bole-gejz: test-only surface — compiled out of the shipped lib/binary.
#[cfg(feature = "testing")]
use crate::auth::{principal_kind, RequestAuth};
#[cfg(feature = "testing")]
use axum::Json;
#[cfg(feature = "testing")]
use serde_json::json;

/// A test-only router that echoes the resolved principal. Not mounted in the
/// real server; used by auth tests (requires the `testing` feature).
#[cfg(feature = "testing")]
pub fn debug_auth_router(state: AppState) -> Router {
    Router::new()
        .route("/debug/whoami", get(debug_whoami))
        .with_state(state)
}

#[cfg(feature = "testing")]
async fn debug_whoami(auth: RequestAuth) -> Json<serde_json::Value> {
    Json(json!({
        "principal": principal_kind(&auth.principal),
        "actor": auth.actor,
    }))
}
