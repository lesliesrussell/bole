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
