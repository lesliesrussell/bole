// bole-gvy
//! Converting between the on-disk work tree and the object store's tree graph.
//!
//! The tree-building, snapshot-reading, and diff logic now lives in the `bole`
//! library ([`bole::build_tree`], [`bole::snapshot_paths`], [`bole::diff_paths`])
//! so the CLI and the library's in-memory [`bole::EphemeralWorkspace`] share one
//! implementation. This module keeps only the disk-specific walk and re-exports
//! the library primitives under the names the CLI commands use.

// bole-uxt: shared core lives in the library; re-export under the CLI's names.
// bole-1kz: the disk walk itself now lives in the library as
// `bole::DiskWorkspace` (its private `collect`), the single implementation.
// The former `collect_blobs` duplicate here was removed; commands construct a
// `DiskWorkspace` instead.
pub use bole::{build_tree as build_root_tree, diff_paths as diff, snapshot_paths as snapshot_blobs};
