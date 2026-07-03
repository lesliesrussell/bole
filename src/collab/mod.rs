// bole-eup
//! Collaboration substrate: signed, content-addressed identity and trust
//! objects (`Profile`, `TrustEdge`), a typed depth-bounded trust graph, and a
//! trust-graph-local discovery index. See
//! `docs/superpowers/specs/2026-07-03-ws8a-collaboration-substrate-design.md`.

pub mod object;
// bole-18p
pub mod discovery;
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
pub use trust::{TrustGraph, VouchSuggestion};
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
