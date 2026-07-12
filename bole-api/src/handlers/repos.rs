// bole-3xj5
//! `GET /v1/repos` — the single store this server hosts (there is no multi-repo
//! primitive today; the collection shape is forward-compatible).
//! bole-wy0f: `GET /v1/users/{key}/repos` and `GET /v1/repos/{owner}/{name}` —
//! the announced RepoRecords a developer owns, for Grove's hub views.

use axum::extract::State;
use axum::Json;
use serde_json::json;

// bole-wy0f
use crate::extract::ApiPath;
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

// bole-wy0f
/// Parses a 64-hex collab/owner key path segment into a 32-byte key, mapping a
/// malformed value to a 400 (not a 404 or 500).
fn parse_key(key_hex: &str) -> Result<bole::collab::Key, ApiError> {
    let raw = hex::decode(key_hex).map_err(|_| ApiError::bad_request("key must be 64 hex chars"))?;
    raw.try_into().map_err(|_| ApiError::bad_request("key must be 32 bytes (64 hex)"))
}

// bole-wy0f
/// One announced repo as JSON. `owner` is hex-encoded (raw bytes otherwise).
fn repo_json(r: &bole::reporecord::RepoRecord) -> serde_json::Value {
    json!({
        "name": r.name,
        "description": r.description,
        "owner": bole::key_hex(&r.owner),
        "seq": r.seq,
    })
}

// bole-wy0f
/// `GET /v1/users/{key}/repos` — the repos owned by `key`, sorted by name.
/// Empty (never 404) for a key that has announced nothing.
pub async fn list_for_owner(
    State(state): State<AppState>,
    ApiPath(key_hex): ApiPath<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let owner = parse_key(&key_hex)?;
    let repos = state.repo.list_repos(&owner).await?;
    let rows: Vec<_> = repos.iter().map(repo_json).collect();
    Ok(Json(json!({ "owner": bole::key_hex(&owner), "repos": rows })))
}

// bole-wy0f
/// `GET /v1/repos/{owner}/{name}` — one announced repo record. 404 if the owner
/// has no repo by that name.
pub async fn get_one(
    State(state): State<AppState>,
    ApiPath((owner_hex, name)): ApiPath<(String, String)>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let owner = parse_key(&owner_hex)?;
    let rec = state
        .repo
        .get_repo(&owner, &name)
        .await?
        .ok_or_else(|| ApiError::not_found("no such repo"))?;
    Ok(Json(repo_json(&rec)))
}
