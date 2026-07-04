// bole-3xj5
//! `GET /v1/timelines` and `GET /v1/timelines/{name}`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut timelines = Vec::new();
    for name in state.repo.refs.list("")? {
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

pub async fn get_one(
    State(state): State<AppState>,
    Path(name): Path<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ref_name = bole::RefName::new(name).map_err(|_| ApiError::bad_request("invalid ref name"))?;
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
