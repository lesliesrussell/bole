// bole-ub3h
//! Repo-record persistence for a `Repository`.
//!
//! A [`RepoRecord`](crate::reporecord::RepoRecord) is published under the
//! owner's public collab prefix, one ref per (owner, name), keeping only the
//! highest `seq`. Verification is fail-closed on write and read. This is what a
//! hub enumerates to list a user's repos under their profile.

use crate::collab::{fingerprint, Key};
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::refs::{Ref, RefName, Tag};
use crate::reporecord::{verify_repo, RepoRecord};
use crate::repo::collab::COLLAB_PUBLIC_PREFIX;
use crate::repo::Repository;

// bole-ub3h
/// The leaf name under `refs/collab/public/` for one owner's repo record.
fn repo_ref(owner: &Key, name: &str) -> String {
    format!("{COLLAB_PUBLIC_PREFIX}repo/{}/{name}", fingerprint(owner))
}

impl Repository {
    // bole-ub3h
    /// Publishes a signed [`RepoRecord`] to the public prefix. Rejects an
    /// invalid signature and any `seq` not strictly greater than the current
    /// record's for the same (owner, name). Serialized under the publish lock
    /// so a concurrent publish cannot pass a stale seq check (as for profiles).
    pub async fn publish_repo(&self, r: &RepoRecord) -> Result<ObjectId> {
        let _guard = self.publish_lock.lock().await;
        if !verify_repo(r) {
            return Err(Error::PolicyViolation("repo record signature does not verify".into()));
        }
        let name = RefName::new(repo_ref(&r.owner, &r.name))?;
        if let Some(tag) = self.refs.get_tag(&name)? {
            if let Some(Object::RepoRecord(cur)) = self.objects.get(&tag.target).await? {
                if r.seq <= cur.seq {
                    return Err(Error::PolicyViolation(
                        "repo record seq must be greater than the current record's".into(),
                    ));
                }
            }
        }
        let id = self.objects.put(&Object::RepoRecord(r.clone())).await?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-ub3h
    /// The current [`RepoRecord`] for `(owner, name)`, verified fail-closed.
    pub async fn get_repo(&self, owner: &Key, name: &str) -> Result<Option<RepoRecord>> {
        let ref_name = RefName::new(repo_ref(owner, name))?;
        let tag = match self.refs.get_tag(&ref_name)? {
            Some(t) => t,
            None => return Ok(None),
        };
        match self.objects.get(&tag.target).await? {
            Some(Object::RepoRecord(r)) if verify_repo(&r) => Ok(Some(r)),
            _ => Ok(None),
        }
    }

    // bole-ub3h
    /// Every repo `owner` has announced (verified fail-closed), name-sorted.
    pub async fn list_repos(&self, owner: &Key) -> Result<Vec<RepoRecord>> {
        let prefix = format!("{COLLAB_PUBLIC_PREFIX}repo/{}/", fingerprint(owner));
        let mut out = Vec::new();
        for ref_name in self.refs.list(&prefix)? {
            if let Some(tag) = self.refs.get_tag(&ref_name)? {
                if let Some(Object::RepoRecord(r)) = self.objects.get(&tag.target).await? {
                    if verify_repo(&r) {
                        out.push(r);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::reporecord::RepoSigner;
    use crate::Repository;

    #[tokio::test]
    async fn publish_get_list_and_monotonic() {
        let repo = Repository::memory();
        let ada = RepoSigner::from_seed([20u8; 32]);
        repo.publish_repo(&ada.sign_repo("grove", "the frontend", 1)).await.unwrap();
        repo.publish_repo(&ada.sign_repo("dotfiles", "config", 1)).await.unwrap();

        let repos = repo.list_repos(&ada.public_key()).await.unwrap();
        assert_eq!(repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["dotfiles", "grove"]);
        assert_eq!(repo.get_repo(&ada.public_key(), "grove").await.unwrap().unwrap().description, "the frontend");

        // A higher seq supersedes; a stale seq is rejected.
        repo.publish_repo(&ada.sign_repo("grove", "the frontend hub", 2)).await.unwrap();
        assert_eq!(repo.get_repo(&ada.public_key(), "grove").await.unwrap().unwrap().description, "the frontend hub");
        assert!(repo.publish_repo(&ada.sign_repo("grove", "rollback", 1)).await.is_err(), "stale seq refused");
        assert_eq!(repo.list_repos(&ada.public_key()).await.unwrap().len(), 2, "still two repos");
    }

    #[tokio::test]
    async fn publish_rejects_unsigned_and_lists_per_owner() {
        let repo = Repository::memory();
        let ada = RepoSigner::from_seed([21u8; 32]);
        let bob = RepoSigner::from_seed([22u8; 32]);

        let mut bad = ada.sign_repo("x", "d", 1);
        bad.description = "tampered".into();
        assert!(repo.publish_repo(&bad).await.is_err(), "tampered record refused");

        repo.publish_repo(&ada.sign_repo("a", "", 1)).await.unwrap();
        repo.publish_repo(&bob.sign_repo("b", "", 1)).await.unwrap();
        assert_eq!(repo.list_repos(&ada.public_key()).await.unwrap().len(), 1, "only ada's repos");
        assert_eq!(repo.list_repos(&bob.public_key()).await.unwrap().len(), 1, "only bob's repos");
        assert!(repo.get_repo(&ada.public_key(), "b").await.unwrap().is_none(), "b is bob's, not ada's");
    }
}
