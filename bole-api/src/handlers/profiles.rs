// bole-3xj5
//! `GET /v1/profiles/{key}` — a published Profile by 64-hex collab key.

use axum::extract::State;
// bole-rvyl
use crate::extract::ApiPath;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_profile(
    State(state): State<AppState>,
    ApiPath(key_hex): ApiPath<String>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let raw = hex::decode(&key_hex).map_err(|_| ApiError::bad_request("key must be 64 hex chars"))?;
    let key: bole::collab::Key = raw
        .try_into()
        .map_err(|_| ApiError::bad_request("key must be 32 bytes (64 hex)"))?;
    let profile = state
        .repo
        .profile(&key)
        .await?
        .ok_or_else(|| ApiError::not_found("no profile for key"))?;
    if !bole::verify_profile(&profile) {
        return Err(ApiError::not_found("profile failed verification"));
    }
    // Real `Profile` fields (src/collab/object.rs): key, display_name, bio,
    // endpoints, dns_aliases, seq, sig. `key` and `sig` are raw bytes, so they
    // are hex-encoded for a clean JSON representation.
    Ok(Json(json!({
        "key": bole::key_hex(&profile.key),
        "display_name": profile.display_name,
        "bio": profile.bio,
        "endpoints": profile.endpoints,
        "dns_aliases": profile.dns_aliases,
        "seq": profile.seq,
        "sig": hex::encode(&profile.sig),
    })))
}
