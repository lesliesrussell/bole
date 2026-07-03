// bole-18p
//! Collaboration-object publication and serving for a `Repository`.
//!
//! Publication is an explicit act: a collaboration object becomes discoverable
//! only when pinned under [`COLLAB_PUBLIC_PREFIX`]. Serving and discovery read
//! *only* that prefix, so scoped objects (a future capability-scoped mode) are
//! never surfaced. Per key / per (from,kind,to) only the highest `seq` is kept.

use async_trait::async_trait;

use crate::collab::discovery::PublicObjectSource;
use crate::collab::{fingerprint, verify_edge, verify_profile, CollabObject, Key, Profile, TrustEdge, TrustKind};
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;

/// Ref prefix under which discoverable (public) collaboration objects are pinned.
pub const COLLAB_PUBLIC_PREFIX: &str = "refs/collab/public/";
/// Ref prefix reserved for future capability-scoped collaboration objects. Never
/// served or indexed by this slice.
pub const COLLAB_SCOPED_PREFIX: &str = "refs/collab/scoped/";
// bole-x5u
/// Ref prefix under which a pulled peer's verified public objects are tracked,
/// keyed by the peer's key fingerprint. Never merged into the node's own
/// published set (`refs/collab/public/`).
pub const COLLAB_REMOTES_PREFIX: &str = "refs/collab/remotes/";

fn kind_seg(kind: TrustKind) -> &'static str {
    match kind {
        TrustKind::Vouch => "vouch",
        TrustKind::Follow => "follow",
        TrustKind::Review => "review",
    }
}

impl Repository {
    // bole-18p
    /// Publishes a signed `Profile` to the public prefix. Rejects an invalid
    /// signature and any `seq` not strictly greater than the current profile's.
    pub async fn publish_profile(&self, p: &Profile) -> Result<ObjectId> {
        // bole-eul
        let _publish_guard = self.publish_lock.lock().await;
        if !verify_profile(p) {
            return Err(Error::PolicyViolation("profile signature does not verify".into()));
        }
        if let Some(cur) = self.profile(&p.key).await? {
            if p.seq <= cur.seq {
                return Err(Error::PolicyViolation(
                    "profile seq must be greater than the current profile's".into(),
                ));
            }
        }
        let id = self
            .objects
            .put(&Object::Collab(CollabObject::Profile(p.clone())))
            .await?;
        let leaf = format!("{COLLAB_PUBLIC_PREFIX}profile/{}", fingerprint(&p.key));
        let mut tx = self.refs.transaction();
        tx.set(
            RefName::new(leaf)?,
            Ref::Tag(Tag { target: id, created_at: 0, message: None }),
        );
        tx.commit()?;
        Ok(id)
    }

    // bole-18p
    /// The current (highest-`seq`) profile for `key`, if any is published.
    pub async fn profile(&self, key: &Key) -> Result<Option<Profile>> {
        let name =
            RefName::new(format!("{COLLAB_PUBLIC_PREFIX}profile/{}", fingerprint(key)))?;
        let tag = match self.refs.get_tag(&name)? {
            Some(t) => t,
            None => return Ok(None),
        };
        match self.objects.get(&tag.target).await? {
            Some(Object::Collab(CollabObject::Profile(p))) => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    // bole-18p
    /// Publishes a signed `TrustEdge`. Rejects an invalid signature and any `seq`
    /// not strictly greater than the current edge's for the same `(from,kind,to)`.
    pub async fn publish_edge(&self, e: &TrustEdge) -> Result<ObjectId> {
        // bole-eul
        let _publish_guard = self.publish_lock.lock().await;
        if !verify_edge(e) {
            return Err(Error::PolicyViolation("trust edge signature does not verify".into()));
        }
        let leaf = format!(
            "{COLLAB_PUBLIC_PREFIX}edge/{}/{}/{}",
            fingerprint(&e.from_key),
            kind_seg(e.kind),
            fingerprint(&e.to_key),
        );
        let name = RefName::new(leaf)?;
        if let Some(tag) = self.refs.get_tag(&name)? {
            if let Some(Object::Collab(CollabObject::TrustEdge(cur))) =
                self.objects.get(&tag.target).await?
            {
                if e.seq <= cur.seq {
                    return Err(Error::PolicyViolation(
                        "trust edge seq must be greater than the current edge's".into(),
                    ));
                }
            }
        }
        let id = self
            .objects
            .put(&Object::Collab(CollabObject::TrustEdge(e.clone())))
            .await?;
        let mut tx = self.refs.transaction();
        tx.set(
            name,
            Ref::Tag(Tag { target: id, created_at: 0, message: None }),
        );
        tx.commit()?;
        Ok(id)
    }

    // bole-18p
    /// Every current public profile (one per key).
    pub async fn public_profiles(&self) -> Result<Vec<Profile>> {
        let mut out = Vec::new();
        for name in self.refs.list(&format!("{COLLAB_PUBLIC_PREFIX}profile/"))? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(CollabObject::Profile(p))) =
                    self.objects.get(&tag.target).await?
                {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    // bole-18p
    /// Every current public trust edge.
    pub async fn public_edges(&self) -> Result<Vec<TrustEdge>> {
        let mut out = Vec::new();
        for name in self.refs.list(&format!("{COLLAB_PUBLIC_PREFIX}edge/"))? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(CollabObject::TrustEdge(e))) =
                    self.objects.get(&tag.target).await?
                {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }
}

// bole-18p
#[async_trait]
impl PublicObjectSource for Repository {
    async fn public_objects(&self) -> Result<Vec<CollabObject>> {
        let mut out: Vec<CollabObject> = Vec::new();
        for p in self.public_profiles().await? {
            out.push(CollabObject::Profile(p));
        }
        for e in self.public_edges().await? {
            out.push(CollabObject::TrustEdge(e));
        }
        Ok(out)
    }
}

// bole-18p
#[cfg(test)]
mod tests {
    use crate::collab::discovery::PublicObjectSource;
    use crate::collab::{CollabObject, CollabSigner, TrustKind};
    use crate::object::Object;
    use crate::refs::{Ref, RefName, Tag};
    use crate::repo::collab::{COLLAB_PUBLIC_PREFIX, COLLAB_SCOPED_PREFIX};
    use crate::repo::Repository;

    #[tokio::test]
    async fn serve_returns_only_public() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([10u8; 32]);
        let p = a.sign_profile("A".into(), String::new(), vec![], vec![], 1);
        repo.publish_profile(&p).await.unwrap();
        let served = repo.public_objects().await.unwrap();
        assert_eq!(served.len(), 1);
        assert!(matches!(&served[0], CollabObject::Profile(pp) if pp.key == a.public_key()));
        assert!(COLLAB_PUBLIC_PREFIX.starts_with("refs/collab/"));
    }

    #[tokio::test]
    async fn scoped_collab_never_served() {
        // Directly pin a collab object under the SCOPED prefix (simulating a
        // future capability-scoped object) and prove discovery/serve never sees it.
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([11u8; 32]);
        let p = a.sign_profile("secret".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(p))).await.unwrap();
        let leaf = format!("{COLLAB_SCOPED_PREFIX}profile/scoped");
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(leaf).unwrap(), Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let served = repo.public_objects().await.unwrap();
        assert!(served.is_empty(), "scoped objects must never be served");
    }

    #[tokio::test]
    async fn higher_seq_profile_supersedes() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([12u8; 32]);
        repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_profile(&a.sign_profile("v2".into(), String::new(), vec![], vec![], 2)).await.unwrap();
        let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
        assert_eq!(cur.display_name, "v2");
        assert_eq!(cur.seq, 2);
    }

    #[tokio::test]
    async fn stale_seq_rejected() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([13u8; 32]);
        repo.publish_profile(&a.sign_profile("v2".into(), String::new(), vec![], vec![], 2)).await.unwrap();
        let err = repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await;
        assert!(err.is_err(), "publishing a lower seq must be rejected");
        let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
        assert_eq!(cur.seq, 2);
        // bole-18p
        // equal seq is also stale, not an advance
        let eq = repo.publish_profile(&a.sign_profile("v2-again".into(), String::new(), vec![], vec![], 2)).await;
        assert!(eq.is_err(), "re-publishing the same seq must be rejected");
    }

    #[tokio::test]
    async fn higher_seq_edge_supersedes() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([14u8; 32]);
        let b = CollabSigner::from_seed([15u8; 32]);
        repo.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Vouch, Some("b1".into()), 1)).await.unwrap();
        repo.publish_edge(&a.sign_edge(b.public_key(), TrustKind::Vouch, Some("b2".into()), 2)).await.unwrap();
        let edges = repo.public_edges().await.unwrap();
        let v: Vec<_> = edges.iter().filter(|e| e.from_key == a.public_key() && e.kind == TrustKind::Vouch).collect();
        assert_eq!(v.len(), 1, "only the current edge per (from,kind,to)");
        assert_eq!(v[0].petname.as_deref(), Some("b2"));
    }

    #[tokio::test]
    async fn rejects_unsigned_profile() {
        let repo = Repository::memory();
        let a = CollabSigner::from_seed([16u8; 32]);
        let mut p = a.sign_profile("A".into(), String::new(), vec![], vec![], 1);
        p.display_name = "forged".into();
        assert!(repo.publish_profile(&p).await.is_err());
    }

    // bole-eul
    #[tokio::test]
    async fn concurrent_publish_keeps_higher_seq() {
        use std::sync::Arc;
        let repo = Arc::new(Repository::memory());
        let a = CollabSigner::from_seed([50u8; 32]);
        // seq 1 exists first so both concurrent publishes are "advances".
        repo.publish_profile(&a.sign_profile("v1".into(), String::new(), vec![], vec![], 1)).await.unwrap();

        let p2 = a.sign_profile("v2".into(), String::new(), vec![], vec![], 2);
        let p3 = a.sign_profile("v3".into(), String::new(), vec![], vec![], 3);
        let (r2, r3) = (repo.clone(), repo.clone());
        let (a2, a3) = (p2.clone(), p3.clone());
        let t2 = tokio::spawn(async move { let _ = r2.publish_profile(&a2).await; });
        let t3 = tokio::spawn(async move { let _ = r3.publish_profile(&a3).await; });
        t2.await.unwrap();
        t3.await.unwrap();

        // Whichever ordering occurred, a lower seq must never overwrite a higher one.
        let cur = repo.profile(&a.public_key()).await.unwrap().unwrap();
        assert_eq!(cur.seq, 3, "highest seq must be current after concurrent publish");
    }
}
