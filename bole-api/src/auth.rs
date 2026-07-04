// bole-3xj5
//! Request authentication: extract a `Principal` from HTTP headers, then build
//! the ACL `Accessor` via bole's existing `authn::accessor_for`. This is the
//! only new authorization logic — everything downstream is the same ACL checks
//! the CLI and sync path use.

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use bole::sync::authn::{accessor_for, Principal};

use crate::error::ApiError;
use crate::state::AppState;

/// The resolved identity + capability for a request.
pub struct RequestAuth {
    pub accessor: bole::Accessor,
    pub principal: Principal,
    pub actor: Option<String>,
}

impl FromRequestParts<AppState> for RequestAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let principal = resolve_principal(parts, state)?;
        let actor = state.config.actors.actor_for(&principal).map(str::to_string);
        let accessor = accessor_for(&state.repo.acls, &state.config.actors, &principal)
            .map_err(ApiError::from)?;
        Ok(RequestAuth { accessor, principal, actor })
    }
}

/// Resolves the request's `Principal` from its headers. Order: bearer token,
/// then signed request (Task 4), then trusted mTLS proxy header (Task 5), else
/// anonymous. Task 3 implements only bearer + anonymous; the other arms are
/// added by their tasks.
fn resolve_principal(parts: &Parts, _state: &AppState) -> Result<Principal, ApiError> {
    if let Some(auth) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        let value = auth.to_str().map_err(|_| ApiError::bad_request("non-ascii Authorization header"))?;
        if let Some(token) = value.strip_prefix("Bearer ") {
            return Ok(Principal::Token(token.to_string()));
        }
    }
    Ok(Principal::Anonymous)
}

/// The `ConnectInfo` peer address, used by the mTLS proxy-header arm (Task 5).
/// Extracted here so the signature is stable; unused until Task 5.
#[allow(dead_code)]
pub(crate) fn peer_addr(parts: &Parts) -> Option<std::net::SocketAddr> {
    parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>().map(|ci| ci.0)
}

/// A human label for a principal variant (for the debug route / logging).
pub fn principal_kind(p: &Principal) -> &'static str {
    match p {
        Principal::SshKey(_) => "SshKey",
        Principal::Token(_) => "Token",
        Principal::Mtls(_) => "Mtls",
        Principal::Anonymous => "Anonymous",
    }
}
