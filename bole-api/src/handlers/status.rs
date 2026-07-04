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
