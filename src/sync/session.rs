// bole-6qy
//! The sync protocol state machine, written entirely against the [`Conn`] trait
//! so any transport can drive it. `serve` is the responder; `client_fetch` /
//! `client_push` are the initiators. The object transfer reuses the WS4 pack
//! (self-verifying receive) and the ref reconciliation reuses the WS4
//! `RefTransaction` CAS — the same primitives as the in-process core in
//! [`super`], now exchanged as [`Message`] frames.

use std::collections::HashSet;

use crate::acl::hook::{PolicyContext, PolicyDecision, PolicyEvent};
use crate::acl::{Accessor, ResourceRef};
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
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
    // bole-yl2: the objects we may serve are rooted only at refs this accessor is
    // authorized to read. Capture that set BEFORE trusting the client's `want`.
    let authorized: HashSet<ObjectId> = refs.iter().map(|r| r.target).collect();
    // bole-nbug
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs, relay_sig: None }).await?;
    let (want, have) = match conn.recv().await? {
        Message::HaveWant { want, have } => (want, have),
        _ => return Err(Error::Storage("protocol: expected HaveWant".into())),
    };
    // bole-yl2: do NOT trust client-supplied roots. Constrain `want` to the
    // authorized advertised targets, so a client cannot name an arbitrary
    // ObjectId (e.g. a protected head or a Secret) and pull its closure. This
    // enforces the read-ACL on served objects, not just on the advert, and
    // matches the HTTP fetch path (which derives wants from the adverts).
    let want: Vec<ObjectId> = want.into_iter().filter(|w| authorized.contains(w)).collect();
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
    // bole-nbug
    conn.send(&Message::Welcome { proto: PROTO_VERSION, caps: CapSet::EMPTY, refs, relay_sig: None }).await?;

    // Decode + verify the pack (bounded — bole-oby) but do NOT land it yet.
    let pack = match conn.recv().await? {
        Message::Pack(p) => p,
        _ => return Err(Error::Storage("protocol: expected Pack".into())),
    };
    let decoded = decode_pack(&pack)?;

    let ops = match conn.recv().await? {
        Message::RefUpdate(ops) => ops,
        _ => return Err(Error::Storage("protocol: expected RefUpdate".into())),
    };

    // bole-zez: authorize before landing. A connection with no write capability
    // at all can never have a ref op accepted, so refuse it without writing any
    // objects to the durable store (prevents an unauthorized/anonymous peer from
    // planting objects or consuming storage on a push that will be fully denied).
    if !accessor.has_write_capability() {
        let denied = ops
            .iter()
            .map(|op| RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("no write capability".into()),
            })
            .collect();
        conn.send(&Message::RefStatus(denied)).await?;
        return Ok(());
    }

    // Land the pushed objects (self-verifying) BEFORE the ref CAS; apply_push_ops
    // needs them present to validate heads and ancestry.
    for (_id, canonical) in &decoded {
        repo.objects.put_raw(canonical).await?;
    }
    let results = apply_push_ops(repo, accessor, &ops).await?;
    conn.send(&Message::RefStatus(results)).await?;
    Ok(())
}

// bole-sq4
/// True iff every object reachable from `root` is present in the store (and thus
/// decodable). Iterative with a seen-set; content addressing rules out cycles so
/// it always terminates.
async fn closure_present(objects: &crate::store::ObjectStore, root: ObjectId) -> Result<bool> {
    let mut stack = vec![root];
    let mut seen: HashSet<ObjectId> = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        match objects.get(&id).await? {
            Some(obj) => stack.extend(negotiate::child_edges(&obj)),
            None => return Ok(false),
        }
    }
    Ok(true)
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

    // bole-7c1
    // A replicated advance may only be gated by policy that every replica would
    // evaluate identically. If the repo binds any non-deterministic hook, refuse
    // all incoming ops fail-closed rather than accept a history a peer might
    // reject (divergence under CAS-on-heads). Default repos bind only the
    // deterministic built-in, so this is a no-op for them.
    let registry = repo.policy_registry().await?;
    if !registry.deterministic() {
        let reason = format!(
            "repository binds non-deterministic policy hook(s) [{}]; refusing replicated push (fail-closed)",
            registry.non_deterministic().join(", ")
        );
        return Ok(ops
            .iter()
            .map(|op| RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied(reason.clone()),
            })
            .collect());
    }

    for op in ops {
        // bole-au0t: the policy namespace is repository governance, never
        // writable via push. Without this, a write-capable peer could squat
        // `refs/policy/root` as a timeline and wedge every later
        // `policy_root()` read (persistent fail-closed DoS) — or the whole
        // batch would abort on the WrongRefKind read below, collateral-failing
        // legitimate ops.
        if op.name.as_str().starts_with("refs/policy/") {
            results.push(RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("reserved policy ref".into()),
            });
            continue;
        }
        let label = rules.label_for_timeline(&lattice, op.name.as_str());
        if !accessor.can_write(&label, ResourceRef::Timeline(op.name.as_str())) {
            results.push(RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("write denied on timeline".into()),
            });
            continue;
        }
        // bole-e9a: a ref may only point at a real Snapshot. Reject an op whose
        // new_head is absent or is some other object type, so a peer cannot make
        // a timeline dangle at a blob/tree id.
        if !matches!(repo.objects.get(&op.new_head).await?, Some(Object::Snapshot(_))) {
            results.push(RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("new head is not a snapshot".into()),
            });
            continue;
        }
        // bole-sq4: the whole reachable closure of new_head must be present, not
        // just the head node. A pusher that omits part of the snapshot's tree/blob
        // or parent closure would otherwise advance the head to an un-checkout-able
        // state that replicates the corruption to other peers.
        if !closure_present(&repo.objects, op.new_head).await? {
            results.push(RefStatusEntry {
                name: op.name.clone(),
                status: RefApplyStatus::Denied("incomplete object closure".into()),
            });
            continue;
        }
        let current = repo.refs.get_timeline(&op.name)?;
        if let (Some(old), Some(remote)) = (op.expected_old, current.as_ref()) {
            let ff = repo.find_common_ancestor(old, op.new_head).await? == Some(old);
            if !matches!(remote.policy, TimelinePolicy::Unrestricted) && !ff {
                results.push(RefStatusEntry {
                    name: op.name.clone(),
                    status: RefApplyStatus::NonFastForward { server_head: remote.head },
                });
                continue;
            }
        }
        // bole-rdh: run the deterministic policy registry on a REPLICATED advance
        // to an EXISTING timeline (a create is governed by the can_write check
        // above). The registry is known-deterministic here — a non-deterministic
        // hook already fail-closed all ops above — so evaluate_replayable's
        // verdict is safe to enforce. This is the sync-side analogue of the
        // registry.evaluate call in Repository::advance_timeline.
        if let Some(remote) = current.as_ref() {
            let ctx = PolicyContext {
                event: PolicyEvent::Advance {
                    timeline: &op.name,
                    old_head: remote.head,
                    new_head: op.new_head,
                },
                accessor,
                objects: &repo.objects,
                refs: &repo.refs,
                now: 0,
            };
            match registry.evaluate_replayable(&ctx).await {
                PolicyDecision::Allow => {}
                PolicyDecision::Deny(reason)
                | PolicyDecision::RequiresApproval { reason, .. } => {
                    results.push(RefStatusEntry {
                        name: op.name.clone(),
                        status: RefApplyStatus::Denied(reason),
                    });
                    continue;
                }
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
                // bole-e9a: a pushed-created timeline defaults to FastForwardOnly,
                // not Unrestricted. Unrestricted would permanently allow history
                // rewrite on that timeline via later pushes; fast-forward-only is
                // the safe default (the wire op carries no policy to honour).
                tx.create_timeline(
                    op.name.clone(),
                    op.new_head,
                    TimelinePolicy::FastForwardOnly,
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
    // bole-nbug
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Fetch,
        client_nonce: None,
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
    // bole-nbug
    conn.send(&Message::Hello {
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
        caps: CapSet::EMPTY,
        intent: Intent::Push,
        client_nonce: None,
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
    // bole-7c1
    /// A repo that binds a non-deterministic hook must refuse a replicated push
    /// fail-closed, and accept it once the hook is absent.
    #[tokio::test]
    async fn push_refused_when_policy_non_deterministic() {
        use crate::acl::policy_object::HookSpec;
        let mut repo = Repository::memory();
        let (name, base) = seed(&repo, "main", b"a").await;

        // A real child advance base -> child.
        let child = repo
            .objects
            .put_snapshot(Snapshot {
                root: repo.objects.put_tree(BTreeMap::new()).await.unwrap(),
                parents: vec![base],
                author: "t".into(),
                created_at: 1,
                message: "c".into(),
            })
            .await
            .unwrap();
        let op = RefUpdateOp { name: name.clone(), expected_old: Some(base), new_head: child };

        // With no custom hook (only the deterministic built-in), the push applies.
        let ok = apply_push_ops(&repo, &writer(), std::slice::from_ref(&op)).await.unwrap();
        assert!(matches!(ok[0].status, RefApplyStatus::Ok), "clean repo should accept: {:?}", ok[0].status);

        // Bind the non-deterministic signed-approval hook (bole-6i7: it loads
        // attestations from mutable refs). The guard fires before any head check,
        // so the current head state is irrelevant.
        repo.register_hook(HookSpec {
            kind: "signed-approval".into(),
            pattern: "**".into(),
            params: BTreeMap::from([("needed".to_string(), 1u64)]),
        });

        let denied = apply_push_ops(&repo, &writer(), std::slice::from_ref(&op)).await.unwrap();
        match &denied[0].status {
            RefApplyStatus::Denied(r) => {
                assert!(r.contains("non-deterministic"), "reason: {r}");
                assert!(r.contains("signed-approval"), "reason should name the hook: {r}");
            }
            other => panic!("expected fail-closed Denied, got {other:?}"),
        }
    }

    // bole-au0t
    /// A replica whose pinned policy root names a hook kind this binary does not
    /// recognize must refuse a replicated push fail-closed, not skip the hook.
    #[tokio::test]
    async fn push_refused_when_pinned_root_has_unknown_hook_kind() {
        use crate::acl::policy_object::{HookSpec, PolicyRoot};
        let repo = Repository::memory();
        let (name, base) = seed(&repo, "main", b"a").await;
        let child = repo
            .objects
            .put_snapshot(Snapshot {
                root: repo.objects.put_tree(BTreeMap::new()).await.unwrap(),
                parents: vec![base],
                author: "t".into(),
                created_at: 1,
                message: "c".into(),
            })
            .await
            .unwrap();
        let op = RefUpdateOp { name, expected_old: Some(base), new_head: child };

        repo.set_policy_root(&PolicyRoot {
            lattice: ObjectId::from_content(b"lattice"),
            rules: ObjectId::from_content(b"rules"),
            parent: None,
            hooks: vec![HookSpec {
                kind: "quantum-approval".into(),
                pattern: "**".into(),
                params: BTreeMap::new(),
            }],
        })
        .await
        .unwrap();

        let err = apply_push_ops(&repo, &writer(), std::slice::from_ref(&op)).await.unwrap_err();
        assert!(
            err.to_string().contains("unknown policy hook kind"),
            "expected fail-closed unknown-kind rejection, got {err:?}"
        );
    }

    // bole-au0t
    /// Ops naming reserved policy refs are denied per-op: a write-capable peer
    /// must not be able to squat `refs/policy/root` (wedging every later
    /// `policy_root()` read) or otherwise touch the policy namespace via push.
    /// Legitimate ops in the same batch still apply.
    #[tokio::test]
    async fn push_op_naming_policy_ref_is_denied_as_reserved() {
        let repo = Repository::memory();
        let (name, base) = seed(&repo, "main", b"a").await;
        let child = repo
            .objects
            .put_snapshot(Snapshot {
                root: repo.objects.put_tree(BTreeMap::new()).await.unwrap(),
                parents: vec![base],
                author: "t".into(),
                created_at: 1,
                message: "c".into(),
            })
            .await
            .unwrap();
        let squat = RefUpdateOp {
            name: RefName::new("refs/policy/root").unwrap(),
            expected_old: None,
            new_head: child,
        };
        let legit = RefUpdateOp { name: name.clone(), expected_old: Some(base), new_head: child };

        let results = apply_push_ops(&repo, &writer(), &[squat, legit]).await.unwrap();
        match &results[0].status {
            RefApplyStatus::Denied(r) => assert!(r.contains("reserved"), "reason: {r}"),
            other => panic!("expected reserved-ref denial, got {other:?}"),
        }
        assert!(matches!(results[1].status, RefApplyStatus::Ok), "legit op must still apply: {:?}", results[1].status);
        // The policy namespace is untouched; the repo is not wedged.
        assert!(repo.policy_root().await.unwrap().is_none());
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, child);
    }

    // bole-sq4
    #[tokio::test]
    async fn push_with_incomplete_closure_is_denied() {
        let repo = Repository::memory();
        // A snapshot whose root tree id was never stored — its closure is broken.
        let orphan_tree = ObjectId::from_content(b"never-stored-tree");
        let snap = repo
            .objects
            .put_snapshot(Snapshot {
                root: orphan_tree,
                parents: vec![],
                author: "t".into(),
                created_at: 0,
                message: "m".into(),
            })
            .await
            .unwrap();
        // The snapshot node itself IS present (passes the bole-e9a check), but its
        // reachable closure is incomplete.
        let op = RefUpdateOp {
            name: RefName::new("main").unwrap(),
            expected_old: None,
            new_head: snap,
        };
        let res = apply_push_ops(&repo, &writer(), std::slice::from_ref(&op)).await.unwrap();
        assert!(
            matches!(&res[0].status, RefApplyStatus::Denied(m) if m.contains("incomplete object closure")),
            "incomplete closure must be denied, got {:?}",
            res[0].status
        );
        assert!(repo.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().is_none());
    }

    // bole-zez
    #[tokio::test]
    async fn push_without_write_capability_lands_nothing() {
        let server = Arc::new(Repository::memory());
        let client = Repository::memory();
        let (name, head) = seed(&client, "main", b"data").await;
        let missing =
            crate::sync::negotiate::missing_closure(&client, &[head], &HashSet::new()).await.unwrap();
        let pack = build_pack(&client, &missing).await.unwrap();

        let (mut cc, mut sc) = InProcessConn::pair();
        let srv = server.clone();
        let handle = tokio::spawn(async move {
            // Read-only accessor: no write capability at all.
            serve(&mut sc, &srv, &Accessor::new()).await
        });

        // bole-nbug
        cc.send(&Message::Hello {
            proto_min: PROTO_VERSION,
            proto_max: PROTO_VERSION,
            caps: CapSet::EMPTY,
            intent: Intent::Push,
            client_nonce: None,
        })
        .await
        .unwrap();
        let _welcome = cc.recv().await.unwrap();
        cc.send(&Message::Pack(pack)).await.unwrap();
        cc.send(&Message::RefUpdate(vec![RefUpdateOp {
            name: name.clone(),
            expected_old: None,
            new_head: head,
        }]))
        .await
        .unwrap();
        let status = cc.recv().await.unwrap();
        handle.await.unwrap().unwrap();

        match status {
            Message::RefStatus(s) => {
                assert!(matches!(&s[0].status, RefApplyStatus::Denied(_)), "got {:?}", s[0].status)
            }
            other => panic!("expected RefStatus, got {other:?}"),
        }
        // Objects must NOT have landed for a connection with no write capability.
        assert!(
            server.objects.get(&head).await.unwrap().is_none(),
            "no-write push must not land objects"
        );
    }

    // bole-e9a
    #[tokio::test]
    async fn push_create_defaults_to_fast_forward_and_rejects_non_snapshot() {
        let repo = Repository::memory();
        // A real snapshot to create a timeline at.
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot {
                root: tree,
                parents: vec![],
                author: "t".into(),
                created_at: 0,
                message: "m".into(),
            })
            .await
            .unwrap();

        // Create via push -> the new timeline must be FastForwardOnly, not
        // Unrestricted (which would permanently allow history rewrite).
        let name = RefName::new("main").unwrap();
        let create = RefUpdateOp { name: name.clone(), expected_old: None, new_head: snap };
        let res = apply_push_ops(&repo, &writer(), std::slice::from_ref(&create)).await.unwrap();
        assert!(matches!(res[0].status, RefApplyStatus::Ok), "create should succeed: {:?}", res[0].status);
        assert_eq!(
            repo.refs.get_timeline(&name).unwrap().unwrap().policy,
            TimelinePolicy::FastForwardOnly
        );

        // A ref op whose new_head is a blob (not a snapshot) must be refused.
        let blob = repo.objects.put_blob(bytes::Bytes::from("x")).await.unwrap();
        let bad = RefUpdateOp {
            name: RefName::new("evil").unwrap(),
            expected_old: None,
            new_head: blob,
        };
        let res2 = apply_push_ops(&repo, &writer(), std::slice::from_ref(&bad)).await.unwrap();
        assert!(
            matches!(&res2[0].status, RefApplyStatus::Denied(m) if m.contains("not a snapshot")),
            "non-snapshot head must be denied, got {:?}",
            res2[0].status
        );
        assert!(repo.refs.get_timeline(&RefName::new("evil").unwrap()).unwrap().is_none());
    }

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

    // bole-yl2
    #[tokio::test]
    async fn serve_fetch_refuses_unauthorized_want() {
        use crate::acl::TimelineAcl;
        let server = Arc::new(Repository::memory());
        let (_pub_name, _pub_head) = seed(&server, "main", b"public").await;
        let (_sec_name, secret_head) = seed(&server, "secret/x", b"classified").await;
        server.acls.set_timeline_acl(TimelineAcl { pattern: "secret/**".into() }).unwrap();

        let (mut client_conn, mut server_conn) = InProcessConn::pair();
        let srv = server.clone();
        let handle = tokio::spawn(async move {
            // Empty accessor: not cleared for the protected timeline.
            serve(&mut server_conn, &srv, &Accessor::new()).await
        });

        // Client asks for the protected head DIRECTLY, bypassing the advert.
        // bole-nbug
        client_conn
            .send(&Message::Hello {
                proto_min: PROTO_VERSION,
                proto_max: PROTO_VERSION,
                caps: CapSet::EMPTY,
                intent: Intent::Fetch,
                client_nonce: None,
            })
            .await
            .unwrap();
        let welcome = client_conn.recv().await.unwrap();
        if let Message::Welcome { refs, .. } = &welcome {
            assert!(
                refs.iter().all(|r| r.target != secret_head),
                "protected head must not be advertised"
            );
        } else {
            panic!("expected Welcome, got {welcome:?}");
        }
        client_conn
            .send(&Message::HaveWant { want: vec![secret_head], have: vec![] })
            .await
            .unwrap();
        let pack = match client_conn.recv().await.unwrap() {
            Message::Pack(p) => p,
            other => panic!("expected Pack, got {other:?}"),
        };
        let _done = client_conn.recv().await.unwrap();
        handle.await.unwrap().unwrap();

        let served: Vec<ObjectId> =
            decode_pack(&pack).unwrap().into_iter().map(|(id, _)| id).collect();
        assert!(
            !served.contains(&secret_head),
            "serve_fetch leaked a protected object the accessor cannot read"
        );
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
