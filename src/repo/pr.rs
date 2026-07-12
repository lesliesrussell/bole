// bole-060a
//! Change-proposal persistence for a `Repository`.
//!
//! A [`ChangeProposal`](crate::pr::ChangeProposal) is stored like any other
//! content-addressed object; these helpers verify it fail-closed on both write
//! and read, so a stored proposal is always signature-valid. Later slices add
//! a name/id registry, review threads, and the approval-gated merge action.

use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::pr::{verify_comment, verify_proposal, ChangeProposal, ReviewComment};
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;

// bole-xwqv
/// Ref prefix under which published change proposals are pinned (one tag per
/// proposal, named by its object id). Pinning makes proposals GC-roots so they
/// survive `gc()`, and lets `list_proposals` enumerate them.
pub const PROPOSALS_PREFIX: &str = "refs/proposals/";

// bole-t290
/// The ref prefix under which a proposal's review comments are pinned. Each
/// comment tag is `refs/proposals/comments/<proposal-id>/<comment-id>`. Keeping
/// comments under their own top-level segment (not nested inside a proposal's
/// own tag name) keeps the two enumerations independent.
pub const COMMENTS_PREFIX: &str = "refs/proposals/comments/";

impl Repository {
    // bole-060a
    /// Stores a signed [`ChangeProposal`], returning its content-addressed id.
    /// Fail-closed: a proposal whose signature does not verify is rejected and
    /// nothing is stored.
    ///
    /// NOTE (slice 1): the proposal is stored but not yet pinned under any ref,
    /// so it is a GC-ephemeral leaf — a `gc()` past the grace window collects
    /// it. The registry slice (bole-xwqv) MUST pin proposals under a
    /// `refs/proposals/...` tag so they survive GC, mirroring how
    /// `publish_profile` pins profiles.
    pub async fn put_proposal(&self, p: &ChangeProposal) -> Result<ObjectId> {
        if !verify_proposal(p) {
            return Err(Error::PolicyViolation(
                "change proposal signature does not verify".into(),
            ));
        }
        self.objects.put(&Object::ChangeProposal(p.clone())).await
    }

    // bole-060a
    /// Loads the [`ChangeProposal`] at `id`. `None` if absent or not a proposal.
    /// Fail-closed: a stored object whose signature does not verify is treated
    /// as absent (`None`) rather than returned.
    pub async fn get_proposal(&self, id: &ObjectId) -> Result<Option<ChangeProposal>> {
        match self.objects.get(id).await? {
            Some(Object::ChangeProposal(p)) if verify_proposal(&p) => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    // bole-xwqv
    /// Publishes a signed [`ChangeProposal`]: stores it (fail-closed verify) and
    /// pins it under `refs/proposals/<id>` so it survives GC and appears in
    /// [`list_proposals`](Repository::list_proposals). Returns its id.
    /// Idempotent — the ref name is the content id.
    pub async fn publish_proposal(&self, p: &ChangeProposal) -> Result<ObjectId> {
        let id = self.put_proposal(p).await?;
        let name = RefName::new(format!("{PROPOSALS_PREFIX}{id}"))?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-xwqv
    /// Every published proposal (id + proposal), verified fail-closed — a
    /// pinned ref whose object is missing or does not verify is skipped.
    pub async fn list_proposals(&self) -> Result<Vec<(ObjectId, ChangeProposal)>> {
        let mut out = Vec::new();
        for name in self.refs.list(PROPOSALS_PREFIX)? {
            // bole-t290: the comments sub-namespace lives under the same top
            // prefix; skip it so proposal enumeration stays clean.
            if name.as_str().starts_with(COMMENTS_PREFIX) {
                continue;
            }
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(p) = self.get_proposal(&tag.target).await? {
                    out.push((tag.target, p));
                }
            }
        }
        Ok(out)
    }

    // bole-t290
    /// Adds a signed [`ReviewComment`] to its proposal's thread: verifies it
    /// (fail-closed), stores it, and pins it under
    /// `refs/proposals/comments/<proposal-id>/<comment-id>` so it survives GC
    /// and appears in [`list_comments`](Repository::list_comments). Errors if
    /// the referenced proposal is not present (a comment must attach to a real,
    /// verified proposal). Returns the comment's id.
    pub async fn add_comment(&self, c: &ReviewComment) -> Result<ObjectId> {
        if !verify_comment(c) {
            return Err(Error::PolicyViolation(
                "review comment signature does not verify".into(),
            ));
        }
        if self.get_proposal(&c.proposal).await?.is_none() {
            return Err(Error::PolicyViolation(format!(
                "review comment references an unknown proposal: {}",
                c.proposal
            )));
        }
        let id = self.objects.put(&Object::ReviewComment(c.clone())).await?;
        let name = RefName::new(format!("{COMMENTS_PREFIX}{}/{id}", c.proposal))?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-t290
    /// Loads the [`ReviewComment`] at `id`, verified fail-closed. `None` if
    /// absent, not a comment, or unverifiable.
    pub async fn get_comment(&self, id: &ObjectId) -> Result<Option<ReviewComment>> {
        match self.objects.get(id).await? {
            Some(Object::ReviewComment(c)) if verify_comment(&c) => Ok(Some(c)),
            _ => Ok(None),
        }
    }

    // bole-t290
    /// Every review comment on `proposal` (id + comment), verified fail-closed.
    pub async fn list_comments(&self, proposal: &ObjectId) -> Result<Vec<(ObjectId, ReviewComment)>> {
        let mut out = Vec::new();
        let prefix = format!("{COMMENTS_PREFIX}{proposal}/");
        for name in self.refs.list(&prefix)? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(c) = self.get_comment(&tag.target).await? {
                    out.push((tag.target, c));
                }
            }
        }
        Ok(out)
    }

    // bole-ooxm
    /// Merges the proposal at `id`: its `source` timeline into its `target`.
    /// Reuses the existing merge machinery — no new gating engine. The ACL leak
    /// scan and the approval `PolicyHook` are enforced by
    /// [`advance_timeline`](Repository::advance_timeline), so a proposal into an
    /// approval-gated target (e.g. `release/**` requiring N signed approvals)
    /// returns [`Error::ApprovalRequired`](crate::Error::ApprovalRequired) until
    /// the approvals exist. On a clean, permitted merge the target advances to a
    /// new merge snapshot; a conflicting merge is reported without applying.
    pub async fn merge_proposal(
        &self,
        id: &ObjectId,
        author: String,
        created_at: u64,
        message: String,
        accessor: &crate::acl::Accessor,
    ) -> Result<ProposalMerge> {
        let p = self
            .get_proposal(id)
            .await?
            .ok_or_else(|| Error::Storage(format!("proposal not found: {id}")))?;
        let source = RefName::new(&p.source)?;
        let target = RefName::new(&p.target)?;
        let source_head = self
            .refs
            .get_timeline(&source)?
            .ok_or_else(|| Error::Storage(format!("source timeline not found: {}", p.source)))?
            .head;
        let target_head = self
            .refs
            .get_timeline(&target)?
            .ok_or_else(|| Error::Storage(format!("target timeline not found: {}", p.target)))?
            .head;

        let result = self.merge_timelines(&source, &target, accessor).await?;
        if !result.conflicts.is_empty() {
            return Ok(ProposalMerge::Conflicts(result.conflicts));
        }
        let root = crate::repo::ephemeral::build_tree(&self.objects, &result.merged).await?;
        let merged_snap = self
            .objects
            .put_snapshot(crate::object::Snapshot {
                root,
                parents: vec![target_head, source_head],
                author,
                created_at,
                message,
            })
            .await?;
        // Approval/ACL gate is enforced here (bole-rdh / bole-p2bf).
        self.advance_timeline(&target, merged_snap, accessor).await?;
        Ok(ProposalMerge::Merged(merged_snap))
    }
}

// bole-ooxm
/// The outcome of [`Repository::merge_proposal`] short of an error: either the
/// merge was applied (the target advanced to `Merged(snapshot)`), or it had
/// path conflicts and was not applied. Approval-required and access-denied
/// outcomes surface as `Err` from `merge_proposal`, not here.
#[derive(Debug, Clone)]
pub enum ProposalMerge {
    /// Applied cleanly; the target now points at this merge snapshot.
    Merged(crate::object::ObjectId),
    /// Not applied — these paths conflicted.
    Conflicts(Vec<crate::repo::merge::MergeConflict>),
}

#[cfg(test)]
mod tests {
    use crate::pr::ProposalSigner;
    use crate::Repository;

    #[tokio::test]
    async fn put_get_round_trip() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([7u8; 32]);
        let p = signer.sign_proposal("feature/x", "release/1.0", "Add x", 5);
        let id = repo.put_proposal(&p).await.unwrap();
        let got = repo.get_proposal(&id).await.unwrap().expect("proposal round-trips");
        assert_eq!(got, p);
    }

    #[tokio::test]
    async fn put_rejects_unsigned() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([8u8; 32]);
        let mut bad = signer.sign_proposal("a", "b", "t", 0);
        bad.title = "tampered".into(); // breaks the signature
        assert!(repo.put_proposal(&bad).await.is_err(), "unsigned proposal must be refused");
    }

    #[tokio::test]
    async fn get_absent_and_wrong_type_is_none() {
        let repo = Repository::memory();
        // Absent id.
        let missing = crate::ObjectId::from_content(b"nope");
        assert!(repo.get_proposal(&missing).await.unwrap().is_none());
        // An id pointing at a non-proposal object.
        let blob = repo.objects.put_blob(bytes::Bytes::from("x")).await.unwrap();
        assert!(repo.get_proposal(&blob).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn publish_list_and_survives_gc() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([11u8; 32]);
        let a = signer.sign_proposal("feature/a", "main", "A", 1);
        let b = signer.sign_proposal("feature/b", "main", "B", 2);
        let ida = repo.publish_proposal(&a).await.unwrap();
        let idb = repo.publish_proposal(&b).await.unwrap();

        let mut listed = repo.list_proposals().await.unwrap();
        listed.sort_by_key(|(_, p)| p.title.clone());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, ida);
        assert_eq!(listed[1].0, idb);

        // Pinned proposals are GC-roots: a sweep does not collect them.
        repo.gc(&[], 0, 1_000_000).await.unwrap();
        assert!(repo.get_proposal(&ida).await.unwrap().is_some(), "pinned proposal must survive GC");
        assert_eq!(repo.list_proposals().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn publish_is_idempotent() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([12u8; 32]);
        let p = signer.sign_proposal("x", "y", "t", 0);
        let id1 = repo.publish_proposal(&p).await.unwrap();
        let id2 = repo.publish_proposal(&p).await.unwrap();
        assert_eq!(id1, id2);
        assert_eq!(repo.list_proposals().await.unwrap().len(), 1, "same proposal pinned once");
    }

    #[tokio::test]
    async fn comment_add_list_and_survives_gc() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([13u8; 32]);
        let pid = repo.publish_proposal(&signer.sign_proposal("f", "main", "t", 0)).await.unwrap();

        let c1 = signer.sign_comment(pid, "first", false, 1);
        let c2 = signer.sign_comment(pid, "resolve", true, 2);
        let id1 = repo.add_comment(&c1).await.unwrap();
        let _id2 = repo.add_comment(&c2).await.unwrap();

        let mut listed = repo.list_comments(&pid).await.unwrap();
        listed.sort_by_key(|(_, c)| c.created_at);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].1.body, "first");
        assert!(listed[1].1.resolves);

        // Comments and their proposal survive GC; proposal listing is unaffected.
        repo.gc(&[], 0, 1_000_000).await.unwrap();
        assert_eq!(repo.list_comments(&pid).await.unwrap().len(), 2, "comments survive GC");
        assert_eq!(repo.list_proposals().await.unwrap().len(), 1, "comments dont leak into proposal listing");
        assert!(repo.get_comment(&id1).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn add_comment_rejects_unsigned_and_unknown_proposal() {
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([14u8; 32]);
        let pid = repo.publish_proposal(&signer.sign_proposal("f", "main", "t", 0)).await.unwrap();

        // Tampered comment -> refused.
        let mut bad = signer.sign_comment(pid, "b", false, 0);
        bad.body = "tampered".into();
        assert!(repo.add_comment(&bad).await.is_err(), "tampered comment refused");

        // Comment on a non-existent proposal -> refused.
        let orphan = signer.sign_comment(crate::ObjectId::from_content(b"ghost"), "b", false, 0);
        assert!(repo.add_comment(&orphan).await.is_err(), "comment on unknown proposal refused");
    }

    #[tokio::test]
    async fn merge_proposal_clean_advances_target() {
        use crate::acl::{Accessor, PathRole, Permission, TimelineRole};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use bytes::Bytes;
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        // base snapshot on both timelines; source adds a file.
        let base_tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot { root: base_tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() }).await.unwrap();
        let blob = repo.objects.put_blob(Bytes::from("hi")).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert("f.txt".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let src_tree = repo.objects.put_tree(e).await.unwrap();
        let src_head = repo.objects.put_snapshot(Snapshot { root: src_tree, parents: vec![base], author: "t".into(), created_at: 1, message: "add f".into() }).await.unwrap();
        repo.refs.create_timeline(RefName::new("feature/x").unwrap(), src_head, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(RefName::new("main").unwrap(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let signer = ProposalSigner::from_seed([21u8; 32]);
        let pid = repo.publish_proposal(&signer.sign_proposal("feature/x", "main", "add f", 0)).await.unwrap();
        let writer = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });

        let outcome = repo.merge_proposal(&pid, "merger".into(), 2, "merge".into(), &writer).await.unwrap();
        match outcome {
            crate::repo::pr::ProposalMerge::Merged(snap) => {
                assert_eq!(repo.refs.get_timeline(&RefName::new("main").unwrap()).unwrap().unwrap().head, snap, "target advanced to the merge snapshot");
            }
            other => panic!("expected clean merge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn merge_proposal_into_approval_gated_target_requires_approval() {
        use crate::acl::attestation::{ApproverRegistry, AttestationSigner};
        use crate::acl::policy_object::HookSpec;
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let mut repo = Repository::memory();
        // release/** requires 1 signed approval to advance.
        repo.register_hook(HookSpec { kind: "signed-approval".into(), pattern: "release/**".into(), params: BTreeMap::from([("needed".to_string(), 1u64)]) });

        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() }).await.unwrap();
        let src_head = repo.objects.put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into() }).await.unwrap();
        repo.refs.create_timeline(RefName::new("feature/x").unwrap(), src_head, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(RefName::new("release/1.0").unwrap(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let alice = AttestationSigner::from_seed("alice", [1u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        repo.set_approvers(&reg).await.unwrap();

        let signer = ProposalSigner::from_seed([22u8; 32]);
        let pid = repo.publish_proposal(&signer.sign_proposal("feature/x", "release/1.0", "ship", 0)).await.unwrap();
        let writer = Accessor::new().with_timeline_role(TimelineRole { pattern: "release/**".into(), permission: Permission::Write });

        // No approval yet -> merge_proposal surfaces ApprovalRequired (bole-p2bf),
        // and the target head does NOT move.
        let err = repo.merge_proposal(&pid, "m".into(), 2, "merge".into(), &writer).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::ApprovalRequired { .. }), "PR into release/** needs approval, got {err:?}");
        assert_eq!(repo.refs.get_timeline(&RefName::new("release/1.0").unwrap()).unwrap().unwrap().head, base, "target must not move");
    }

    #[tokio::test]
    async fn get_drops_tampered_stored_proposal() {
        use crate::object::Object;
        let repo = Repository::memory();
        let signer = ProposalSigner::from_seed([9u8; 32]);
        let mut p = signer.sign_proposal("a", "b", "t", 0);
        p.target = "release/prod".into(); // tamper AFTER signing
        // Store it raw, bypassing put_proposal's write check.
        let id = repo.objects.put(&Object::ChangeProposal(p)).await.unwrap();
        assert!(repo.get_proposal(&id).await.unwrap().is_none(), "tampered stored proposal must read as absent");
    }
}
