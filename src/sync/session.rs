// bole-6qy
//! The sync protocol state machine, written entirely against the [`Conn`] trait
//! so any transport can drive it. `serve` is the responder; `client_fetch` /
//! `client_push` are the initiators. The object transfer reuses the WS4 pack
//! (self-verifying receive) and the ref reconciliation reuses the WS4
//! `RefTransaction` CAS — the same primitives as the in-process core in
//! [`super`], now exchanged as [`Message`] frames.

use std::collections::HashSet;

use crate::acl::{Accessor, ResourceRef};
use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::{Ref, RefName, Tag, TimelinePolicy};
use crate::repo::Repository;
use crate::store::pack::{decode_pack, PackBuilder};
use crate::sync::negotiate;
use crate::sync::transport::Conn;
use crate::sync::wire::{
    CapSet, Intent, Message, RefAdvert, RefApplyStatus, RefStatusEntry, RefUpdateOp, PROTO_VERSION,
};

// bole-6qy
/// Responder: reads the client's `Hello` and drives the requested exchange.
/// `accessor` gates which refs are advertised and authorizes pushes (WS1).
pub async fn serve(conn: &mut dyn Conn, repo: &Repository, accessor: &Accessor) -> Result<()> {
    match conn.recv().await? {
        Message::Hello { intent, .. } => match intent {
            Intent::Fetch | Intent::Clone => serve_fetch(conn, repo, accessor).await,
            Intent::Push => serve_push(conn, repo, accessor).await,
        },
        _ => {
            conn.send(&Message::Error("expected Hello".into())).await?;
            Err(Error::Storage("protocol: expected Hello".into()))
        }
    }
}

async fn serve_fetch(conn: &mut dyn Conn, repo: &Repository, accessor: &Accessor) -> Result<()> {
    let refs = advertise(repo, accessor)?;
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs }).await?;
    let (want, have) = match conn.recv().await? {
        Message::HaveWant { want, have } => (want, have),
        _ => return Err(Error::Storage("protocol: expected HaveWant".into())),
    };
    let have: HashSet<ObjectId> = have.into_iter().collect();
    let missing = negotiate::missing_closure(repo, &want, &have).await?;
    let pack = build_pack(repo, &missing).await?;
    conn.send(&Message::Pack(pack)).await?;
    conn.send(&Message::Done).await?;
    Ok(())
}

async fn serve_push(conn: &mut dyn Conn, repo: &Repository, accessor: &Accessor) -> Result<()> {
    // Advertise the server's current heads (its `have` summary for the targets).
    let refs = advertise(repo, accessor)?;
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs }).await?;

    // Land the pushed objects (self-verifying) BEFORE any ref CAS.
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("protocol: expected Pack".into())),
    };
    for (_id, canonical) in decode_pack(&pack)? {
        repo.objects.put_raw(&canonical).await?;
    }

    let ops = match conn.recv().await? {
        Message::RefUpdate(ops) => ops,
        _ => return Err(Error::Storage("protocol: expected RefUpdate".into())),
    };
    let results = apply_push_ops(repo, accessor, &ops).await?;
    conn.send(&Message::RefStatus(results)).await?;
    Ok(())
}

// bole-6qy
/// Server-side: authorize + fast-forward-gate each op, then CAS all survivors in
/// one `RefTransaction`; a concurrent-winner conflict reports NonFastForward.
pub(crate) async fn apply_push_ops(
    repo: &Repository,
    accessor: &Accessor,
    ops: &[RefUpdateOp],
) -> Result<Vec<RefStatusEntry>> {
    let lattice = repo.acls.lattice()?;
    let rules = repo.acls.label_ruleset()?;
    let mut results = Vec::new();
    let mut accepted: Vec<RefUpdateOp> = Vec::new();

    for op in ops {
        let label = rules.label_for_timeline(&lattice, op.name.as_str());
        if !accessor.can_write(&label, ResourceRef::Timeline(op.name.as_str())) {
            results.push(RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("write denied on timeline".into()),
            });
            continue;
        }
        if let (Some(old), Some(remote)) = (op.expected_old, repo.refs.get_timeline(&op.name)?) {
            let ff = repo.find_common_ancestor(old, op.new_head).await? == Some(old);
            if !matches!(remote.policy, TimelinePolicy::Unrestricted) && !ff {
                results.push(RefStatusEntry {
                    name: op.name.clone(),
                    status: RefApplyStatus::NonFastForward { server_head: remote.head },
                });
                continue;
            }
        }
        accepted.push(op.clone());
    }

    if accepted.is_empty() {
        return Ok(results);
    }
    let mut tx = repo.refs.transaction();
    for op in &accepted {
        match op.expected_old {
            Some(old) => {
                tx.advance_head_if(op.name.clone(), old, op.new_head);
            }
            None => {
                tx.create_timeline(
                    op.name.clone(),
                    op.new_head,
                    TimelinePolicy::Unrestricted,
                    0,
                    "persistent".into(),
                    None,
                );
            }
        }
    }
    match tx.commit() {
        Ok(()) => {
            for op in &accepted {
                results.push(RefStatusEntry { name: op.name.clone(), status: RefApplyStatus::Ok });
            }
        }
        Err(Error::TransactionConflict(_)) => {
            for op in &accepted {
                let server_head = repo
                    .refs
                    .get_timeline(&op.name)?
                    .map(|t| t.head)
                    .unwrap_or_else(|| ObjectId::new([0u8; 32]));
                results.push(RefStatusEntry {
                    name: op.name.clone(),
                    status: RefApplyStatus::NonFastForward { server_head },
                });
            }
        }
        Err(e) => return Err(e),
    }
    Ok(results)
}

// bole-6qy
/// Initiator: pull the peer's readable ref closure into `local`, updating
/// `refs/remotes/<remote_name>/*` tracking refs. Returns the tracking refs set.
pub async fn client_fetch(
    conn: &mut dyn Conn,
    local: &Repository,
    remote_name: &str,
) -> Result<Vec<(RefName, ObjectId)>> {
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Fetch,
    })
    .await?;
    let refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("protocol: expected Welcome".into())),
    };
    let want: Vec<ObjectId> = refs.iter().map(|r| r.target).collect();
    let have: Vec<ObjectId> = local.objects.list().await?;
    conn.send(&Message::HaveWant { want, have }).await?;

    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("protocol: expected Pack".into())),
    };
    let _done = conn.recv().await?; // Done
    for (_id, canonical) in decode_pack(&pack)? {
        local.objects.put_raw(&canonical).await?;
    }

    let mut tx = local.refs.transaction();
    let mut tracked = Vec::new();
    for r in &refs {
        let tref = tracking_ref(remote_name, &r.name)?;
        tx.set(tref.clone(), Ref::Tag(Tag { target: r.target, created_at: 0, message: None }));
        tracked.push((tref, r.target));
    }
    tx.commit()?;
    Ok(tracked)
}

// bole-6qy
/// Initiator: push the given local timelines, CAS-ing the peer's heads against
/// this repo's remote-tracking refs. Returns per-ref status; advances tracking
/// refs for the ops the server accepted.
pub async fn client_push(
    conn: &mut dyn Conn,
    local: &Repository,
    remote_name: &str,
    timelines: &[RefName],
) -> Result<Vec<RefStatusEntry>> {
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Push,
    })
    .await?;
    let server_refs = match conn.recv().await? {
        Message::Welcome { refs, .. } => refs,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("protocol: expected Welcome".into())),
    };
    let server_have: HashSet<ObjectId> = server_refs.iter().map(|r| r.target).collect();

    let mut ops = Vec::new();
    let mut wants = Vec::new();
    for name in timelines {
        let tl = match local.refs.get_timeline(name)? {
            Some(t) => t,
            None => continue,
        };
        let tracking = tracking_ref(remote_name, name)?;
        let expected_old = local.refs.get_tag(&tracking)?.map(|t| t.target);
        wants.push(tl.head);
        ops.push(RefUpdateOp { name: name.clone(), expected_old, new_head: tl.head });
    }

    let missing = negotiate::missing_closure(local, &wants, &server_have).await?;
    let pack = build_pack(local, &missing).await?;
    conn.send(&Message::Pack(pack)).await?;
    conn.send(&Message::RefUpdate(ops.clone())).await?;

    let results = match conn.recv().await? {
        Message::RefStatus(r) => r,
        Message::Error(e) => return Err(Error::Storage(e)),
        _ => return Err(Error::Storage("protocol: expected RefStatus".into())),
    };

    // Advance tracking refs for accepted ops (we now know the peer's head).
    let mut tx = local.refs.transaction();
    for entry in &results {
        if entry.status == RefApplyStatus::Ok {
            if let Some(op) = ops.iter().find(|o| o.name == entry.name) {
                tx.set(
                    tracking_ref(remote_name, &entry.name)?,
                    Ref::Tag(Tag { target: op.new_head, created_at: 0, message: None }),
                );
            }
        }
    }
    tx.commit()?;
    Ok(results)
}

// bole-6qy
fn tracking_ref(remote_name: &str, name: &RefName) -> Result<RefName> {
    RefName::new(format!("refs/remotes/{remote_name}/{}", name.as_str()))
        .map_err(|e| Error::Storage(format!("bad tracking ref name: {e}")))
}

// bole-6qy
pub(crate) fn advertise(repo: &Repository, accessor: &Accessor) -> Result<Vec<RefAdvert>> {
    let mut out = Vec::new();
    for name in repo.list_refs_filtered("", accessor)? {
        match repo.refs.get(&name)? {
            Some(Ref::Timeline(t)) => out.push(RefAdvert { name, target: t.head, is_timeline: true }),
            Some(Ref::Tag(t)) => out.push(RefAdvert { name, target: t.target, is_timeline: false }),
            None => {}
        }
    }
    Ok(out)
}

// bole-6qy
pub(crate) async fn build_pack(repo: &Repository, ids: &[ObjectId]) -> Result<Vec<u8>> {
    let mut builder = PackBuilder::new();
    for id in ids {
        if let Some(bytes) = repo.objects.get_raw(id).await? {
            builder.add(*id, bytes.to_vec());
        }
    }
    let (pack, _entries, _digest) = builder.finish()?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
    use crate::acl::lattice::{Label, LabelLattice};
    use crate::acl::rules::LabelRuleSet;
    use crate::object::{EntryKind, Snapshot, TreeEntry};
    use crate::sync::transport::InProcessConn;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// A write-capable accessor (server authorizes pushes with this).
    fn writer() -> Accessor {
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

    #[tokio::test]
    async fn fetch_over_session_transfers_and_tracks() {
        let server = Arc::new(Repository::memory());
        let (name, head) = seed(&server, "main", b"v1").await;
        let client = Repository::memory();

        let (mut client_conn, mut server_conn) = InProcessConn::pair();
        let srv = server.clone();
        let handle = tokio::spawn(async move {
            let acc = Accessor::privileged();
            serve(&mut server_conn, &srv, &acc).await
        });

        let tracked = client_fetch(&mut client_conn, &client, "origin").await.unwrap();
        handle.await.unwrap().unwrap();

        assert!(client.objects.get(&head).await.unwrap().is_some());
        assert_eq!(tracked.len(), 1);
        let tref = RefName::new("refs/remotes/origin/main").unwrap();
        assert_eq!(client.refs.get_tag(&tref).unwrap().unwrap().target, head);
        // Fetch never creates a local timeline.
        assert!(client.refs.get_timeline(&name).unwrap().is_none());
    }

    #[tokio::test]
    async fn push_over_session_cas_advances_head() {
        // Server has main@base; client fetches, advances, pushes.
        let server = Arc::new(Repository::memory());
        let (name, base) = seed(&server, "main", b"base").await;
        let client = Repository::memory();

        // Fetch to seed the client + its tracking ref.
        {
            let (mut cc, mut sc) = InProcessConn::pair();
            let srv = server.clone();
            let h = tokio::spawn(async move {
                serve(&mut sc, &srv, &Accessor::privileged()).await
            });
            client_fetch(&mut cc, &client, "origin").await.unwrap();
            h.await.unwrap().unwrap();
        }
        // Create the local timeline at base and advance it.
        client.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let next = {
            let blob = client.objects.put_blob(bytes::Bytes::from_static(b"next")).await.unwrap();
            let mut e = BTreeMap::new();
            e.insert("f".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
            let tree = client.objects.put_tree(e).await.unwrap();
            let s = client.objects.put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "n".into() }).await.unwrap();
            client.refs.advance_head(&name, s).unwrap();
            s
        };

        // Push over a session (server authorizes with a privileged accessor).
        let (mut cc, mut sc) = InProcessConn::pair();
        let srv = server.clone();
        let h = tokio::spawn(async move {
            serve(&mut sc, &srv, &writer()).await
        });
        let results = client_push(&mut cc, &client, "origin", std::slice::from_ref(&name)).await.unwrap();
        h.await.unwrap().unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, RefApplyStatus::Ok);
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, next);
        assert!(server.objects.get(&next).await.unwrap().is_some());
    }
}
