// bole-eup
//! Collaboration substrate: signed, content-addressed identity and trust
//! objects (`Profile`, `TrustEdge`), a typed depth-bounded trust graph, and a
//! trust-graph-local discovery index. See
//! `docs/superpowers/specs/2026-07-03-ws8a-collaboration-substrate-design.md`.

pub mod object;
// bole-18p
pub mod discovery;
// bole-su8
mod relay;
pub use relay::RelayPin;
// bole-p6j
pub mod trust;
// bole-t7c
pub mod naming;
// bole-0ms
pub mod alias;

pub use object::{
    verify_profile, CollabObject, CollabSigner, Profile,
    // bole-2zq
    verify_edge, TrustEdge, TrustKind,
};
// bole-p6j
// bole-obb
pub use trust::{TrustGraph, TrustHop, VouchSuggestion};
// bole-t7c
pub use naming::{Namer, PetnameResolution};
// bole-0ms
pub use alias::{verify_alias, AliasResolver, AliasStatus};
// bole-3nk
pub use discovery::{gather, DiscoveryResult, Index, PublicObjectSource};

/// The canonical identity of a collaboration participant: an Ed25519 public key.
/// Petnames and DNS aliases are non-authoritative labels *for* a `Key`.
pub type Key = [u8; 32];

/// A stable, human-copyable fingerprint for a key (BLAKE3 hex of the key bytes).
pub fn fingerprint(key: &Key) -> String {
    blake3::hash(key).to_hex().to_string()
}

// bole-gp0
/// The raw 64-char lowercase hex of a key's bytes — the same canonical form the
/// CLI displays. This is what a user copies and searches for, so discovery
/// matches against it (never the BLAKE3 `fingerprint`, which is never shown).
pub fn key_hex(key: &Key) -> String {
    let mut s = String::with_capacity(64);
    for b in key {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
