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
