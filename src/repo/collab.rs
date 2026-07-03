// bole-18p
//! Collaboration-object publication and serving for a `Repository`.
//!
//! Publication is an explicit act: a collaboration object becomes discoverable
//! only when pinned under [`COLLAB_PUBLIC_PREFIX`]. Serving and discovery read
//! *only* that prefix, so scoped objects (a future capability-scoped mode) are
//! never surfaced. Per key / per (from,kind,to) only the highest `seq` is kept.

use async_trait::async_trait;

use crate::collab::discovery::{Index, PublicObjectSource};
use crate::collab::trust::TrustGraph;
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

pub(crate) fn kind_seg(kind: TrustKind) -> &'static str {
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

    // bole-440
    /// Every verified collab object currently tracked from pulled peers (under
    /// `refs/collab/remotes/`).
    pub async fn tracked_collab(&self) -> Result<Vec<CollabObject>> {
        let mut out = Vec::new();
        for name in self.refs.list(COLLAB_REMOTES_PREFIX)? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Collab(obj)) = self.objects.get(&tag.target).await? {
                    let ok = match &obj {
                        CollabObject::Profile(p) => crate::collab::verify_profile(p),
                        CollabObject::TrustEdge(e) => crate::collab::verify_edge(e),
                    };
                    if ok {
                        out.push(obj);
                    }
                }
            }
        }
        Ok(out)
    }

    // bole-440
    /// Builds the WS8a discovery [`Index`] from local state: own public objects at
    /// distance 0, plus tracked peers whose key is within `hops` of `self_key` in
    /// the combined `Follow` graph. Peers outside the neighborhood are excluded.
    pub async fn local_discovery_index(&self, self_key: &Key, hops: u8) -> Result<Index> {
        // Own public objects (distance 0).
        let mut own: Vec<CollabObject> = Vec::new();
        for p in self.public_profiles().await? {
            own.push(CollabObject::Profile(p));
        }
        for e in self.public_edges().await? {
            own.push(CollabObject::TrustEdge(e));
        }
        let tracked = self.tracked_collab().await?;

        // Combined edge set drives the follow neighborhood.
        let mut edges: Vec<TrustEdge> = Vec::new();
        for o in own.iter().chain(tracked.iter()) {
            if let CollabObject::TrustEdge(e) = o {
                edges.push(e.clone());
            }
        }
        let graph = TrustGraph::from_edges(edges);
        // bole-wg8
        let paths = graph.follow_paths(self_key, hops);

        // Group tracked objects by author, keep only in-neighborhood authors.
        use std::collections::BTreeMap;
        let mut by_author: BTreeMap<Key, Vec<CollabObject>> = BTreeMap::new();
        for o in tracked {
            let a = match &o {
                CollabObject::Profile(p) => p.key,
                CollabObject::TrustEdge(e) => e.from_key,
            };
            by_author.entry(a).or_default().push(o);
        }
        // bole-wg8
        let mut pulled: Vec<(Key, u8, Vec<Key>, Vec<CollabObject>)> = Vec::new();
        for (author, objs) in by_author {
            if let Some(path) = paths.get(&author) {
                let dist = path.len().saturating_sub(1) as u8;
                pulled.push((author, dist, path.clone(), objs));
            }
        }
        Ok(Index::build(*self_key, own, pulled))
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

    // bole-440
    #[tokio::test]
    async fn local_index_ranks_by_distance() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([60u8; 32]);
        let bob = CollabSigner::from_seed([61u8; 32]);
        // I publish my profile and follow bob.
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(bob.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        // Track bob's profile under the remotes prefix (as a pull would).
        let bp = bob.sign_profile("bob".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(bp))).await.unwrap();
        let fp = crate::collab::fingerprint(&bob.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        let me_hits = idx.query("me");
        assert_eq!(me_hits.len(), 1);
        assert_eq!(me_hits[0].distance, 0, "own profile at distance 0");
        let bob_hits = idx.query("bob");
        assert_eq!(bob_hits.len(), 1);
        assert_eq!(bob_hits[0].distance, 1, "followed peer at distance 1");
    }

    // bole-440
    #[tokio::test]
    async fn local_index_excludes_unfollowed() {
        use crate::collab::{CollabObject, CollabSigner};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([62u8; 32]);
        let stranger = CollabSigner::from_seed([63u8; 32]);
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // Track a stranger I do NOT follow.
        let sp = stranger.sign_profile("stranger".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(sp))).await.unwrap();
        let fp = crate::collab::fingerprint(&stranger.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        assert!(idx.query("stranger").is_empty(), "unfollowed peer is not in the neighborhood");
    }

    // bole-wg8
    #[tokio::test]
    async fn index_emits_depth2_path() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([70u8; 32]);
        let b = CollabSigner::from_seed([71u8; 32]);
        let c = CollabSigner::from_seed([72u8; 32]);
        // me -follow-> b ; and I have cached b's profile, b's follow-edge to c, and c's profile.
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();

        async fn cache(repo: &Repository, obj: CollabObject) {
            let author = match &obj { CollabObject::Profile(p) => p.key, CollabObject::TrustEdge(e) => e.from_key };
            let leaf = match &obj {
                CollabObject::Profile(_) => "profile".to_string(),
                CollabObject::TrustEdge(e) => format!("edge/follow/{}", fingerprint(&e.to_key)),
            };
            let id = repo.objects.put(&Object::Collab(obj)).await.unwrap();
            let fp = fingerprint(&author);
            let mut tx = repo.refs.transaction();
            tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{fp}/{leaf}")).unwrap(),
                   Ref::Tag(Tag { target: id, created_at: 0, message: None }));
            tx.commit().unwrap();
        }
        cache(&repo, CollabObject::Profile(b.sign_profile("bob".into(), String::new(), vec![], vec![], 1))).await;
        cache(&repo, CollabObject::TrustEdge(b.sign_edge(c.public_key(), TrustKind::Follow, None, 1))).await;
        cache(&repo, CollabObject::Profile(c.sign_profile("cee".into(), String::new(), vec![], vec![], 1))).await;

        let idx = repo.local_discovery_index(&me.public_key(), 2).await.unwrap();
        let cee = idx.query("cee");
        assert_eq!(cee.len(), 1);
        assert_eq!(cee[0].distance, 2, "c reached at depth 2 via cache-forward");
        assert_eq!(cee[0].trust_path, vec![me.public_key(), b.public_key(), c.public_key()], "path [me,b,c]");
    }
}
