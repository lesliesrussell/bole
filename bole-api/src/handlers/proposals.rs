// bole-4cnv
//! `GET /v1/proposals` and `GET /v1/proposals/{id}` — read the PR system's
//! change proposals and their review threads, so a frontend (Grove) can render
//! them. Proposals are public collaboration metadata (they name timelines, like
//! a profile names a key); they carry no per-object read ACL in v1.

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::extract::ApiPath;
use crate::state::AppState;

// bole-4cnv
/// `GET /v1/proposals` — every open change proposal (verified fail-closed).
pub async fn list(
    State(state): State<AppState>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let proposals = state.repo.list_proposals().await?;
    let rows: Vec<_> = proposals
        .iter()
        .map(|(id, p)| json!({
            "id": id.to_string(),
            "from": p.source,
            "into": p.target,
            "title": p.title,
            "author": hex::encode(p.author),
            "created_at": p.created_at,
        }))
        .collect();
    Ok(Json(json!({ "proposals": rows })))
}

// bole-4cnv
/// `GET /v1/proposals/{id}` — one proposal plus its review thread. 404 if the
/// id is not a stored, verified proposal.
pub async fn get_one(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let oid: bole::ObjectId = id.parse().map_err(|_| ApiError::bad_request("invalid proposal id"))?;
    let p = state
        .repo
        .get_proposal(&oid)
        .await?
        .ok_or_else(|| ApiError::not_found("no such proposal"))?;
    let comments = state.repo.list_comments(&oid).await?;
    let comment_rows: Vec<_> = comments
        .iter()
        .map(|(cid, c)| json!({
            "id": cid.to_string(),
            "body": c.body,
            "resolves": c.resolves,
            "author": hex::encode(c.author),
            "created_at": c.created_at,
        }))
        .collect();
    Ok(Json(json!({
        "id": oid.to_string(),
        "from": p.source,
        "into": p.target,
        "title": p.title,
        "author": hex::encode(p.author),
        "created_at": p.created_at,
        "comments": comment_rows,
    })))
}
