// bole-p8u
//! # bole
//!
//! A next-generation version-control library built on content-addressed
//! storage.  Every piece of data — files, directory trees, history nodes,
//! secrets, and environment bundles — is stored as a BLAKE3-addressed object
//! and retrieved by its [`ObjectId`].  This design makes deduplication,
//! integrity checking, and structural sharing automatic.
//!
//! ## Core concepts
//!
//! | Type | Role |
//! |------|------|
//! | [`ObjectId`] | 32-byte BLAKE3 content address, the fundamental key |
//! | [`ObjectStore`] | Façade over a [`StorageBackend`] for typed object I/O |
//! | [`Snapshot`] | Immutable DAG node linking a root [`Tree`] to its parents |
//! | [`Timeline`] | A named, mutable pointer that advances through the snapshot DAG |
//! | [`Tag`] | A named, fixed pointer to a specific snapshot |
//! | [`Repository`] | Unified handle bundling object store, ref store, and ACL store |
//!
//! ## Storage backends
//!
//! `bole` ships two backends: [`MemoryBackend`] for ephemeral use (tests,
//! short-lived operations) and [`DiskBackend`] for persistent storage on the
//! local filesystem.  Both implement [`StorageBackend`] so application code
//! can be backend-agnostic.
//!
//! ## Access control
//!
//! Path and timeline access is governed by [`Accessor`] credentials checked
//! against [`PathAcl`] and [`TimelineAcl`] rules stored in the repository's
//! [`AclStore`].  Operations that require ACL checks accept an `&Accessor`
//! parameter; internal operations that must bypass user-level checks use
//! [`Accessor::privileged`].

// bole-49r
// bole-a7c
// bole-s5y
pub mod error;
pub mod object;
pub mod refs;
pub mod store;
// bole-1vi
pub mod repo;
// bole-mhs
pub mod acl;
// bole-eup
pub mod collab;
// bole-9mz
pub mod crypto;
// bole-cy6
pub mod sync;
// bole-eean
pub mod audit;
// bole-lkv3
pub mod board;
pub use board::{verify_post, BoardSigner, Post};
// bole-ub3h
pub mod reporecord;
pub use reporecord::{verify_repo, RepoRecord, RepoSigner};
// bole-lkv3
pub use repo::board::BOARD_PREFIX;
// bole-060a
pub mod pr;
pub use pr::{verify_comment, verify_proposal, ChangeProposal, ProposalSigner, ReviewComment};
// bole-xwqv
pub use repo::pr::PROPOSALS_PREFIX;
// bole-ooxm
pub use repo::pr::ProposalMerge;
pub use audit::{AuditDecision, AuditEvent, AuditSink};
pub use crypto::key_provider::{KeyProvider, LocalKeyProvider, ProviderChain, WrappedKey};
pub use acl::{
    Accessor, AclStore, CapabilityTrace, ClearanceEval, PathAcl, PathRole, Permission, SecretAcl,
    TimelineAcl, TimelineRole,
};
// bole-0tp
pub use acl::authority::{
    reconcile, verify_chain, PolicyResolution, PolicySigner, PolicyVerdict, RootSignature,
    SignatureStore, TrustAnchor, TrustStore,
};
// bole-fz1
pub use acl::attestation::{
    count_valid_approvals, verify_attestation, Approver, ApproverRegistry, Attestation,
    AttestationSigner, SignedApprovalHook,
};
// bole-ehx
pub use acl::policy_object::HookSpec;
// bole-au0t
pub use acl::policy_object::{PolicyRoot, POLICY_ROOT_REF};
// bole-eup
pub use collab::{fingerprint, key_hex, verify_profile, CollabObject, CollabSigner, Key, Profile,
    // bole-2zq
    verify_edge, TrustEdge, TrustKind,
    // bole-jtf
    verify_relay_challenge,
    // bole-su8
    RelayPin,
    // bole-n9fx
    search_ball,
    // bole-3q5g
    MAX_SEARCH_HOPS, MIN_SEARCH_TERM_LEN,
};
// bole-su8
pub use repo::collab::COLLAB_RELAYS_PREFIX;
// bole-obb
pub use collab::trust::TrustHop;
// bole-jom
pub use collab::discovery::{rank_strangers, StrangerHit};
// bole-yc9x
pub use collab::discovery::rank_strangers_multi;

pub(crate) mod codec;

pub use error::{Error, Result};

// bole-q5rm
/// Generate a fresh random 32-byte ed25519 seed — the private half of a brand
/// new account/identity. Feed it to [`RepoSigner::from_seed`] /
/// [`CollabSigner::from_seed`]. The `bole account create` CLI writes this to a
/// key file; it is the only secret a user needs to own repos on a hub.
pub fn generate_seed() -> [u8; 32] {
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}
// bole-qj8
pub use object::{Blob, EntryKind, Object, ObjectId, ParseObjectIdError, Snapshot, Tree, TreeEntry};
// bole-hto
pub use object::{EnvOverlay, EnvValue, Secret};
// bole-9mz
pub use object::{MultiRecipientSecret, SecretAad, SecretV2};
// bole-wmu
pub use refs::{
    backend::RefBackend,
    disk::DiskRefBackend,
    memory::MemoryRefBackend,
    Ref, RefName, RefStore, Tag, Timeline, TimelinePolicy,
};
pub use store::{
    backend::StorageBackend,
    disk::DiskBackend,
    memory::MemoryBackend,
    ObjectStore,
};
// bole-1vi
pub use repo::{copy_objects, materialize::materialize, Repository};
// bole-18p
pub use repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_REMOTES_PREFIX, COLLAB_SCOPED_PREFIX};
// bole-581
pub use repo::collab::QueryHit;
// bole-k93a
pub use repo::collab::{ProfileBundle, TimelineView};
// bole-6bd
pub use repo::git_projection::project_to_git;
// bole-9by
pub use repo::{AccessExplanation, Decision, FilteredSnapshot, MergeCheck};
// bole-9lj
pub use repo::merge::{MergeConflict, MergeResult};
// bole-l0i
pub use repo::workspace::WorkspaceView;
// bole-uxt
pub use repo::ephemeral::{build_tree, diff_paths, snapshot_paths, DiskWorkspace, EphemeralWorkspace, PathDiff, Workspace, IGNORE_FILE};
// bole-g7i
pub use sync::collab::{collab_adverts, serve_collab};
// bole-x5u
pub use sync::collab::collab_pull;
// bole-63b
pub use sync::collab::collab_fetch_transient;
// bole-8lm
pub use sync::collab::serve_collab_tcp_once;
// bole-yc9x
pub use sync::collab::{collab_fetch_authenticated, query_relay_set};
// bole-dxlj
pub use sync::collab::{collab_search, collab_search_authenticated};

// bole-q5rm
#[cfg(test)]
mod seed_tests {
    use super::*;

    #[test]
    fn generate_seed_is_random_and_usable() {
        let a = generate_seed();
        let b = generate_seed();
        assert_ne!(a, b, "two fresh seeds must differ");
        // A generated seed yields a valid, stable account id.
        let key1 = RepoSigner::from_seed(a).public_key();
        let key2 = RepoSigner::from_seed(a).public_key();
        assert_eq!(key1, key2, "same seed -> same account id");
        assert_eq!(key1.len(), 32);
    }
}
