// bole-3xj5
//! `GET /v1/timelines` and `GET /v1/timelines/{name}`.

use axum::extract::State;
// bole-rvyl
use crate::extract::ApiPath;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

// bole-3xj5
pub async fn list(
    State(state): State<AppState>,
    auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut timelines = Vec::new();
    // Only enumerate refs the accessor is actually permitted to read; this
    // omits ACL-protected timelines entirely rather than exposing their
    // name/head/policy to callers who can't read them.
    // bole-e78l
    // list_refs_served additionally excludes refs/collab/scoped/** for every
    // caller (M2): unlabeled refs default to the lattice bottom, so the label
    // check alone would enumerate scoped names/ids to anonymous callers.
    for name in state.repo.list_refs_served("", &auth.accessor)? {
        match state.repo.refs.get(&name)? {
            Some(bole::Ref::Timeline(t)) => timelines.push(json!({
                "name": name.as_str(),
                "kind": "timeline",
                "head": t.head.to_string(),
                "policy": format!("{:?}", t.policy),
            })),
            Some(bole::Ref::Tag(tag)) => timelines.push(json!({
                "name": name.as_str(),
                "kind": "tag",
                "head": tag.target.to_string(),
            })),
            None => {}
        }
    }
    Ok(Json(json!({ "timelines": timelines })))
}

// bole-3xj5
pub async fn get_one(
    State(state): State<AppState>,
    ApiPath(name): ApiPath<String>,
    auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ref_name = bole::RefName::new(name).map_err(|_| ApiError::bad_request("invalid ref name"))?;
    // A timeline the accessor cannot read must be indistinguishable from one
    // that doesn't exist: return 404 (never 403) when it's hidden.
    // bole-tgr8
    // ref_served is the point-lookup twin of the list endpoint's
    // list_refs_served (same label gate, same scoped-collab exclusion) —
    // membership without the O(all-refs) scan.
    if !state.repo.ref_served(&ref_name, &auth.accessor)? {
        return Err(ApiError::not_found("no such ref"));
    }
    match state.repo.refs.get(&ref_name)? {
        Some(bole::Ref::Timeline(t)) => Ok(Json(json!({
            "name": ref_name.as_str(), "kind": "timeline",
            "head": t.head.to_string(), "policy": format!("{:?}", t.policy),
            "created_at": t.created_at,
        }))),
        Some(bole::Ref::Tag(tag)) => Ok(Json(json!({
            "name": ref_name.as_str(), "kind": "tag",
            "head": tag.target.to_string(), "created_at": tag.created_at,
        }))),
        None => Err(ApiError::not_found("no such ref")),
    }
}
