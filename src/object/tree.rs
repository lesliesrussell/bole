// bole-dq0
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// bole-p8u
/// Discriminates whether a `TreeEntry` points to a leaf blob or a nested tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryKind {
    // bole-p8u
    /// The entry is a raw byte payload (`Blob`).
    Blob,
    // bole-p8u
    /// The entry is a nested directory (`Tree`).
    Tree,
}

// bole-p8u
/// A single named entry within a `Tree`, pointing to either a blob or a subtree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeEntry {
    // bole-p8u
    /// The content address of the referenced object.
    pub id: ObjectId,
    // bole-p8u
    /// Whether the referenced object is a blob or a nested tree.
    pub kind: EntryKind,
}

// bole-p8u
/// An immutable, sorted map of names to child entries that represents a directory.
///
/// Trees are stored content-addressed: two trees with the same entries always
/// share the same `ObjectId`, enabling structural sharing across snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    // bole-p8u
    /// Sorted map from entry name to its `TreeEntry`; the sort order is deterministic
    /// so that identical directory contents always produce the same object id.
    pub entries: BTreeMap<String, TreeEntry>,
}
