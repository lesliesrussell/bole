// bole-eup
//! Collaboration substrate: signed, content-addressed identity and trust
//! objects (`Profile`, `TrustEdge`), a typed depth-bounded trust graph, and a
//! trust-graph-local discovery index. See
//! `docs/superpowers/specs/2026-07-03-ws8a-collaboration-substrate-design.md`.

pub mod object;

pub use object::{
    verify_profile, CollabObject, CollabSigner, Profile,
    // bole-2zq
    verify_edge, TrustEdge, TrustKind,
};

/// The canonical identity of a collaboration participant: an Ed25519 public key.
/// Petnames and DNS aliases are non-authoritative labels *for* a `Key`.
pub type Key = [u8; 32];

/// A stable, human-copyable fingerprint for a key (BLAKE3 hex of the key bytes).
pub fn fingerprint(key: &Key) -> String {
    blake3::hash(key).to_hex().to_string()
}
