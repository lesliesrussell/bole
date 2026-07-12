// bole-p0lo
//! `GET /v1/boards/{board}` — read a discussion board's posts as threaded JSON,
//! so a frontend (Grove) can render it. Posts are public collaboration metadata
//! (they carry no per-object read ACL in v1); each is verified fail-closed.

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::extract::ApiPath;
use crate::state::AppState;

// bole-p0lo
/// `GET /v1/boards/{board}` — every post on `board`, with each post's `parent`
/// (the id it replies to, or `null`) so the caller can reconstruct threads.
pub async fn get_board(
    State(state): State<AppState>,
    ApiPath(board): ApiPath<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let posts = state.repo.list_posts(&board).await?;
    let rows: Vec<_> = posts
        .iter()
        .map(|(id, p)| json!({
            "id": id.to_string(),
            "body": p.body,
            "parent": p.parent.map(|x| x.to_string()),
            "author": hex::encode(p.author),
            "created_at": p.created_at,
        }))
        .collect();
    Ok(Json(json!({ "board": board, "posts": rows })))
}
