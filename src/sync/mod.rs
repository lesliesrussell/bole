// bole-cy6
//! The distributed sync engine (in-process core).
//!
//! `fetch`/`push`/`clone_from` between two `Repository` handles: negotiate the
//! missing object closure, transfer it through a real WS4 pack (self-verifying
//! receive), and reconcile refs — fetch writes remote-tracking refs, push does a
//! fast-forward-gated CAS on the peer's timeline heads via a WS4 `RefTransaction`.
//! Objects land before any ref moves, so a failed transfer leaves only orphans.
//!
//! This is the spec's `InProcessTransport` backbone; the wire codec, `Transport`
//! trait, HTTP/SSH, signed policy verification, and authn mapping are deferred to
//! bole-6qy / bole-0tp / bole-6h7.

pub mod negotiate;
// bole-6qy
pub mod wire;
pub mod transport;
pub mod session;
// bole-vih
pub mod http;
// bole-6h7
pub mod authn;

use crate::acl::{Accessor, ResourceRef};
use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::{Ref, RefName, Tag, TimelinePolicy};
use crate::repo::Repository;
use crate::store::pack::{decode_pack, PackBuilder};

// bole-cy6
/// The outcome of a single pushed ref.
#[derive(Debug, Clone, PartialEq)]
pub enum PushStatus {
    Ok,
    /// The peer's head is not the expected old value (a concurrent push won, or
    /// the update is not a fast-forward). Carries the peer's actual head.
    NonFastForward { server_head: ObjectId },
    /// The actor lacks write access, or the local ref is missing.
    Denied(String),
}

// bole-cy6
/// A per-ref push result.
#[derive(Debug, Clone)]
pub struct RefResult {
    pub name: RefName,
    pub status: PushStatus,
}

impl Repository {
    // bole-cy6
    /// Transfers the missing closure of `wants` from `src` into `self` as a WS4
    /// pack (each frame BLAKE3-verified on receipt). Returns the number landed.
    async fn sync_transfer(&self, src: &Repository, wants: &[ObjectId]) -> Result<usize> {
        let have = negotiate::have_set(self).await?;
        let missing = negotiate::missing_closure(src, wants, &have).await?;
        if missing.is_empty() {
            return Ok(0);
        }
        let mut builder = PackBuilder::new();
        for id in &missing {
            if let Some(bytes) = src.objects.get_raw(id).await? {
                builder.add(*id, bytes.to_vec());
            }
        }
        let (pack, _entries, _digest) = builder.finish()?;
        // Self-verifying receive: decode_pack checks every frame's id + digest.
        let decoded = decode_pack(&pack)?;
        for (_id, canonical) in &decoded {
            self.objects.put_raw(canonical).await?;
        }
        Ok(decoded.len())
    }

    // bole-cy6
    /// Pulls the readable ref closure from `from` into `self` and updates
    /// remote-tracking refs `refs/remotes/<remote_name>/<ref>`. Never touches
    /// local timelines. Returns the tracking refs set. `accessor` gates which of
    /// `from`'s refs are advertised (ref-granularity, WS1 `list_refs_filtered`).
    pub async fn fetch(
        &self,
        remote_name: &str,
        from: &Repository,
        accessor: &Accessor,
    ) -> Result<Vec<(RefName, ObjectId)>> {
        let adverts = advertise(from, accessor)?;
        let wants: Vec<ObjectId> = adverts.iter().map(|(_, t)| *t).collect();
        self.sync_transfer(from, &wants).await?;

        let mut tx = self.refs.transaction();
        let mut tracked = Vec::new();
        for (name, target) in &adverts {
            let tracking = RefName::new(format!("refs/remotes/{remote_name}/{}", name.as_str()))
                .map_err(|e| Error::Storage(format!("bad tracking ref name: {e}")))?;
            tx.set(tracking.clone(), Ref::Tag(Tag { target: *target, created_at: 0, message: None }));
            tracked.push((tracking, *target));
        }
        tx.commit()?;
        Ok(tracked)
    }

    // bole-cy6
    /// Pushes the given local timelines to peer `to` (named `remote_name`): lands
    /// the missing closure on `to` first, then compare-and-swaps each head via
    /// `advance_head_if`, using this repo's remote-tracking ref (what it last saw
    /// of the peer) as the expected old head — so a peer that moved since the last
    /// fetch rejects the push. Fast-forward-gated by the peer's timeline policy.
    /// On success the local tracking ref advances to the pushed head.
    pub async fn push(
        &self,
        remote_name: &str,
        to: &Repository,
        timelines: &[RefName],
        accessor: &Accessor,
    ) -> Result<Vec<RefResult>> {
        let mut results = Vec::new();
        let mut wants = Vec::new();
        // (name, local_head, expected_old, tracking_ref, local_timeline)
        let mut plan: Vec<(RefName, ObjectId, Option<ObjectId>, RefName, crate::refs::Timeline)> =
            Vec::new();

        let lattice = to.acls.lattice()?;
        let rules = to.acls.label_ruleset()?;

        for name in timelines {
            let local = match self.refs.get_timeline(name)? {
                Some(t) => t,
                None => {
                    results.push(RefResult {
                        name: name.clone(),
                        status: PushStatus::Denied("no such local timeline".into()),
                    });
                    continue;
                }
            };
            // WS1 authz: the actor must be able to write the peer timeline's label.
            let label = rules.label_for_timeline(&lattice, name.as_str());
            if !accessor.can_write(&label, ResourceRef::Timeline(name.as_str())) {
                results.push(RefResult {
                    name: name.clone(),
                    status: PushStatus::Denied("write denied on timeline".into()),
                });
                continue;
            }
            // expected_old = the head we last saw of the peer (our tracking ref).
            let tracking = RefName::new(format!("refs/remotes/{remote_name}/{}", name.as_str()))
                .map_err(|e| Error::Storage(format!("bad tracking ref name: {e}")))?;
            let expected_old = self.refs.get_tag(&tracking)?.map(|t| t.target);

            // Fast-forward gate against what we last saw (unless peer policy is
            // Unrestricted or the peer has no such timeline yet).
            if let (Some(old), Some(remote)) = (expected_old, to.refs.get_timeline(name)?) {
                let ff = to.find_common_ancestor(old, local.head).await? == Some(old);
                if !matches!(remote.policy, TimelinePolicy::Unrestricted) && !ff {
                    results.push(RefResult {
                        name: name.clone(),
                        status: PushStatus::NonFastForward { server_head: remote.head },
                    });
                    continue;
                }
            }
            wants.push(local.head);
            plan.push((name.clone(), local.head, expected_old, tracking, local));
        }

        if plan.is_empty() {
            return Ok(results);
        }

        // Objects-before-refs: land the closure on the peer before any CAS.
        to.sync_transfer(self, &wants).await?;

        let mut tx = to.refs.transaction();
        for (name, local_head, expected_old, _tracking, local_tl) in &plan {
            match expected_old {
                Some(old) => {
                    tx.advance_head_if(name.clone(), *old, *local_head);
                }
                None => {
                    tx.create_timeline(
                        name.clone(),
                        *local_head,
                        local_tl.policy.clone(),
                        local_tl.created_at,
                        local_tl.kind.clone(),
                        local_tl.expires_at,
                    );
                }
            }
        }
        match tx.commit() {
            Ok(()) => {
                // Advance our tracking refs to the newly-pushed heads.
                let mut track_tx = self.refs.transaction();
                for (_name, local_head, _old, tracking, _tl) in &plan {
                    track_tx.set(
                        tracking.clone(),
                        Ref::Tag(Tag { target: *local_head, created_at: 0, message: None }),
                    );
                }
                track_tx.commit()?;
                for (name, _, _, _, _) in &plan {
                    results.push(RefResult { name: name.clone(), status: PushStatus::Ok });
                }
            }
            Err(Error::TransactionConflict(_)) => {
                // A concurrent pusher won: report the peer's actual head per ref.
                for (name, _, _, _, _) in &plan {
                    let server_head = to
                        .refs
                        .get_timeline(name)?
                        .map(|t| t.head)
                        .unwrap_or_else(|| ObjectId::new([0u8; 32]));
                    results.push(RefResult {
                        name: name.clone(),
                        status: PushStatus::NonFastForward { server_head },
                    });
                }
            }
            Err(e) => return Err(e),
        }
        Ok(results)
    }

    // bole-cy6
    /// Bootstraps a fresh in-memory repo from `from`: the maximal fetch plus
    /// local timelines mirroring the advertised heads and `origin` tracking refs.
    pub async fn clone_from(from: &Repository, accessor: &Accessor) -> Result<Repository> {
        let repo = Repository::memory();
        repo.fetch("origin", from, accessor).await?;
        let mut tx = repo.refs.transaction();
        for name in from.list_refs_filtered("", accessor)? {
            if let Some(Ref::Timeline(t)) = from.refs.get(&name)? {
                tx.create_timeline(
                    name.clone(),
                    t.head,
                    t.policy.clone(),
                    t.created_at,
                    t.kind.clone(),
                    t.expires_at,
                );
            }
        }
        tx.commit()?;
        Ok(repo)
    }
}

// bole-cy6
/// The `(ref, target)` heads a fetching actor may pull (ref-granularity filter).
fn advertise(from: &Repository, accessor: &Accessor) -> Result<Vec<(RefName, ObjectId)>> {
    let mut adverts = Vec::new();
    for name in from.list_refs_filtered("", accessor)? {
        match from.refs.get(&name)? {
            Some(Ref::Timeline(t)) => adverts.push((name, t.head)),
            Some(Ref::Tag(t)) => adverts.push((name, t.target)),
            None => {}
        }
    }
    Ok(adverts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
    use crate::acl::lattice::{Label, LabelLattice};
    use crate::acl::rules::LabelRuleSet;
    use crate::object::{EntryKind, Snapshot, TreeEntry};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    async fn seed(repo: &Repository, name: &str, payload: &[u8]) -> (RefName, ObjectId) {
        let blob = repo.objects.put_blob(bytes::Bytes::copy_from_slice(payload)).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert("f".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(e).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();
        let rn = RefName::new(name).unwrap();
        repo.refs
            .create_timeline(rn.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        (rn, snap)
    }

    async fn advance(repo: &Repository, name: &RefName, payload: &[u8], parent: ObjectId) -> ObjectId {
        let blob = repo.objects.put_blob(bytes::Bytes::copy_from_slice(payload)).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert("f".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(e).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![parent], author: "t".into(), created_at: 1, message: "n".into() })
            .await
            .unwrap();
        repo.refs.advance_head(name, snap).unwrap();
        snap
    }

    fn writer() -> Accessor {
        // Confined-free Write clearance up to protected, scoped to all timelines.
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: Label::protected(),
                cap: Capability::WRITE,
                scope: Some(ClearanceScope::Timeline("**".into())),
            }],
            confined: false,
        };
        Accessor::from_parts(Arc::new(LabelLattice::two_point()), Arc::new(LabelRuleSet::default()), clr)
    }

    #[tokio::test]
    async fn fetch_transfers_closure_and_tracks_refs() {
        let src = Repository::memory();
        let (name, head) = seed(&src, "main", b"v1").await;
        let dst = Repository::memory();

        let tracked = dst.fetch("origin", &src, &Accessor::privileged()).await.unwrap();
        // Objects transferred (self-verifying receive).
        assert!(dst.objects.get(&head).await.unwrap().is_some());
        // Remote-tracking ref set; local timeline NOT created.
        let tref = RefName::new("refs/remotes/origin/main").unwrap();
        assert_eq!(dst.refs.get_tag(&tref).unwrap().unwrap().target, head);
        assert!(dst.refs.get_timeline(&name).unwrap().is_none());
        assert_eq!(tracked.len(), 1);
    }

    #[tokio::test]
    async fn fetch_is_minimal_second_time() {
        let src = Repository::memory();
        seed(&src, "main", b"v1").await;
        let dst = Repository::memory();
        dst.fetch("origin", &src, &Accessor::privileged()).await.unwrap();
        let before = dst.objects.count().await.unwrap();
        // A second fetch with nothing new transfers zero objects.
        let have = negotiate::have_set(&dst).await.unwrap();
        let heads: Vec<_> = advertise(&src, &Accessor::privileged()).unwrap().into_iter().map(|(_, t)| t).collect();
        let missing = negotiate::missing_closure(&src, &heads, &have).await.unwrap();
        assert!(missing.is_empty(), "nothing new to send");
        dst.fetch("origin", &src, &Accessor::privileged()).await.unwrap();
        assert_eq!(dst.objects.count().await.unwrap(), before);
    }

    #[tokio::test]
    async fn clone_then_push_fast_forward() {
        let server = Repository::memory();
        let (name, base) = seed(&server, "main", b"base").await;

        let client = Repository::clone_from(&server, &Accessor::privileged()).await.unwrap();
        // Clone created the local timeline + tracking ref + objects.
        assert_eq!(client.refs.get_timeline(&name).unwrap().unwrap().head, base);

        let next = advance(&client, &name, b"next", base).await;
        let res = client.push("origin", &server, std::slice::from_ref(&name), &writer()).await.unwrap();
        assert_eq!(res[0].status, PushStatus::Ok);
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, next);
    }

    #[tokio::test]
    async fn concurrent_push_one_wins_one_nonff() {
        let server = Repository::memory();
        let (name, base) = seed(&server, "main", b"base").await;
        let a = Repository::clone_from(&server, &Accessor::privileged()).await.unwrap();
        let b = Repository::clone_from(&server, &Accessor::privileged()).await.unwrap();

        // Both diverge from base, then both push (both saw tracking = base).
        let a_head = advance(&a, &name, b"a-work", base).await;
        let b_head = advance(&b, &name, b"b-work", base).await;

        let ra = a.push("origin", &server, std::slice::from_ref(&name), &writer()).await.unwrap();
        let rb = b.push("origin", &server, std::slice::from_ref(&name), &writer()).await.unwrap();

        let a_ok = ra[0].status == PushStatus::Ok;
        let b_ok = rb[0].status == PushStatus::Ok;
        assert!(a_ok ^ b_ok, "exactly one push must win: {ra:?} {rb:?}");

        let winner = if a_ok { a_head } else { b_head };
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, winner);
        let loser = if a_ok { &rb[0] } else { &ra[0] };
        assert_eq!(loser.status, PushStatus::NonFastForward { server_head: winner });
    }

    #[tokio::test]
    async fn push_denied_without_write_clearance() {
        let server = Repository::memory();
        let (name, base) = seed(&server, "main", b"base").await;
        // Protect the timeline so a no-clearance actor cannot write it.
        server.acls.set_timeline_acl(crate::acl::TimelineAcl { pattern: "main".into() }).unwrap();

        let client = Repository::clone_from(&server, &Accessor::privileged()).await.unwrap();
        advance(&client, &name, b"next", base).await;
        // A default accessor has no write clearance.
        let res = client.push("origin", &server, std::slice::from_ref(&name), &Accessor::new()).await.unwrap();
        assert!(matches!(res[0].status, PushStatus::Denied(_)), "got {:?}", res[0].status);
        // Server head unchanged.
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, base);
    }
}
