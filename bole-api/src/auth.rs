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
        // bole-261x
        // Contract: presented-but-unknown credentials are 401, never a silent
        // downgrade to anonymous. The lib's ActorMap maps unmapped principals
        // to anonymous (a sync-serve convention); over HTTP that would make a
        // stale or typo'd token indistinguishable from no token at all, masking
        // misconfiguration and inviting confused-deputy mistakes. Only a
        // request that presents NO credential is anonymous.
        if actor.is_none() && principal != Principal::Anonymous {
            return Err(ApiError::unauthorized("credentials do not map to a known actor"));
        }
        let accessor = accessor_for(&state.repo.acls, &state.config.actors, &principal)
            .map_err(ApiError::from)?;
        Ok(RequestAuth { accessor, principal, actor })
    }
}

/// Resolves the request's `Principal` from its headers. Order: bearer token,
/// then signed request (Task 4), then trusted mTLS proxy header (Task 5), else
/// anonymous. Task 3 implements only bearer + anonymous; the other arms are
/// added by their tasks.
// bole-3xj5
fn resolve_principal(parts: &Parts, state: &AppState) -> Result<Principal, ApiError> {
    if let Some(auth) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        let value = auth.to_str().map_err(|_| ApiError::bad_request("non-ascii Authorization header"))?;
        // bole-261x
        // An Authorization header is a presented credential: an unrecognized
        // or malformed scheme is 401, never a silent fallthrough to anonymous.
        // Scheme names compare case-insensitively (RFC 7235), so a
        // spec-compliant `bearer` client reaches the bearer arm.
        let (scheme, rest) = value
            .split_once(' ')
            .ok_or_else(|| ApiError::unauthorized("malformed Authorization header"))?;
        if scheme.eq_ignore_ascii_case("bearer") {
            return Ok(Principal::Token(rest.to_string()));
        }
        if scheme.eq_ignore_ascii_case("signature") {
            return verify_signed(rest, parts, state);
        }
        return Err(ApiError::unauthorized("unrecognized Authorization scheme"));
    }
    // bole-3xj5
    // mTLS via trusted-proxy header: only honored when the immediate peer is an
    // allowlisted proxy (the proxy is trusted to have verified the client cert).
    if let Some(subject) = parts.headers.get("x-bole-client-subject").and_then(|v| v.to_str().ok()) {
        // bole-6lzk
        // Compare normalized IpAddrs, not strings: a dual-stack listener
        // reports IPv4 peers as IPv4-mapped IPv6 (::ffff:a.b.c.d), which never
        // string-matches a "127.0.0.1" config entry. Unparseable config
        // entries are skipped (fail-closed).
        let trusted = peer_addr(parts)
            .map(|a| {
                let peer = canonical_ip(a.ip());
                state
                    .config
                    .trusted_proxies
                    .iter()
                    .filter_map(|t| t.parse::<std::net::IpAddr>().ok())
                    .any(|t| canonical_ip(t) == peer)
            })
            .unwrap_or(false);
        if trusted {
            return Ok(Principal::Mtls(subject.to_string()));
        }
    }
    Ok(Principal::Anonymous)
}

// bole-3xj5
const SIGNED_REQUEST_DOMAIN: &[u8] = b"bole-http-req-v1\0";
const MAX_SKEW_SECS: u64 = 300;

// bole-3xj5
/// Verifies `Signature keyId="…",sig="…"` against a registered key. The
/// canonical message binds the method, the full request target (path + query,
/// per bole-e333), the `X-Bole-Date`, and a hash of the body. The body is not
/// read here (read-only endpoints have empty request bodies), so `body_hash` is
/// the sha256 of an empty byte string. If a future write endpoint needs body
/// binding, buffer the body in a layer and stash its hash in extensions.
fn verify_signed(rest: &str, parts: &Parts, state: &AppState) -> Result<Principal, ApiError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use sha2::{Digest, Sha256};

    // bole-6lzk
    // Every failure in this arm returns the SAME generic 401: distinct error
    // texts would let a caller distinguish unknown-keyId from bad-signature
    // and enumerate registered key ids. For the same reason the replay window
    // is checked BEFORE the key lookup.
    let generic = || ApiError::unauthorized("signed request rejected");

    let key_id = extract_param(rest, "keyId").ok_or_else(generic)?;
    let sig_hex = extract_param(rest, "sig").ok_or_else(generic)?;

    // Replay window (before key lookup — bole-6lzk).
    let date = parts
        .headers
        .get("x-bole-date")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(generic)?;
    let ts: u64 = date.parse().map_err(|_| generic())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.abs_diff(ts) > MAX_SKEW_SECS {
        return Err(generic());
    }

    // bole-6lzk
    let registered = state.config.keys.get(&key_id).ok_or_else(generic)?;

    // Canonical message.
    let method = parts.method.as_str();
    // bole-e333: bind the full request target (path AND query) so query params
    // (e.g. `?path=`) cannot be altered in transit without breaking the
    // signature. Falls back to the bare path when there is no query.
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| parts.uri.path());
    let body_hash = hex::encode(Sha256::digest(b""));
    let mut msg = Vec::new();
    msg.extend_from_slice(SIGNED_REQUEST_DOMAIN);
    msg.extend_from_slice(format!("{method}\n{path}\n{date}\n{body_hash}").as_bytes());

    // bole-6lzk: same generic 401 on every failure (no enumeration oracle).
    let vk = VerifyingKey::from_bytes(&registered.pubkey).map_err(|_| generic())?;
    let sig_bytes: [u8; 64] = hex::decode(&sig_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(generic)?;
    let signature = Signature::from_bytes(&sig_bytes);
    vk.verify(&msg, &signature).map_err(|_| generic())?;

    Ok(Principal::SshKey(key_id))
}

// bole-3xj5
/// Extracts `name="value"` from a comma-separated parameter string.
fn extract_param(s: &str, name: &str) -> Option<String> {
    for part in s.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

/// The `ConnectInfo` peer address, used by the mTLS proxy-header arm.
pub(crate) fn peer_addr(parts: &Parts) -> Option<std::net::SocketAddr> {
    parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>().map(|ci| ci.0)
}

// bole-6lzk
/// IPv4-mapped IPv6 addresses (::ffff:a.b.c.d) normalize to their IPv4 form so
/// dual-stack peer addresses compare equal to IPv4 config entries.
fn canonical_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => v6.to_canonical(),
        v4 => v4,
    }
}

/// A human label for a principal variant (for the debug route / logging).
// bole-gejz: test-only surface, compiled out of the shipped lib/binary.
#[cfg(feature = "testing")]
pub fn principal_kind(p: &Principal) -> &'static str {
    match p {
        Principal::SshKey(_) => "SshKey",
        Principal::Token(_) => "Token",
        Principal::Mtls(_) => "Mtls",
        Principal::Anonymous => "Anonymous",
    }
}
