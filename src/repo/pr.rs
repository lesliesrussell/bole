// bole-060a
//! Change-proposal persistence for a `Repository`.
//!
//! A [`ChangeProposal`](crate::pr::ChangeProposal) is stored like any other
//! content-addressed object; these helpers verify it fail-closed on both write
//! and read, so a stored proposal is always signature-valid. Later slices add
//! a name/id registry, review threads, and the approval-gated merge action.

use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::pr::{verify_proposal, ChangeProposal};
use crate::repo::Repository;

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
