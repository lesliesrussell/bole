// bole-p8u
//! Core object model for the bole content-addressed store.
//!
//! Every piece of data in bole is represented as one of the types exported
//! from this module and stored by its [`ObjectId`] — a 32-byte BLAKE3 hash
//! of its serialised form.  The top-level [`Object`] enum is the single type
//! written to and read from a [`crate::store::ObjectStore`].
//!
//! The object taxonomy:
//! - [`Blob`] — raw bytes (file contents, configuration values, etc.)
//! - [`Tree`] — a sorted map of names to child objects, forming a directory
//! - [`Snapshot`] — a point-in-time record linking a root tree to its parents
//! - [`Secret`] — a ChaCha20-Poly1305-encrypted typed object
//! - [`EnvOverlay`] — a typed bundle of environment variable values

// bole-dq0
pub mod blob;
pub mod id;
pub mod snapshot;
pub mod tree;
// bole-hto
pub mod env;
pub mod secret;

pub use blob::Blob;
pub use id::{ObjectId, ParseObjectIdError};
pub use snapshot::Snapshot;
pub use tree::{EntryKind, Tree, TreeEntry};
// bole-hto
pub use env::{EnvOverlay, EnvValue};
pub use secret::Secret;
// bole-9mz
pub use secret::{MultiRecipientSecret, SecretAad, SecretV2};

use serde::{Deserialize, Serialize};
// bole-fo2
use crate::acl::policy_object::PolicyObject;
// bole-eup
use crate::collab::CollabObject;

// bole-p8u
/// The tagged union of every object type that the store can persist.
///
/// Callers typically work with the concrete variants directly; `Object` exists
/// so the store can serialise and deserialise any object through a single code
/// path and verify content-addressing uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Object {
    // bole-p8u
    /// A raw byte payload.
    Blob(Blob),
    // bole-p8u
    /// A sorted directory of named child objects.
    Tree(Tree),
    // bole-p8u
    /// An immutable snapshot node in the history DAG.
    Snapshot(Snapshot),
    // bole-hto
    // bole-p8u
    /// An encrypted opaque value.
    Secret(Secret),
    // bole-p8u
    /// A typed bundle of environment variable values.
    EnvOverlay(EnvOverlay),
    // bole-fo2
    /// A content-addressed access-policy payload (lattice, rules, grant, or root).
    Policy(PolicyObject),
    // bole-9mz
    /// An envelope-encrypted secret (per-secret data key wrapped by a master key).
    SecretV2(SecretV2),
    // bole-amy
    /// An envelope-encrypted secret whose data key is wrapped per-recipient, so
    /// each actor decrypts with their own master key (no shared master key).
    MultiRecipientSecret(MultiRecipientSecret),
    // bole-eup
    /// A signed, content-addressed collaboration object (profile or trust edge).
    Collab(CollabObject),
    // bole-060a
    /// A signed change proposal (a PR: merge one timeline into another).
    ChangeProposal(crate::pr::ChangeProposal),
    // bole-t290
    /// A signed comment in a change proposal's review thread.
    ReviewComment(crate::pr::ReviewComment),
}
