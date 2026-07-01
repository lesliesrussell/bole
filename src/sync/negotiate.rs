// bole-cy6
//! have/want negotiation: the missing-object-closure walk.
//!
//! The receiver's `have` set is every id it stores. The sender walks the object
//! graph from the `want` ref targets and prunes any subtree whose root the
//! receiver already has — sound because content-addressing guarantees that
//! having object X means having X's entire reachable closure (X embeds its
//! children's ids). So one `have` exchange suffices for correctness.

use std::collections::HashSet;

use crate::acl::policy_object::PolicyObject;
use crate::error::Result;
use crate::object::{EnvValue, Object, ObjectId};
use crate::repo::Repository;

// bole-cy6
/// The receiver's `have` set: every object id it currently stores.
pub async fn have_set(repo: &Repository) -> Result<HashSet<ObjectId>> {
    Ok(repo.objects.list().await?.into_iter().collect())
}

// bole-cy6
/// The outbound reference edges of an object (WS4 §6.2 GC edges + WS1 policy).
pub fn child_edges(obj: &Object) -> Vec<ObjectId> {
    match obj {
        Object::Snapshot(s) => {
            let mut v = Vec::with_capacity(1 + s.parents.len());
            v.push(s.root);
            v.extend(s.parents.iter().copied());
            v
        }
        Object::Tree(t) => t.entries.values().map(|e| e.id).collect(),
        Object::EnvOverlay(o) => o
            .entries
            .values()
            .filter_map(|v| match v {
                EnvValue::Secret(id) => Some(*id),
                EnvValue::Plain(_) => None,
            })
            .collect(),
        Object::Policy(PolicyObject::Root(r)) => {
            let mut v = vec![r.lattice, r.rules];
            if let Some(parent) = r.parent {
                v.push(parent);
            }
            v
        }
        // Blob, Secret, SecretV2, and non-Root policy payloads are leaves.
        _ => Vec::new(),
    }
}

// bole-cy6
/// The exact set of objects in `src` reachable from `wants` that the receiver
/// (`have`) lacks. Pruning on `have` skips whole already-present subtrees.
pub async fn missing_closure(
    src: &Repository,
    wants: &[ObjectId],
    have: &HashSet<ObjectId>,
) -> Result<Vec<ObjectId>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = wants.to_vec();
    while let Some(id) = stack.pop() {
        if have.contains(&id) || !seen.insert(id) {
            continue;
        }
        if let Some(obj) = src.objects.get(&id).await? {
            out.push(id);
            stack.extend(child_edges(&obj));
        }
    }
    Ok(out)
}
