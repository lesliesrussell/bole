// bole-sk6
//! Atomic multi-ref transactions.
//!
//! Objects need no transaction (immutable, content-addressed), but refs are
//! mutable named pointers, so advancing a head, moving a tag, and deleting a ref
//! must commit all-or-nothing. A [`RefTransaction`] buffers operations and
//! optimistic-concurrency preconditions, then [`RefTransaction::commit`] resolves
//! them to a set of *absolute final ref values* and applies them atomically via
//! the backend (a write-ahead journal on disk; see [`super::disk`]).

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::{Ref, RefBackend, RefName, RefStore, Tag, Timeline, TimelinePolicy};

// bole-sk6
/// A buffered ref operation or precondition. Mutations resolve to absolute final
/// values at commit; preconditions are checked against current state.
#[derive(Debug, Clone)]
pub enum RefOp {
    CreateTag { name: RefName, target: ObjectId, message: Option<String>, now: u64 },
    MoveTag { name: RefName, target: ObjectId },
    CreateTimeline {
        name: RefName,
        head: ObjectId,
        policy: TimelinePolicy,
        now: u64,
        kind: String,
        expires_at: Option<u64>,
    },
    AdvanceHead { name: RefName, new_head: ObjectId },
    /// Unconditional upsert to an absolute ref value (e.g. remote-tracking refs).
    Set { name: RefName, value: Ref },
    Delete { name: RefName },
    /// CAS: the ref must currently equal `expected` (`None` = absent).
    Expect { name: RefName, expected: Option<Ref> },
    /// CAS: `name` is a timeline whose head must equal `expected_old`, then advance.
    AdvanceHeadIf { name: RefName, expected_old: ObjectId, new_head: ObjectId },
}

// bole-sk6
/// A builder that records ref operations and commits them atomically.
pub struct RefTransaction<'a> {
    store: &'a RefStore,
    ops: Vec<RefOp>,
}

impl<'a> RefTransaction<'a> {
    pub(crate) fn new(store: &'a RefStore) -> Self {
        Self { store, ops: Vec::new() }
    }

    pub fn create_tag(&mut self, name: RefName, target: ObjectId, message: Option<String>, now: u64) -> &mut Self {
        self.ops.push(RefOp::CreateTag { name, target, message, now });
        self
    }
    pub fn move_tag(&mut self, name: RefName, target: ObjectId) -> &mut Self {
        self.ops.push(RefOp::MoveTag { name, target });
        self
    }
    pub fn create_timeline(
        &mut self,
        name: RefName,
        head: ObjectId,
        policy: TimelinePolicy,
        now: u64,
        kind: String,
        expires_at: Option<u64>,
    ) -> &mut Self {
        self.ops.push(RefOp::CreateTimeline { name, head, policy, now, kind, expires_at });
        self
    }
    pub fn advance_head(&mut self, name: RefName, new_head: ObjectId) -> &mut Self {
        self.ops.push(RefOp::AdvanceHead { name, new_head });
        self
    }
    pub fn set(&mut self, name: RefName, value: Ref) -> &mut Self {
        self.ops.push(RefOp::Set { name, value });
        self
    }
    pub fn delete_ref(&mut self, name: RefName) -> &mut Self {
        self.ops.push(RefOp::Delete { name });
        self
    }
    pub fn expect(&mut self, name: RefName, expected: Option<Ref>) -> &mut Self {
        self.ops.push(RefOp::Expect { name, expected });
        self
    }
    pub fn advance_head_if(&mut self, name: RefName, expected_old: ObjectId, new_head: ObjectId) -> &mut Self {
        self.ops.push(RefOp::AdvanceHeadIf { name, expected_old, new_head });
        self
    }

    /// Validates all preconditions, resolves the ops to absolute final ref
    /// values, and applies them atomically. On any failure nothing is applied.
    pub fn commit(self) -> Result<()> {
        self.store.commit_transaction(&self.ops)
    }
}

// bole-sk6
/// Resolves buffered ops into a plan of absolute final ref values (`None` =
/// tombstone/delete), reading current state via `backend` and layering earlier
/// ops so later ops observe them. Validates existing rules and CAS preconditions;
/// any violation aborts with an error before anything is applied.
pub(crate) fn resolve(
    backend: &dyn RefBackend,
    ops: &[RefOp],
) -> Result<Vec<(RefName, Option<Ref>)>> {
    let mut overlay: BTreeMap<RefName, Option<Ref>> = BTreeMap::new();
    let current = |overlay: &BTreeMap<RefName, Option<Ref>>, name: &RefName| -> Result<Option<Ref>> {
        if let Some(v) = overlay.get(name) {
            Ok(v.clone())
        } else {
            backend.get(name)
        }
    };

    for op in ops {
        match op {
            RefOp::CreateTag { name, target, message, now } => {
                if current(&overlay, name)?.is_some() {
                    return Err(Error::Storage(format!("ref already exists: {}", name.as_str())));
                }
                overlay.insert(
                    name.clone(),
                    Some(Ref::Tag(Tag { target: *target, created_at: *now, message: message.clone() })),
                );
            }
            RefOp::MoveTag { name, target } => match current(&overlay, name)? {
                Some(Ref::Tag(mut t)) => {
                    t.target = *target;
                    overlay.insert(name.clone(), Some(Ref::Tag(t)));
                }
                Some(Ref::Timeline(_)) => {
                    return Err(Error::WrongRefKind(format!("'{}' is a timeline, not a tag", name.as_str())))
                }
                None => return Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
            },
            RefOp::CreateTimeline { name, head, policy, now, kind, expires_at } => {
                if current(&overlay, name)?.is_some() {
                    return Err(Error::Storage(format!("ref already exists: {}", name.as_str())));
                }
                overlay.insert(
                    name.clone(),
                    Some(Ref::Timeline(Timeline {
                        head: *head,
                        policy: policy.clone(),
                        created_at: *now,
                        kind: kind.clone(),
                        expires_at: *expires_at,
                    })),
                );
            }
            RefOp::AdvanceHead { name, new_head } => match current(&overlay, name)? {
                Some(Ref::Timeline(mut tl)) => {
                    tl.head = *new_head;
                    overlay.insert(name.clone(), Some(Ref::Timeline(tl)));
                }
                Some(Ref::Tag(_)) => {
                    return Err(Error::WrongRefKind(format!("'{}' is a tag, not a timeline", name.as_str())))
                }
                None => return Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
            },
            RefOp::Set { name, value } => {
                overlay.insert(name.clone(), Some(value.clone()));
            }
            RefOp::Delete { name } => {
                overlay.insert(name.clone(), None);
            }
            RefOp::Expect { name, expected } => {
                if current(&overlay, name)? != *expected {
                    return Err(Error::TransactionConflict(format!(
                        "precondition failed for '{}'",
                        name.as_str()
                    )));
                }
            }
            RefOp::AdvanceHeadIf { name, expected_old, new_head } => match current(&overlay, name)? {
                Some(Ref::Timeline(mut tl)) if tl.head == *expected_old => {
                    tl.head = *new_head;
                    overlay.insert(name.clone(), Some(Ref::Timeline(tl)));
                }
                Some(Ref::Timeline(_)) => {
                    return Err(Error::TransactionConflict(format!(
                        "head of '{}' is not the expected value",
                        name.as_str()
                    )))
                }
                Some(Ref::Tag(_)) => {
                    return Err(Error::WrongRefKind(format!("'{}' is a tag, not a timeline", name.as_str())))
                }
                None => return Err(Error::Storage(format!("ref not found: {}", name.as_str()))),
            },
        }
    }
    Ok(overlay.into_iter().collect())
}
