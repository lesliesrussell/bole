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
// bole-su8
use crate::collab::RelayPin;
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
// bole-su8
/// Local-only namespace for trusted-relay pins. NEVER advertised or served
/// (see `collab_adverts`, which is an allowlist of public + remotes only).
pub const COLLAB_RELAYS_PREFIX: &str = "refs/collab/relays/";

pub(crate) fn kind_seg(kind: TrustKind) -> &'static str {
    match kind {
        TrustKind::Vouch => "vouch",
        TrustKind::Follow => "follow",
        TrustKind::Review => "review",
    }
}

// bole-k93a
/// The head-snapshot summary of one timeline in this repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineView {
    pub name: String,
    pub head: ObjectId,
    pub author: String,
    pub created_at: u64,
}

// bole-k93a
/// The locally-verifiable hub view of a developer key: identity + own trust
/// out-edges (+ this repo's timelines when `key` is the repo's own identity).
/// Every emitted profile/edge is verified fail-closed. Transport-agnostic — the
/// library returns typed data; a caller renders JSON.
#[derive(Debug, Clone)]
pub struct ProfileBundle {
    pub key: Key,
    pub is_local: bool,
    pub profile: Option<Profile>,
    pub edges: Vec<TrustEdge>,
    pub timelines: Vec<TimelineView>,
}

// bole-581
/// One resolved discovery hit for the CLI: the canonical key, the author's
/// self-asserted display name (a hint), the trust-graph-resolved petname (None
/// when only the fingerprint is known), the reach distance (0/1/2), and the
/// minimal-hop trust path.
#[derive(Debug, Clone)]
pub struct QueryHit {
    pub key: Key,
    pub display_name: String,
    pub petname: Option<String>,
    pub reach: u8,
    pub trust_path: Vec<Key>,
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

    // bole-k93a
    /// Aggregate the locally-verifiable hub view of `key`: verified identity,
    /// own trust out-edges (`from_key == key`), and — when `key` is this repo's
    /// own published identity — the repo's timelines. Read-only; every emitted
    /// profile and edge is verified fail-closed (dropped if it does not verify).
    /// The first "bole as backend API" surface for the Grove frontend.
    pub async fn profile_bundle(&self, key: &Key, accessor: &crate::acl::Accessor) -> Result<ProfileBundle> {
        // is_local: this repo publishes a PUBLIC profile for `key`.
        let publics = self.public_profiles().await?;
        let is_local = publics.iter().any(|p| &p.key == key);

        // profile: own published (if local) else tracked peer; verified.
        let profile = if is_local {
            publics.into_iter().find(|p| &p.key == key)
        } else {
            self.tracked_collab().await?.into_iter().find_map(|o| match o {
                CollabObject::Profile(p) if &p.key == key => Some(p),
                _ => None,
            })
        }
        .filter(verify_profile);

        // edges: out-edges (from_key == key), verified fail-closed.
        let mut edges = Vec::new();
        if is_local {
            for e in self.public_edges().await? {
                if &e.from_key == key && verify_edge(&e) {
                    edges.push(e);
                }
            }
        } else {
            for o in self.tracked_collab().await? {
                if let CollabObject::TrustEdge(e) = o {
                    if &e.from_key == key && verify_edge(&e) {
                        edges.push(e);
                    }
                }
            }
        }

        // timelines: this repo's timelines when local, else empty. bole-k93a:
        // enumerate through the serve-path gate (list_refs_served) with the
        // caller's accessor, not a raw refs.list — so an ACL-protected or
        // scoped-collab timeline never enters a bundle served to a caller who
        // cannot read it (the bole-e78l serve-path invariant). The CLI passes
        // a privileged (read-all) accessor for the owner's own hub view.
        let mut timelines = Vec::new();
        if is_local {
            for name in self.list_refs_served("", accessor)? {
                if let Some(Ref::Timeline(t)) = self.refs.get(&name)? {
                    if let Some(Object::Snapshot(s)) = self.objects.get(&t.head).await? {
                        timelines.push(TimelineView {
                            name: name.as_str().to_string(),
                            head: t.head,
                            author: s.author,
                            created_at: s.created_at,
                        });
                    }
                }
            }
        }

        Ok(ProfileBundle { key: *key, is_local, profile, edges, timelines })
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

    // bole-581
    /// Runs the local discovery index for `term` and resolves a trust-scoped
    /// petname (via `Namer` over the combined follow/vouch graph; fingerprint
    /// fallback → None) plus reach + trust path for each hit.
    pub async fn query_discovery(&self, self_key: &Key, hops: u8, term: &str) -> Result<Vec<QueryHit>> {
        use crate::collab::naming::{Namer, PetnameResolution};
        let idx = self.local_discovery_index(self_key, hops).await?;

        // Rebuild the combined edge graph for petname resolution.
        let mut edges: Vec<TrustEdge> = self.public_edges().await?;
        for o in self.tracked_collab().await? {
            if let CollabObject::TrustEdge(e) = o {
                edges.push(e);
            }
        }
        let graph = TrustGraph::from_edges(edges);
        let local: std::collections::BTreeMap<Key, String> = std::collections::BTreeMap::new();
        let namer = Namer::new(*self_key, &local, &graph);

        let mut hits = Vec::new();
        for r in idx.query(term) {
            let display_name = match &r.object {
                CollabObject::Profile(p) => p.display_name.clone(),
                CollabObject::TrustEdge(_) => String::new(),
            };
            let petname = match namer.resolve(&r.key) {
                PetnameResolution::Local(n) => Some(n),
                PetnameResolution::Vouch { name, .. } => Some(name),
                PetnameResolution::Fingerprint(_) => None,
            };
            hits.push(QueryHit {
                key: r.key,
                display_name,
                petname,
                reach: r.distance,
                trust_path: r.trust_path.clone(),
            });
        }
        Ok(hits)
    }

    // bole-su8
    /// Upserts a trusted-relay pin, keyed by `fingerprint(&pin.key)` so a key maps
    /// to exactly one endpoint. Stored as an `Object::Blob` under
    /// `refs/collab/relays/<relay-fp>`. Local config; not a signed collab object.
    pub async fn add_relay(&self, pin: RelayPin) -> Result<()> {
        use crate::object::{Blob, Object};
        let id = self
            .objects
            .put(&Object::Blob(Blob { data: bytes::Bytes::from(pin.to_bytes()) }))
            .await?;
        let leaf = format!("{COLLAB_RELAYS_PREFIX}{}", crate::collab::fingerprint(&pin.key));
        let mut tx = self.refs.transaction();
        tx.set(RefName::new(leaf)?, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(())
    }

    // bole-su8
    /// Removes a relay pin by key. Returns whether a pin existed.
    pub async fn remove_relay(&self, key: &Key) -> Result<bool> {
        let leaf = format!("{COLLAB_RELAYS_PREFIX}{}", crate::collab::fingerprint(key));
        let name = RefName::new(leaf)?;
        if self.refs.get_tag(&name)?.is_none() {
            return Ok(false);
        }
        let mut tx = self.refs.transaction();
        tx.delete_ref(name);
        tx.commit()?;
        Ok(true)
    }

    // bole-su8
    /// All trusted-relay pins, ordered by ref name (relay fingerprint).
    pub async fn relays(&self) -> Result<Vec<RelayPin>> {
        use crate::object::Object;
        let mut out = Vec::new();
        for name in self.refs.list(COLLAB_RELAYS_PREFIX)? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(Object::Blob(b)) = self.objects.get(&tag.target).await? {
                    if let Some(pin) = RelayPin::from_bytes(&b.data) {
                        out.push(pin);
                    }
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

    // bole-k93a
    /// Caches a verified peer collab object under the remotes/ prefix, as a pull
    /// would — the shared fixture the discovery tests use.
    async fn cache_peer(repo: &Repository, key: &crate::collab::Key, obj: CollabObject, leaf: &str) {
        use crate::refs::{Ref, RefName, Tag};
        let id = repo.objects.put(&Object::Collab(obj)).await.unwrap();
        let fp = crate::collab::fingerprint(key);
        let mut tx = repo.refs.transaction();
        tx.set(
            RefName::new(format!("{}{fp}/{leaf}", crate::repo::collab::COLLAB_REMOTES_PREFIX)).unwrap(),
            Ref::Tag(Tag { target: id, created_at: 0, message: None }),
        );
        tx.commit().unwrap();
    }

    // bole-k93a
    #[tokio::test]
    async fn bundle_own_identity_full() {
        use crate::collab::{CollabSigner, TrustKind};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([1u8; 32]);
        let x = CollabSigner::from_seed([2u8; 32]);
        repo.publish_profile(&me.sign_profile("Me".into(), "hi".into(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(x.public_key(), TrustKind::Follow, Some("ex".into()), 1)).await.unwrap();

        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap_id = repo.objects.put(&Object::Snapshot(Snapshot { root: tree, parents: vec![], author: "me".into(), created_at: 7, message: "m".into() })).await.unwrap();
        repo.refs.create_timeline(RefName::new("main").unwrap(), snap_id, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let b = repo.profile_bundle(&me.public_key(), &crate::acl::Accessor::privileged()).await.unwrap();
        assert!(b.is_local);
        assert_eq!(b.profile.as_ref().unwrap().display_name, "Me");
        assert_eq!(b.edges.len(), 1);
        assert_eq!(b.edges[0].to_key, x.public_key());
        assert_eq!(b.timelines.len(), 1);
        assert_eq!(b.timelines[0].name, "main");
        assert_eq!(b.timelines[0].head, snap_id);
        assert_eq!(b.timelines[0].author, "me");
        assert_eq!(b.timelines[0].created_at, 7);
    }

    // bole-k93a
    /// A timeline the accessor cannot read is excluded from the bundle — the
    /// serve-path ACL gate (bole-e78l) applies to profile_bundle too.
    #[tokio::test]
    async fn bundle_timelines_respect_accessor_acl() {
        use crate::acl::{Accessor, Permission, TimelineAcl, TimelineRole};
        use crate::collab::CollabSigner;
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([20u8; 32]);
        repo.publish_profile(&me.sign_profile("Me".into(), String::new(), vec![], vec![], 1)).await.unwrap();

        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo.objects.put(&Object::Snapshot(Snapshot { root: tree, parents: vec![], author: "me".into(), created_at: 0, message: "m".into() })).await.unwrap();
        repo.refs.create_timeline(RefName::new("public/main").unwrap(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(RefName::new("secret/x").unwrap(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.acls.set_timeline_acl(TimelineAcl { pattern: "secret/**".into() }).unwrap();

        // An accessor cleared only for the public timeline sees just that one.
        let limited = Accessor::new().with_timeline_role(TimelineRole { pattern: "public/**".into(), permission: Permission::Read });
        let b = repo.profile_bundle(&me.public_key(), &limited).await.unwrap();
        let names: Vec<&str> = b.timelines.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"public/main"));
        assert!(!names.contains(&"secret/x"), "protected timeline must not leak into the bundle: {names:?}");

        // A privileged (read-all) accessor sees both.
        let b2 = repo.profile_bundle(&me.public_key(), &Accessor::privileged()).await.unwrap();
        assert_eq!(b2.timelines.len(), 2);
    }

    // bole-k93a
    #[tokio::test]
    async fn bundle_peer_from_cache_no_timelines() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};

        let repo = Repository::memory();
        let peer = CollabSigner::from_seed([4u8; 32]);
        let y = CollabSigner::from_seed([5u8; 32]);
        cache_peer(&repo, &peer.public_key(), CollabObject::Profile(peer.sign_profile("Peer".into(), String::new(), vec![], vec![], 1)), "profile").await;
        cache_peer(&repo, &peer.public_key(), CollabObject::TrustEdge(peer.sign_edge(y.public_key(), TrustKind::Follow, None, 1)), "edge/follow/y").await;

        let b = repo.profile_bundle(&peer.public_key(), &crate::acl::Accessor::privileged()).await.unwrap();
        assert!(!b.is_local);
        assert_eq!(b.profile.as_ref().unwrap().display_name, "Peer");
        assert_eq!(b.edges.len(), 1);
        assert!(b.timelines.is_empty(), "peers get no timelines");
    }

    // bole-k93a
    #[tokio::test]
    async fn bundle_unknown_key_is_empty() {
        use crate::collab::CollabSigner;
        let repo = Repository::memory();
        let ghost = CollabSigner::from_seed([6u8; 32]);
        let b = repo.profile_bundle(&ghost.public_key(), &crate::acl::Accessor::privileged()).await.unwrap();
        assert!(!b.is_local);
        assert!(b.profile.is_none());
        assert!(b.edges.is_empty());
        assert!(b.timelines.is_empty());
    }

    // bole-k93a
    #[tokio::test]
    async fn bundle_out_edges_only() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
        let repo = Repository::memory();
        let me = CollabSigner::from_seed([7u8; 32]);
        let other = CollabSigner::from_seed([8u8; 32]);
        repo.publish_profile(&me.sign_profile("Me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        // An IN-edge (other -> me) cached under other's remotes prefix must not
        // appear in me's bundle: me authored no out-edge.
        cache_peer(&repo, &other.public_key(), CollabObject::TrustEdge(other.sign_edge(me.public_key(), TrustKind::Follow, None, 1)), "edge/follow/me").await;
        let b = repo.profile_bundle(&me.public_key(), &crate::acl::Accessor::privileged()).await.unwrap();
        assert!(b.edges.iter().all(|e| e.from_key == me.public_key()), "only out-edges");
        assert!(b.edges.is_empty());
    }

    // bole-k93a
    #[tokio::test]
    async fn bundle_drops_unverifiable() {
        use crate::collab::{CollabObject, CollabSigner, TrustKind};
        let repo = Repository::memory();
        let peer = CollabSigner::from_seed([9u8; 32]);
        let y = CollabSigner::from_seed([10u8; 32]);
        // Cache a TAMPERED peer profile + out-edge (seq mutated after signing).
        let mut bad_p = peer.sign_profile("Peer".into(), String::new(), vec![], vec![], 1);
        bad_p.seq = 999;
        let mut bad_e = peer.sign_edge(y.public_key(), TrustKind::Follow, None, 1);
        bad_e.seq = 999;
        cache_peer(&repo, &peer.public_key(), CollabObject::Profile(bad_p), "profile").await;
        cache_peer(&repo, &peer.public_key(), CollabObject::TrustEdge(bad_e), "edge/follow/y").await;

        let b = repo.profile_bundle(&peer.public_key(), &crate::acl::Accessor::privileged()).await.unwrap();
        assert!(b.profile.is_none(), "tampered profile dropped -> None");
        assert!(b.edges.is_empty(), "tampered edge dropped");
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

    // bole-581
    #[tokio::test]
    async fn query_resolves_vouch_petname() {
        use crate::collab::{fingerprint, CollabObject, CollabSigner, TrustKind};
        use crate::object::Object;
        use crate::refs::{Ref, RefName, Tag};
        use crate::repo::collab::COLLAB_REMOTES_PREFIX;

        let repo = Repository::memory();
        let me = CollabSigner::from_seed([80u8; 32]);
        let b = CollabSigner::from_seed([81u8; 32]);
        // me follows b AND vouches b as "bee".
        repo.publish_profile(&me.sign_profile("me".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Follow, None, 1)).await.unwrap();
        repo.publish_edge(&me.sign_edge(b.public_key(), TrustKind::Vouch, Some("bee".into()), 1)).await.unwrap();
        // b's profile cached.
        let bp = b.sign_profile("bob-selfname".into(), String::new(), vec![], vec![], 1);
        let id = repo.objects.put(&Object::Collab(CollabObject::Profile(bp))).await.unwrap();
        let bfp = fingerprint(&b.public_key());
        let mut tx = repo.refs.transaction();
        tx.set(RefName::new(format!("{COLLAB_REMOTES_PREFIX}{bfp}/profile")).unwrap(),
               Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit().unwrap();

        let hits = repo.query_discovery(&me.public_key(), 2, "bob-selfname").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display_name, "bob-selfname", "self-asserted name is a hint");
        assert_eq!(hits[0].petname.as_deref(), Some("bee"), "trust-graph petname resolved");
        assert_eq!(hits[0].reach, 1, "direct follow");
        assert_eq!(hits[0].trust_path, vec![me.public_key(), b.public_key()]);
    }

    // bole-581
    #[tokio::test]
    async fn query_reach_and_path() {
        use crate::collab::CollabSigner;
        let repo = Repository::memory();
        let me = CollabSigner::from_seed([82u8; 32]);
        repo.publish_profile(&me.sign_profile("myself".into(), String::new(), vec![], vec![], 1)).await.unwrap();
        let hits = repo.query_discovery(&me.public_key(), 2, "myself").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].reach, 0, "own profile is self");
        assert_eq!(hits[0].petname, None, "no vouch for self -> fingerprint fallback -> None");
    }

    // bole-su8
    #[tokio::test]
    async fn relay_pin_crud_and_upsert() {
        use crate::collab::RelayPin;
        let repo = Repository::memory();
        let key = [9u8; 32];
        assert!(repo.relays().await.unwrap().is_empty());

        repo.add_relay(RelayPin { key, endpoint: "a:1".into() }).await.unwrap();
        assert_eq!(repo.relays().await.unwrap(), vec![RelayPin { key, endpoint: "a:1".into() }]);

        // Upsert: same key, new endpoint -> still one entry, endpoint replaced.
        repo.add_relay(RelayPin { key, endpoint: "b:2".into() }).await.unwrap();
        assert_eq!(repo.relays().await.unwrap(), vec![RelayPin { key, endpoint: "b:2".into() }]);

        assert!(repo.remove_relay(&key).await.unwrap(), "removed an existing pin");
        assert!(!repo.remove_relay(&key).await.unwrap(), "removing absent pin returns false");
        assert!(repo.relays().await.unwrap().is_empty());
    }
}
