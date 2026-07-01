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
// bole-9mz
pub mod crypto;
pub use crypto::key_provider::{KeyProvider, LocalKeyProvider, ProviderChain, WrappedKey};
pub use acl::{
    Accessor, AclStore, PathAcl, PathRole, Permission, SecretAcl, TimelineAcl, TimelineRole,
};

pub(crate) mod codec;

pub use error::{Error, Result};
// bole-qj8
pub use object::{Blob, EntryKind, Object, ObjectId, ParseObjectIdError, Snapshot, Tree, TreeEntry};
// bole-hto
pub use object::{EnvOverlay, EnvValue, Secret};
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
// bole-6bd
pub use repo::git_projection::project_to_git;
// bole-9by
pub use repo::{FilteredSnapshot, MergeCheck};
// bole-9lj
pub use repo::merge::{MergeConflict, MergeResult};
// bole-l0i
pub use repo::workspace::WorkspaceView;
// bole-uxt
pub use repo::ephemeral::{build_tree, diff_paths, snapshot_paths, DiskWorkspace, EphemeralWorkspace, PathDiff, Workspace};
