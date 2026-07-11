// bole-3xj5
//! `GET /v1/snapshots/{id}` (ACL-filtered metadata) and
//! `GET /v1/snapshots/{id}/blob?path=` (raw bytes for a visible path).

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
// bole-rvyl
use crate::extract::{ApiPath, ApiQuery};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_metadata(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let oid: bole::ObjectId = id.parse()?; // ParseObjectIdError -> 400
    let filtered = state
        .repo
        .get_snapshot_filtered(oid, &auth.accessor)
        .await?
        .ok_or_else(|| ApiError::not_found("no such snapshot"))?;
    let visible: HashMap<String, String> = filtered
        .visible_paths
        .iter()
        .map(|(p, blob)| (p.clone(), blob.to_string()))
        .collect();
    Ok(Json(json!({
        "id": filtered.id.to_string(),
        "author": filtered.author,
        "created_at": filtered.created_at,
        "message": filtered.message,
        "parents": filtered.parents.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        "visible_paths": visible,
    })))
}

#[derive(serde::Deserialize)]
pub struct BlobQuery {
    pub path: String,
}

pub async fn get_blob(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiQuery(q): ApiQuery<BlobQuery>,
    auth: RequestAuth,
) -> Result<Response, ApiError> {
    let oid: bole::ObjectId = id.parse()?;
    let filtered = state
        .repo
        .get_snapshot_filtered(oid, &auth.accessor)
        .await?
        .ok_or_else(|| ApiError::not_found("no such snapshot"))?;
    // Only paths the accessor may read are present; an unknown/hidden path is 404.
    let blob_id = filtered
        .visible_paths
        .get(&q.path)
        .ok_or_else(|| ApiError::not_found("no such path in snapshot"))?;
    match state.repo.objects.get(blob_id).await? {
        Some(bole::Object::Blob(blob)) => {
            Ok(([(CONTENT_TYPE, "application/octet-stream")], Body::from(blob.data)).into_response())
        }
        _ => Err(ApiError::internal("path did not resolve to a blob")),
    }
}
