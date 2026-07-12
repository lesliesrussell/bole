// bole-3xj5
//! `GET /v1/profiles/{key}` — a published Profile by 64-hex collab key.
//! bole-jgjt: `GET /v1/profiles/{key}/bundle` — the aggregated hub view.

use axum::extract::State;
// bole-rvyl
use crate::extract::ApiPath;
use axum::Json;
use serde_json::json;

use crate::auth::RequestAuth;
use crate::error::ApiError;
use crate::state::AppState;

// bole-fmvq
/// `GET /v1/users` — the public user directory: everyone who has published a
/// profile or announced a repo on this hub, with their display name and repo
/// count. Powers Grove's browsable landing page.
pub async fn list_users(
    State(state): State<AppState>,
    _auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let users = state.repo.hub_users().await?;
    let rows: Vec<_> = users
        .iter()
        .map(|u| json!({
            "key": bole::key_hex(&u.key),
            "display_name": u.display_name,
            "repo_count": u.repo_count,
        }))
        .collect();
    Ok(Json(json!({ "users": rows })))
}

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

// bole-jgjt
/// `GET /v1/profiles/{key}/bundle` — the aggregated hub view of a dev key
/// (verified identity + own trust out-edges + their timelines), the landing-page
/// primitive Grove renders. Same stable JSON contract as `bole profile bundle`:
/// `key` and `is_local` always present; `profile` is `null` (never omitted) when
/// absent; `trust.edges` and `timelines` are `[]` when empty. Timelines are
/// filtered by the caller's accessor — an ACL-protected timeline the caller
/// cannot read never appears (the `bole-e78l` serve-path invariant).
pub async fn get_bundle(
    State(state): State<AppState>,
    ApiPath(key_hex): ApiPath<String>,
    auth: RequestAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let raw = hex::decode(&key_hex).map_err(|_| ApiError::bad_request("key must be 64 hex chars"))?;
    let key: bole::collab::Key = raw
        .try_into()
        .map_err(|_| ApiError::bad_request("key must be 32 bytes (64 hex)"))?;
    let b = state.repo.profile_bundle(&key, &auth.accessor).await?;

    let profile = match &b.profile {
        Some(p) => json!({
            "key": bole::key_hex(&p.key),
            "display_name": p.display_name,
            "bio": p.bio,
            "endpoints": p.endpoints,
            "dns_aliases": p.dns_aliases,
            "seq": p.seq,
        }),
        None => serde_json::Value::Null,
    };
    let edges: Vec<_> = b
        .edges
        .iter()
        .map(|e| json!({
            "to": bole::key_hex(&e.to_key),
            "kind": match e.kind {
                bole::TrustKind::Follow => "follow",
                bole::TrustKind::Vouch => "vouch",
                bole::TrustKind::Review => "review",
            },
            "petname": e.petname,
            "seq": e.seq,
        }))
        .collect();
    let timelines: Vec<_> = b
        .timelines
        .iter()
        .map(|t| json!({
            "name": t.name,
            "head": t.head.to_string(),
            "author": t.author,
            "created_at": t.created_at,
        }))
        .collect();
    // bole-x23l: the owner's announced repos (RepoRecords), for the hub view.
    let repos: Vec<_> = b
        .repos
        .iter()
        .map(|r| json!({
            "name": r.name,
            "description": r.description,
            "owner": bole::key_hex(&r.owner),
            "seq": r.seq,
        }))
        .collect();
    Ok(Json(json!({
        "key": bole::key_hex(&b.key),
        "is_local": b.is_local,
        "profile": profile,
        "trust": { "edges": edges },
        "timelines": timelines,
        "repos": repos,
    })))
}
