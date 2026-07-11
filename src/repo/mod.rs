// bole-1vi
pub mod materialize;
// bole-l0i
pub mod workspace;
// bole-9lj
pub mod merge;
// bole-6bd
pub mod git_projection;
// bole-mtq
pub mod git_import;
// bole-uxt
pub mod ephemeral;
// bole-18p
pub mod collab;

// bole-1vi
use std::collections::BTreeMap;
use std::path::Path;
use crate::acl::disk::DiskAclBackend;
use crate::acl::memory::MemoryAclBackend;
use crate::acl::{Accessor, AclStore, PathAcl};
// bole-fo2
use crate::acl::ResourceRef;
use crate::acl::attestation::{ApproverRegistry, Attestation};
use crate::acl::hook::{PolicyContext, PolicyDecision, PolicyEvent, PolicyRegistry};
use crate::acl::policy_object::PolicyObject;
use crate::acl::lattice::LabelLattice;
use crate::acl::rules::LabelRuleSet;
use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId};
// bole-u6p
use merge::{MergeResult, three_way_diff, find_common_ancestor as lca};
use crate::refs::Ref;
// bole-l0i
use crate::object::EnvValue;
// bole-9mz
use crate::crypto::key_provider::{KeyProvider, ProviderChain};
use workspace::WorkspaceView;
// bole-3w9
use crate::refs::{DiskRefBackend, MemoryRefBackend, RefName, RefStore};
use crate::store::{memory::MemoryBackend, ObjectStore};
// bole-81z
use crate::store::packed::PackedDiskBackend;

// bole-9by
// bole-p8u
/// A snapshot projected through an [`Accessor`]'s path ACL, containing only
/// the paths the accessor is permitted to see.
///
/// Returned by [`Repository::get_snapshot_filtered`]; callers should use
/// `visible_paths` rather than walking the full object tree directly.
#[derive(Debug, Clone)]
pub struct FilteredSnapshot {
    // bole-p8u
    /// The `ObjectId` of the underlying [`Snapshot`](crate::object::Snapshot).
    pub id: ObjectId,
    // bole-p8u
    /// Author field copied from the underlying snapshot.
    pub author: String,
    // bole-p8u
    /// Creation timestamp copied from the underlying snapshot.
    pub created_at: u64,
    // bole-p8u
    /// Commit message copied from the underlying snapshot.
    pub message: String,
    // bole-p8u
    /// Parent snapshot ids copied from the underlying snapshot.
    pub parents: Vec<ObjectId>,
    // bole-p8u
    /// Flat map of logical path → blob `ObjectId` for every path the accessor may read.
    pub visible_paths: BTreeMap<String, ObjectId>,
}

// bole-9by
// bole-p8u
/// The outcome of a pre-merge ACL check performed by [`Repository::check_merge`].
#[derive(Debug, Clone, PartialEq)]
pub enum MergeCheck {
    // bole-p8u
    /// The merge may proceed without restriction.
    Allowed,
    // bole-p8u
    /// The merge would expose protected paths; an explicit write-capable accessor may still proceed.
    RequiresApproval(Vec<PathAcl>),
    // bole-p8u
    /// The merge is denied because the accessor lacks write access to the target timeline.
    Rejected(Vec<PathAcl>),
}

// bole-7rn
/// One capability's decision within an [`AccessExplanation`]: the verdict, a
/// human-readable reason, and the per-clearance evaluation behind it.
#[derive(Debug, Clone)]
pub struct Decision {
    /// Whether the capability is granted.
    pub allowed: bool,
    /// A short human-readable justification for the verdict.
    pub reason: String,
    /// Set when a dominating write clearance existed but the confined
    /// no-write-down rule refused the write.
    pub confined_write_down_block: bool,
    /// The per-clearance evaluation trace (empty when the decision was made by a
    /// repo-level short-circuit rather than a clearance).
    pub clearances: Vec<crate::acl::ClearanceEval>,
}

// bole-7rn
/// A full decision trace for `(actor, snapshot, path)`: whether the path exists
/// in the snapshot, its effective label and the rules that set it, and the read
/// and write decisions with their reasons. Answers "why can/can't this actor
/// see or write this path?" from the same logic the enforcement path uses.
#[derive(Debug, Clone)]
pub struct AccessExplanation {
    /// The path that was explained.
    pub path: String,
    /// Whether the path is present in the snapshot's (unfiltered) tree.
    pub present: bool,
    /// The path's effective label (JOIN of matching rules; bottom if none).
    pub label: crate::acl::lattice::Label,
    /// The globs of the label rules that contributed to `label`.
    pub matched_rules: Vec<String>,
    /// The read decision.
    pub read: Decision,
    /// The write decision.
    pub write: Decision,
}

// bole-1vi
// bole-p8u
/// The top-level handle to a bole repository, bundling object storage, ref
/// storage, and ACL storage behind a single unified API.
///
/// Construct one with [`Repository::memory`] (for ephemeral/test use) or
/// [`Repository::disk`] (for persistent storage on the local filesystem).
pub struct Repository {
    // bole-p8u
    /// The content-addressed store for all blobs, trees, snapshots, secrets, and overlays.
    pub objects: ObjectStore,
    // bole-p8u
    /// The store for all named refs (tags and timelines).
    pub refs: RefStore,
    // bole-9by
    // bole-p8u
    /// The store for path and timeline ACL protection rules.
    pub acls: AclStore,
    // bole-fo2
    /// Declarative policy hook bindings resolved into the registry per call.
    hooks: Vec<crate::acl::policy_object::HookSpec>,
    // bole-eul
    /// Serializes collab publish read-check-write so concurrent publishes cannot
    /// both pass a stale monotonic-seq check (WS8b F4).
    publish_lock: tokio::sync::Mutex<()>,
}

// bole-1vi
impl Repository {
    // bole-p8u
    /// Creates a fully in-memory repository; all data is lost when dropped.
    ///
    /// Useful for tests and short-lived operations.
    pub fn memory() -> Self {
        Self {
            objects: ObjectStore::new(MemoryBackend::new()),
            refs: RefStore::new(MemoryRefBackend::new()),
            // bole-9by
            acls: AclStore::new(MemoryAclBackend::new()),
            // bole-fo2
            hooks: Vec::new(),
            // bole-eul
            publish_lock: tokio::sync::Mutex::new(()),
        }
    }

    // bole-p8u
    /// Opens (or creates) a persistent repository rooted at the given directory.
    pub async fn disk(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            // bole-81z: packed backend (loose-first + packs); loose-only repos
            // with no packs/ behave exactly as before.
            objects: ObjectStore::new(PackedDiskBackend::open(root).await?),
            refs: RefStore::new(DiskRefBackend::open(root)?),
            // bole-9by
            acls: AclStore::new(DiskAclBackend::open(root)?),
            // bole-fo2
            hooks: Vec::new(),
            // bole-eul
            publish_lock: tokio::sync::Mutex::new(()),
        })
    }

    // bole-fo2
    /// Registers a declarative policy hook binding.
    pub fn register_hook(&mut self, spec: crate::acl::policy_object::HookSpec) {
        self.hooks.push(spec);
    }

    // bole-sk6
    /// Begins an atomic multi-ref transaction over this repository's refs.
    pub fn transaction(&self) -> crate::refs::RefTransaction<'_> {
        self.refs.transaction()
    }

    // bole-fo2
    /// Builds the active policy registry: the built-in `TimelinePolicyHook` plus
    /// every resolved declarative hook (fail-closed on unknown kinds).
    // bole-7c1: pub(crate) so the sync push-acceptance path can assert the bound
    // policy is deterministic before letting it gate a replicated advance.
    pub(crate) async fn policy_registry(&self) -> Result<PolicyRegistry> {
        let mut reg = PolicyRegistry::new();
        for spec in &self.hooks {
            reg.push(crate::acl::hook::resolve_hook(spec)?);
        }
        // bole-au0t: hooks declared by the pinned, replicated policy root bind
        // too. resolve_hook rejects unknown kinds, so a replica that lacks a
        // kind named in a synced root refuses the operation (WS1-O5).
        if let Some((_, root)) = self.policy_root().await? {
            for spec in &root.hooks {
                reg.push(crate::acl::hook::resolve_hook(spec)?);
            }
        }
        Ok(reg)
    }

    // bole-au0t
    /// Pins `root` as the active policy root: stores it content-addressed and
    /// points `refs/policy/root` at it. On any node that has pinned a root, its
    /// hooks bind through [`Repository::policy_registry`] — a node that cannot
    /// resolve one of the root's hook kinds refuses advance/merge/replicated
    /// push fail-closed (WS1-O5). The root's object closure transfers via sync,
    /// but a replica enforces it only after pinning it locally: adoption of a
    /// fetched root is an explicit step (verified adoption via `verify_chain`
    /// is future work), never a side effect of fetch/clone. Returns the root's
    /// object id.
    pub async fn set_policy_root(&self, root: &crate::acl::policy_object::PolicyRoot) -> Result<ObjectId> {
        let id = self
            .objects
            .put(&Object::Policy(PolicyObject::Root(root.clone())))
            .await?;
        let name = RefName::new(crate::acl::policy_object::POLICY_ROOT_REF)?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(crate::refs::Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-au0t
    /// Loads the pinned policy root (`None` if the repo has never pinned one).
    /// Fail-closed: a `refs/policy/root` that does not point at a stored
    /// `PolicyObject::Root` is an error, not an absent policy.
    pub async fn policy_root(&self) -> Result<Option<(ObjectId, crate::acl::policy_object::PolicyRoot)>> {
        let name = RefName::new(crate::acl::policy_object::POLICY_ROOT_REF)?;
        let target = match self.refs.get(&name)? {
            Some(Ref::Tag(t)) => t.target,
            Some(Ref::Timeline(_)) => {
                return Err(Error::PolicyViolation(
                    "refs/policy/root is a timeline, not a policy-root tag (fail-closed)".into(),
                ))
            }
            None => return Ok(None),
        };
        match self.objects.get(&target).await? {
            Some(Object::Policy(PolicyObject::Root(r))) => Ok(Some((target, r))),
            _ => Err(Error::PolicyViolation(
                "refs/policy/root does not point at a stored policy root (fail-closed)".into(),
            )),
        }
    }

    // bole-6i7
    /// Stores/overwrites the content-addressed approver registry — the set of
    /// keys whose signatures a `signed-approval` hook will accept — pinned by the
    /// `refs/policy/approvers` ref.
    pub async fn set_approvers(&self, registry: &ApproverRegistry) -> Result<()> {
        let id = self
            .objects
            .put(&Object::Policy(PolicyObject::Approvers(registry.clone())))
            .await?;
        let name = RefName::new(crate::acl::attestation::APPROVERS_REF)?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(crate::refs::Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(())
    }

    // bole-6i7
    /// Loads the approver registry (empty if none has been set).
    pub async fn approvers(&self) -> Result<ApproverRegistry> {
        crate::acl::attestation::load_approvers(&self.refs, &self.objects).await
    }

    // bole-6i7
    /// Stores a signed [`Attestation`] as a content-addressed object, pinned by a
    /// `refs/attestations/<object-id>` ref. Idempotent (the ref name is the
    /// object id). Returns the attestation's object id.
    pub async fn add_attestation(&self, att: &Attestation) -> Result<ObjectId> {
        let id = self
            .objects
            .put(&Object::Policy(PolicyObject::Attestation(att.clone())))
            .await?;
        let leaf = format!("{}{}", crate::acl::attestation::ATTESTATIONS_PREFIX, id);
        let name = RefName::new(leaf)?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(crate::refs::Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-6i7
    /// Loads every stored attestation.
    pub async fn attestations(&self) -> Result<Vec<Attestation>> {
        crate::acl::attestation::load_attestations(&self.refs, &self.objects).await
    }

    // bole-p8u
    /// Copies all objects and refs from `self` into `dest`.
    pub async fn copy_to(&self, dest: &Repository) -> Result<()> {
        copy_objects(&self.objects, &dest.objects).await?;
        copy_refs(&self.refs, &dest.refs)?;
        Ok(())
    }

    // bole-9by
    // bole-p8u
    /// Loads the snapshot at `id` and filters its tree to only the paths the `accessor`
    /// is permitted to see, returning a [`FilteredSnapshot`].
    ///
    /// Returns `None` if no snapshot exists at `id`.
    pub async fn get_snapshot_filtered(
        &self,
        id: ObjectId,
        accessor: &Accessor,
    ) -> Result<Option<FilteredSnapshot>> {
        let snap = match self.objects.get(&id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Ok(None),
        };
        let mut visible_paths = BTreeMap::new();
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        walk_tree_filtered(&self.objects, &lattice, &rules, snap.root, "", accessor, &mut visible_paths).await?;
        Ok(Some(FilteredSnapshot {
            id,
            author: snap.author,
            created_at: snap.created_at,
            message: snap.message,
            parents: snap.parents,
            visible_paths,
        }))
    }

    // bole-9by
    // bole-p8u
    /// Lists all refs under `prefix`, omitting any protected timelines the `accessor`
    /// cannot read.
    pub fn list_refs_filtered(&self, prefix: &str, accessor: &Accessor) -> Result<Vec<RefName>> {
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        let all = self.refs.list(prefix)?;
        let mut out = Vec::new();
        for name in all {
            let label = rules.label_for_timeline(&lattice, name.as_str());
            if label == lattice.bottom()
                || accessor.can_read(&label, ResourceRef::Timeline(name.as_str()))
            {
                out.push(name);
            }
        }
        Ok(out)
    }

    // bole-e78l
    /// [`Repository::list_refs_filtered`] minus namespaces that never travel a
    /// serve path: `refs/collab/scoped/**` stays out of every peer-facing
    /// enumeration regardless of labels (an unlabeled ref defaults to the
    /// lattice bottom, i.e. world-readable, so a label check alone cannot gate
    /// it). Every serve-side ref enumeration — wire adverts, in-process fetch,
    /// and the HTTP API — must use this, not `list_refs_filtered` (WS8b M2).
    pub fn list_refs_served(&self, prefix: &str, accessor: &Accessor) -> Result<Vec<RefName>> {
        Ok(self
            .list_refs_filtered(prefix, accessor)?
            .into_iter()
            .filter(|n| !n.as_str().starts_with(collab::COLLAB_SCOPED_PREFIX))
            .collect())
    }

    // bole-tgr8
    /// Point-lookup twin of [`Repository::list_refs_served`]: whether `name`
    /// would appear in that listing, without the O(all-refs) scan. Same label
    /// gate as `list_refs_filtered` and the same structural scoped-collab
    /// exclusion — keep the three in lockstep.
    pub fn ref_served(&self, name: &RefName, accessor: &Accessor) -> Result<bool> {
        if name.as_str().starts_with(collab::COLLAB_SCOPED_PREFIX) {
            return Ok(false);
        }
        if self.refs.get(name)?.is_none() {
            return Ok(false);
        }
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        let label = rules.label_for_timeline(&lattice, name.as_str());
        Ok(label == lattice.bottom()
            || accessor.can_read(&label, ResourceRef::Timeline(name.as_str())))
    }

    // bole-9by
    // bole-p8u
    /// Checks whether merging `source` into `dest` is safe with respect to path ACLs,
    /// without actually performing the merge.
    ///
    /// Returns [`MergeCheck::Allowed`] when no protected paths would be exposed,
    /// [`MergeCheck::RequiresApproval`] when a write-capable accessor could override,
    /// or [`MergeCheck::Rejected`] when the accessor lacks the required write access.
    pub async fn check_merge(
        &self,
        source: &RefName,
        dest: &RefName,
        accessor: &Accessor,
    ) -> Result<MergeCheck> {
        // bole-938
        let source_head = match self.refs.get_timeline(source)? {
            Some(tl) => tl.head,
            None => return Err(Error::Storage(format!("source ref '{}' not found", source.as_str()))),
        };
        // bole-4j3
        let source_tree = match self.objects.get(&source_head).await? {
            Some(Object::Snapshot(s)) => s.root,
            _ => return Err(Error::WrongRefKind(format!("source ref '{}' head is not a snapshot", source.as_str()))),
        };
        let mut visible = BTreeMap::new();
        // bole-hc1
        // bole-g21
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        walk_tree_filtered(&self.objects, &lattice, &rules, source_tree, "", &Accessor::privileged(), &mut visible).await?;
        // bole-fo2
        // A path leaks if its effective label is NOT dominated by the dest
        // timeline's effective label — i.e. content would flow to a strictly
        // less-protected place. Reported as the matching PathAcl(s), as before.
        let dest_label = rules.label_for_timeline(&lattice, dest.as_str());
        let mut leaking: Vec<PathAcl> = Vec::new();
        let path_acls = self.acls.list_path_acls()?;
        for acl in &path_acls {
            let any_leak = visible.keys().any(|p| {
                crate::acl::glob::glob_matches(&acl.glob, p)
                    && {
                        let plabel = rules.label_for_path(&lattice, p);
                        !lattice.dominates(&dest_label, &plabel)
                    }
            });
            if any_leak && !leaking.iter().any(|l| l.glob == acl.glob) {
                leaking.push(acl.clone());
            }
        }
        // bole-fo2
        // After the leak scan, run registered Merge hooks (most restrictive wins).
        let registry = self.policy_registry().await?;
        let ctx = PolicyContext {
            event: PolicyEvent::Merge {
                source,
                target: dest,
                old_head: source_head,
                result_head: source_head,
            },
            accessor,
            objects: &self.objects,
            refs: &self.refs,
            now: 0,
        };
        let hook_decision = registry.evaluate(&ctx).await;

        if let PolicyDecision::Deny(_) = hook_decision {
            return Ok(MergeCheck::Rejected(leaking));
        }
        if leaking.is_empty() {
            match hook_decision {
                PolicyDecision::RequiresApproval { .. } => Ok(MergeCheck::RequiresApproval(leaking)),
                _ => Ok(MergeCheck::Allowed),
            }
        } else if accessor.can_write_timeline(dest.as_str()) {
            Ok(MergeCheck::RequiresApproval(leaking))
        } else {
            Ok(MergeCheck::Rejected(leaking))
        }
    }

    // bole-u6p
    // bole-p8u
    /// Finds the nearest common ancestor snapshot of `a` and `b` in the snapshot DAG,
    /// or `None` if the histories share no common point.
    pub async fn find_common_ancestor(&self, a: ObjectId, b: ObjectId) -> Result<Option<ObjectId>> {
        lca(&self.objects, a, b).await
    }

    // bole-p8u
    /// Computes a three-way diff between `source` and `target` using their common ancestor,
    /// returning a [`MergeResult`] that describes conflicts and clean changes.
    ///
    /// The caller is responsible for checking write permissions before calling this
    /// (or use [`Repository::check_merge`] first).  The `accessor` must hold write
    /// access to `target`.
    pub async fn merge_timelines(
        &self,
        source: &RefName,
        target: &RefName,
        accessor: &Accessor,
    ) -> Result<MergeResult> {
        if !accessor.can_write_timeline(target.as_str()) {
            return Err(Error::AccessDenied(format!(
                "write denied on timeline: {}",
                target.as_str()
            )));
        }
        let source_tl = self.refs.get_timeline(source)?.ok_or_else(|| {
            Error::Storage(format!("timeline not found: {}", source.as_str()))
        })?;
        let target_tl = self.refs.get_timeline(target)?.ok_or_else(|| {
            Error::Storage(format!("timeline not found: {}", target.as_str()))
        })?;
        let ancestor_id = lca(&self.objects, source_tl.head, target_tl.head).await?;
        let source_root = match self.objects.get(&source_tl.head).await? {
            Some(Object::Snapshot(s)) => s.root,
            _ => return Err(Error::Storage(format!("snapshot not found: {}", source_tl.head))),
        };
        let target_root = match self.objects.get(&target_tl.head).await? {
            Some(Object::Snapshot(s)) => s.root,
            _ => return Err(Error::Storage(format!("snapshot not found: {}", target_tl.head))),
        };
        let ancestor_tree = match ancestor_id {
            Some(id) => match self.objects.get(&id).await? {
                Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?,
                _ => BTreeMap::new(),
            },
            None => BTreeMap::new(),
        };
        let source_tree = self.tree_as_map(source_root).await?;
        let target_tree = self.tree_as_map(target_root).await?;
        // ours = target (being merged into), theirs = source
        Ok(three_way_diff(&ancestor_tree, &target_tree, &source_tree))
    }

    // bole-7rn
    /// Explains an actor's read and write access to `path` at `snapshot`,
    /// returning a full decision trace rather than a bare boolean.
    ///
    /// The verdicts match enforcement exactly: read applies the same
    /// public/bottom short-circuit as [`Repository::get_snapshot_filtered`]'s
    /// tree walk, and write mirrors [`Repository::advance_timeline`]'s per-path
    /// check. The trace exposes the effective label, the rules that set it, and
    /// every clearance the actor holds — the answer to "why is this hidden?".
    pub async fn explain_path(
        &self,
        accessor: &Accessor,
        snapshot: ObjectId,
        path: &str,
    ) -> Result<AccessExplanation> {
        use crate::acl::glob::glob_matches;

        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;

        // Effective label + the Path rules that contributed to it.
        let label = rules.label_for_path(&lattice, path);
        let matched_rules: Vec<String> = rules
            .rules
            .iter()
            .filter_map(|r| match r {
                crate::acl::rules::LabelRule::Path { glob, .. } if glob_matches(glob, path) => {
                    Some(glob.clone())
                }
                _ => None,
            })
            .collect();

        // Is the path actually present in the snapshot's unfiltered tree?
        let present = match self.objects.get(&snapshot).await? {
            Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?.contains_key(path),
            _ => return Err(Error::Storage(format!("snapshot not found: {snapshot}"))),
        };

        let (read_trace, write_trace) = accessor.explain(&label, ResourceRef::Path(path));
        let is_public = label == lattice.bottom();

        // Read mirrors walk_tree_filtered: bottom-labelled paths are visible to
        // everyone; otherwise a clearance must grant it.
        let read = if is_public {
            Decision {
                allowed: true,
                reason: "path label is public (lattice bottom); readable by all actors".into(),
                confined_write_down_block: false,
                clearances: read_trace.clearances,
            }
        } else if read_trace.allowed {
            Decision {
                allowed: true,
                reason: format!("granted: a read clearance dominates label `{}`", label.0),
                confined_write_down_block: false,
                clearances: read_trace.clearances,
            }
        } else {
            Decision {
                allowed: false,
                reason: format!(
                    "denied: no in-scope read clearance dominates label `{}`",
                    label.0
                ),
                confined_write_down_block: false,
                clearances: read_trace.clearances,
            }
        };

        // Write mirrors advance_timeline's per-path check: no public
        // short-circuit — an explicit write clearance is always required.
        let write = if write_trace.allowed {
            Decision {
                allowed: true,
                reason: format!("granted: a write clearance dominates label `{}`", label.0),
                confined_write_down_block: false,
                clearances: write_trace.clearances,
            }
        } else if write_trace.confined_write_down_block {
            Decision {
                allowed: false,
                reason: format!(
                    "denied: confined actor may not write down to label `{}`",
                    label.0
                ),
                confined_write_down_block: true,
                clearances: write_trace.clearances,
            }
        } else {
            Decision {
                allowed: false,
                reason: format!(
                    "denied: no in-scope write clearance dominates label `{}`",
                    label.0
                ),
                confined_write_down_block: false,
                clearances: write_trace.clearances,
            }
        };

        Ok(AccessExplanation { path: path.to_string(), present, label, matched_rules, read, write })
    }

    // bole-p8u
    /// Moves the head of timeline `name` to `snapshot_id`, enforcing both timeline-level
    /// and path-level write permissions from `accessor`.
    pub async fn advance_timeline(
        &self,
        name: &RefName,
        snapshot_id: ObjectId,
        accessor: &Accessor,
    ) -> Result<()> {
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        // Timeline write check via the dominance rule.
        let tl_label = rules.label_for_timeline(&lattice, name.as_str());
        if !accessor.can_write(&tl_label, ResourceRef::Timeline(name.as_str())) {
            return Err(Error::AccessDenied(format!(
                "write denied on timeline: {}",
                name.as_str()
            )));
        }
        let snap = match self.objects.get(&snapshot_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Err(Error::Storage(format!("snapshot not found: {}", snapshot_id))),
        };
        // bole-fo2
        // Per-path write check (also enforces the confined no-write-down rule).
        let mut paths = BTreeMap::new();
        walk_tree_filtered(
            &self.objects,
            &lattice,
            &rules,
            snap.root,
            "",
            &Accessor::privileged(),
            &mut paths,
        )
        .await?;
        for path in paths.keys() {
            let label = rules.label_for_path(&lattice, path);
            if !accessor.can_write(&label, ResourceRef::Path(path)) {
                return Err(Error::AccessDenied(format!("write denied on path: {}", path)));
            }
        }
        // bole-fo2
        // The inline TimelinePolicy match is replaced by the policy registry.
        let timeline = self.refs.get_timeline(name)?.ok_or_else(|| {
            Error::Storage(format!("timeline not found: {}", name.as_str()))
        })?;
        // bole-48r
        // Deleting a path is a write to that path, but walk_tree_filtered over the
        // NEW tree cannot see a path that is no longer there. Enumerate the OLD
        // head's paths and require write on every one that the new snapshot
        // removes, so an actor cannot drop protected paths they cannot write.
        let old_paths = match self.objects.get(&timeline.head).await? {
            Some(Object::Snapshot(s)) => self.tree_as_map(s.root).await?,
            _ => BTreeMap::new(),
        };
        for path in old_paths.keys() {
            if !paths.contains_key(path) {
                let label = rules.label_for_path(&lattice, path);
                if !accessor.can_write(&label, ResourceRef::Path(path)) {
                    return Err(Error::AccessDenied(format!(
                        "write denied on removed path: {}",
                        path
                    )));
                }
            }
        }
        let registry = self.policy_registry().await?;
        let ctx = PolicyContext {
            event: PolicyEvent::Advance {
                timeline: name,
                old_head: timeline.head,
                new_head: snapshot_id,
            },
            accessor,
            objects: &self.objects,
            refs: &self.refs,
            now: 0,
        };
        match registry.evaluate(&ctx).await {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny(reason) => return Err(Error::PolicyViolation(reason)),
            PolicyDecision::RequiresApproval { reason, .. } => {
                return Err(Error::PolicyViolation(reason))
            }
        }
        // bole-qj4: commit via a compare-and-swap on the head we read and
        // evaluated policy against (timeline.head), not an unconditional
        // advance_head. If a concurrent writer moved the head since that read,
        // the CAS fails (TransactionConflict) rather than silently clobbering the
        // winner and having accepted this advance on stale lineage. Serialized by
        // the RefStore commit lock (bole-bti).
        let mut tx = self.refs.transaction();
        tx.advance_head_if(name.clone(), timeline.head, snapshot_id);
        tx.commit()?;
        Ok(())
    }

    // bole-p8u
    /// Deletes timeline `name` if it has an `expires_at` timestamp that is ≤ `now`
    /// and no tag points at its head snapshot.
    ///
    /// Returns `true` if the timeline was pruned, `false` otherwise.
    pub fn prune_timeline(&self, name: &RefName, now: u64) -> Result<bool> {
        let tl = match self.refs.get_timeline(name)? {
            Some(t) => t,
            None => return Ok(false),
        };
        match tl.expires_at {
            Some(exp) if exp <= now => {}
            _ => return Ok(false),
        }
        for ref_name in self.refs.list("")? {
            if let Some(Ref::Tag(tag)) = self.refs.get(&ref_name)? {
                if tag.target == tl.head {
                    return Ok(false);
                }
            }
        }
        self.refs.delete_ref(name)?;
        Ok(true)
    }

    async fn tree_as_map(&self, tree_id: ObjectId) -> Result<BTreeMap<String, ObjectId>> {
        let mut map = BTreeMap::new();
        // bole-fo2
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        walk_tree_filtered(
            &self.objects,
            &lattice,
            &rules,
            tree_id,
            "",
            &Accessor::privileged(),
            &mut map,
        )
        .await?;
        Ok(map)
    }

    // bole-l0i
    // bole-p8u
    /// Combines a filtered snapshot view with a decrypted env overlay into a
    /// [`WorkspaceView`] — the complete set of files and resolved environment
    /// variables an accessor can see.
    ///
    /// Returns `None` if the snapshot does not exist.
    pub async fn compute_workspace_view(
        &self,
        snapshot_id: ObjectId,
        overlay_id: ObjectId,
        key: &[u8; 32],
        accessor: &Accessor,
    ) -> Result<Option<WorkspaceView>> {
        let filtered = match self.get_snapshot_filtered(snapshot_id, accessor).await? {
            Some(f) => f,
            None => return Ok(None),
        };
        let overlay = match self.objects.get_overlay(&overlay_id).await? {
            Some(o) => o,
            None => return Err(crate::error::Error::Storage(
                format!("overlay not found: {}", overlay_id)
            )),
        };
        let mut env = std::collections::BTreeMap::new();
        for (var, value) in overlay.entries {
            let resolved = match value {
                EnvValue::Plain(s) => s,
                EnvValue::Secret(id) => {
                    let bytes = self.objects.get_secret(&id, key).await?
                        .ok_or_else(|| crate::error::Error::Storage(
                            format!("secret not found: {}", id)
                        ))?;
                    String::from_utf8(bytes)
                        .map_err(|_| crate::error::Error::SecretNotUtf8)?
                }
            };
            env.insert(var, resolved);
        }
        Ok(Some(WorkspaceView { files: filtered.visible_paths, env }))
    }

    // bole-9mz
    /// Resolve an overlay to a concrete environment, decrypting `Secret` refs
    /// through `chain`. Access-checked per WS1: each secret entry is gated by the
    /// effective label of its env-var name (public/bottom is readable by all;
    /// otherwise the actor must `can_read` it). Fails closed on an uncleared
    /// secret with `Error::AccessDenied` naming the var but never the value —
    /// unless `skip_unauthorized`, which omits the var instead. Plain entries are
    /// always included. Non-UTF-8 secret bytes → `Error::Codec`.
    pub async fn resolve_overlay(
        &self,
        overlay_id: &ObjectId,
        chain: &ProviderChain,
        accessor: &Accessor,
        skip_unauthorized: bool,
    ) -> Result<BTreeMap<String, String>> {
        let overlay = self
            .objects
            .get_overlay(overlay_id)
            .await?
            .ok_or_else(|| Error::Storage(format!("overlay not found: {overlay_id}")))?;
        let lattice = self.acls.lattice()?;
        let rules = self.acls.label_ruleset()?;
        let mut out = BTreeMap::new();
        for (var, value) in overlay.entries {
            match value {
                EnvValue::Plain(s) => {
                    out.insert(var, s);
                }
                EnvValue::Secret(id) => {
                    let label = rules.label_for_secret(&lattice, &var);
                    let cleared = label == lattice.bottom()
                        || accessor.can_read(&label, ResourceRef::Secret(&var));
                    if !cleared {
                        if skip_unauthorized {
                            continue;
                        }
                        return Err(Error::AccessDenied(format!(
                            "not cleared to resolve secret for env var '{var}'"
                        )));
                    }
                    let bytes = self
                        .objects
                        .get_secret_resolved(&id, chain)
                        .await?
                        .ok_or_else(|| Error::Storage(format!("secret not found: {id}")))?;
                    let s = String::from_utf8(bytes).map_err(|_| {
                        Error::Codec(format!("secret for env var '{var}' is not valid UTF-8"))
                    })?;
                    out.insert(var, s);
                }
            }
        }
        Ok(out)
    }

    // bole-9mz
    /// Rotate the master key for the given secret objects: re-wrap each v2
    /// secret's data key from `old` to `new` (value bytes untouched), and upgrade
    /// any v1 secret it encounters to v2 under `new`. Returns `(old_id, new_id)`
    /// for each rekeyed secret (a new wrap ⇒ a new `ObjectId`); callers repoint
    /// registries/overlays. Old objects are left for WS4 GC.
    pub async fn rekey(
        &self,
        ids: &[ObjectId],
        old: &ProviderChain,
        new: &dyn KeyProvider,
    ) -> Result<Vec<(ObjectId, ObjectId)>> {
        let mut mapping = Vec::with_capacity(ids.len());
        for id in ids {
            let new_id = match self.objects.get(id).await? {
                Some(Object::SecretV2(s)) => {
                    let rewrapped = s.rewrap(old, new).await?;
                    self.objects.put(&Object::SecretV2(rewrapped)).await?
                }
                Some(Object::Secret(v1)) => {
                    // Legacy upgrade: decrypt via a legacy key, re-encrypt as v2.
                    let mut pt = None;
                    for k in old.legacy_keys() {
                        if let Ok(bytes) = v1.decrypt(k) {
                            pt = Some(bytes);
                            break;
                        }
                    }
                    let pt = pt.ok_or(Error::DecryptionFailed)?;
                    self.objects
                        .put_secret_enveloped(&pt, new, crate::object::SecretAad::v2(None))
                        .await?
                }
                Some(_) => return Err(Error::Codec(format!("not a secret: {id}"))),
                None => return Err(Error::Storage(format!("secret not found: {id}"))),
            };
            mapping.push((*id, new_id));
        }
        Ok(mapping)
    }

    // bole-81z
    /// Mark-and-sweep GC. Roots = every timeline head (all kinds, incl.
    /// ephemeral) and tag target in the ref store, plus `extra_roots` (for
    /// objects rooted outside refs, e.g. CLI-registry secrets/overlays — see
    /// spec O8). Computes the reachable object closure, then rewrites packs and
    /// unlinks unreachable loose objects older than `grace_secs` (relative to the
    /// unix-seconds clock `now`). Returns the number of objects removed. Never
    /// removes a reachable object.
    pub async fn gc(&self, extra_roots: &[ObjectId], grace_secs: u64, now: u64) -> Result<u64> {
        let mut roots: Vec<ObjectId> = extra_roots.to_vec();
        for name in self.refs.list("")? {
            match self.refs.get(&name)? {
                Some(crate::refs::Ref::Timeline(t)) => roots.push(t.head),
                Some(crate::refs::Ref::Tag(t)) => roots.push(t.target),
                None => {}
            }
        }
        let reachable = self.mark_reachable(&roots).await?;
        self.objects.sweep(&reachable, grace_secs, now).await
    }

    // bole-81z
    /// The reachable object closure from `roots`, following the object-graph
    /// edges: Snapshot → {root, parents}, Tree → entry ids, EnvOverlay → secret
    /// refs. Blobs and secrets are leaves. Shared subtrees are visited once.
    async fn mark_reachable(&self, roots: &[ObjectId]) -> Result<std::collections::HashSet<ObjectId>> {
        let mut reachable: std::collections::HashSet<ObjectId> = std::collections::HashSet::new();
        let mut stack: Vec<ObjectId> = roots.to_vec();
        while let Some(id) = stack.pop() {
            if !reachable.insert(id) {
                continue;
            }
            match self.objects.get(&id).await? {
                Some(Object::Snapshot(s)) => {
                    stack.push(s.root);
                    stack.extend(s.parents);
                }
                Some(Object::Tree(t)) => {
                    for e in t.entries.values() {
                        stack.push(e.id);
                    }
                }
                Some(Object::EnvOverlay(o)) => {
                    for v in o.entries.values() {
                        if let EnvValue::Secret(sid) = v {
                            stack.push(*sid);
                        }
                    }
                }
                _ => {} // Blob, Secret, SecretV2, Policy: leaves
            }
        }
        Ok(reachable)
    }
}

// bole-fo2
// Visible iff the path's effective label is the lattice bottom (public — visible
// to all) or the accessor's clearances dominate it in scope. This collapses the
// old `path_is_protected` gate into the dominance check.
// bole-wy4
/// Maximum tree nesting depth walked. A content-addressed object graph cannot
/// contain a cycle (ids are BLAKE3 of content), so the only unbounded-recursion
/// vector is a very deep *chain* of nested trees — cheap to construct
/// (~60 bytes/level, far under the pack byte caps) yet enough to overflow the
/// worker stack into an uncatchable SIGABRT. Past this depth we return an Error
/// instead of recursing, turning a process abort into a handled failure. Real
/// repositories nest far shallower than this.
///
/// The walk itself is iterative (O(1) stack), but this bound also protects any
/// *recursive* consumer of the walk's output — e.g. `git_projection`'s
/// `write_git_tree_level`, which rebuilds nested trees by path depth — since the
/// walk errors before emitting a path deeper than this.
pub(crate) const MAX_TREE_DEPTH: usize = 256;

// bole-wy4
/// Filters a tree into `out` using an explicit heap work-stack rather than
/// recursion, so the call stack stays O(1) regardless of tree nesting — a
/// maliciously deep tree chain can no longer overflow the worker stack into an
/// uncatchable SIGABRT. `MAX_TREE_DEPTH` still bounds total work (the depth is
/// tracked per stack item). Content addressing rules out true cycles, so depth
/// is the only unbounded dimension.
async fn walk_tree_filtered(
    objects: &ObjectStore,
    lattice: &LabelLattice,
    rules: &LabelRuleSet,
    tree_id: ObjectId,
    prefix: &str,
    accessor: &Accessor,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    // (tree id, path prefix, depth)
    let mut stack: Vec<(ObjectId, String, usize)> = vec![(tree_id, prefix.to_string(), 0)];
    while let Some((tid, pfx, depth)) = stack.pop() {
        if depth >= MAX_TREE_DEPTH {
            return Err(Error::Storage(format!(
                "tree nesting exceeds maximum depth {MAX_TREE_DEPTH}"
            )));
        }
        let tree = match objects.get(&tid).await? {
            Some(Object::Tree(t)) => t,
            _ => continue,
        };
        for (name, entry) in &tree.entries {
            let full_path = if pfx.is_empty() { name.clone() } else { format!("{pfx}/{name}") };
            match entry.kind {
                EntryKind::Blob => {
                    let label = rules.label_for_path(lattice, &full_path);
                    if label == lattice.bottom()
                        || accessor.can_read(&label, ResourceRef::Path(&full_path))
                    {
                        out.insert(full_path, entry.id);
                    }
                }
                EntryKind::Tree => {
                    stack.push((entry.id, full_path, depth + 1));
                }
            }
        }
    }
    Ok(())
}

// bole-1vi
// bole-1cq
// Decode + re-encode rather than raw byte copy. Safe because postcard is
// deterministic and BLAKE3 ids are stable, so round-tripping preserves the id.
// If codec versioning ever changes, revisit this.
// bole-p8u
/// Copies every object from `from` into `to`, re-deriving ids to guarantee
/// correctness even if the serialisation format ever changes.
pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()> {
    for id in from.list().await? {
        if let Some(obj) = from.get(&id).await? {
            to.put(&obj).await?;
        }
    }
    Ok(())
}

// bole-1vi
// bole-p8u
/// Copies every ref from `from` into `to`, preserving both tags and timelines.
pub fn copy_refs(from: &RefStore, to: &RefStore) -> Result<()> {
    for name in from.list("")? {
        if let Some(r) = from.get(&name)? {
            to.set_raw(&name, &r)?;
        }
    }
    Ok(())
}

// bole-1vi
#[cfg(test)]
mod tests {
    use super::{copy_objects, copy_refs, Repository};
    use crate::object::ObjectId;
    use crate::refs::{MemoryRefBackend, RefName, RefStore, TimelinePolicy};
    use crate::store::{memory::MemoryBackend, ObjectStore};
    use bytes::Bytes;
    use tempfile::TempDir;

    #[tokio::test]
    async fn memory_repo_has_working_stores() {
        let repo = Repository::memory();
        let id = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
        assert!(repo.objects.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn disk_repo_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = {
            let repo = Repository::disk(dir.path()).await.unwrap();
            repo.objects.put_blob(Bytes::from("persist")).await.unwrap()
        };
        let repo2 = Repository::disk(dir.path()).await.unwrap();
        assert!(repo2.objects.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn copy_objects_copies_all_five() {
        let from = ObjectStore::new(MemoryBackend::new());
        let to = ObjectStore::new(MemoryBackend::new());
        let ids = [
            from.put_blob(Bytes::from("a")).await.unwrap(),
            from.put_blob(Bytes::from("b")).await.unwrap(),
            from.put_blob(Bytes::from("c")).await.unwrap(),
            from.put_blob(Bytes::from("d")).await.unwrap(),
            from.put_blob(Bytes::from("e")).await.unwrap(),
        ];
        copy_objects(&from, &to).await.unwrap();
        for id in &ids {
            assert!(to.exists(id).await.unwrap(), "id {id} missing after copy");
        }
    }

    #[test]
    fn copy_refs_copies_tags_and_timelines() {
        let from = RefStore::new(MemoryRefBackend::new());
        let to = RefStore::new(MemoryRefBackend::new());
        let id = ObjectId::new([1u8; 32]);
        from.create_tag(RefName::new("v1").unwrap(), id, None, 1).unwrap();
        from.create_tag(RefName::new("v2").unwrap(), id, None, 2).unwrap();
        from.create_timeline(RefName::new("main").unwrap(), id, TimelinePolicy::Unrestricted, 3, "persistent".into(), None).unwrap();
        copy_refs(&from, &to).unwrap();
        assert!(to.get(&RefName::new("v1").unwrap()).unwrap().is_some());
        assert!(to.get(&RefName::new("v2").unwrap()).unwrap().is_some());
        assert!(to.get(&RefName::new("main").unwrap()).unwrap().is_some());
    }

    #[tokio::test]
    async fn copy_to_copies_objects_and_refs() {
        let dir = TempDir::new().unwrap();
        let src = Repository::memory();
        let id = src.objects.put_blob(Bytes::from("data")).await.unwrap();
        let tag_name = RefName::new("v1").unwrap();
        src.refs.create_tag(tag_name.clone(), id, None, 1).unwrap();
        let dest = Repository::disk(dir.path()).await.unwrap();
        src.copy_to(&dest).await.unwrap();
        assert!(dest.objects.exists(&id).await.unwrap());
        assert!(dest.refs.get_tag(&tag_name).unwrap().is_some());
    }

    // bole-9by
    #[tokio::test]
    async fn filtered_snapshot_hides_protected_path() {
        use crate::acl::{Accessor, PathAcl, PathRole, Permission};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        let blob1 = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
        let blob2 = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob1, kind: EntryKind::Blob });
        entries.insert("secrets/prod.key".into(), TreeEntry { id: blob2, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(), created_at: 1, message: "m".into(),
        }).await.unwrap();

        let empty = Accessor::new();
        let filtered = repo.get_snapshot_filtered(snap_id, &empty).await.unwrap().unwrap();
        assert!(filtered.visible_paths.contains_key("src/app.rs"));
        assert!(!filtered.visible_paths.contains_key("secrets/prod.key"));

        let privileged = Accessor::new()
            .with_path_role(PathRole { glob: "secrets/**".into(), permission: Permission::Read });
        let filtered2 = repo.get_snapshot_filtered(snap_id, &privileged).await.unwrap().unwrap();
        assert!(filtered2.visible_paths.contains_key("src/app.rs"));
        assert!(filtered2.visible_paths.contains_key("secrets/prod.key"));
    }

    #[test]
    fn list_refs_filtered_hides_protected_timeline() {
        use crate::acl::{Accessor, TimelineAcl, TimelineRole, Permission};
        use crate::object::ObjectId;

        let repo = Repository::memory();
        repo.acls.set_timeline_acl(TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();

        let id = ObjectId::new([1u8; 32]);
        repo.refs.create_tag(RefName::new("main").unwrap(), id, None, 1).unwrap();
        repo.refs.create_tag(RefName::new("leslie/private/exp").unwrap(), id, None, 2).unwrap();

        let empty = Accessor::new();
        let visible = repo.list_refs_filtered("", &empty).unwrap();
        let names: Vec<&str> = visible.iter().map(|n| n.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(!names.contains(&"leslie/private/exp"));

        let privileged = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "leslie/private/**".into(), permission: Permission::Read });
        let visible2 = repo.list_refs_filtered("", &privileged).unwrap();
        let names2: Vec<&str> = visible2.iter().map(|n| n.as_str()).collect();
        assert!(names2.contains(&"leslie/private/exp"));
    }

    // bole-l0i
    #[tokio::test]
    async fn compute_workspace_view_resolves_env() {
        use crate::acl::{Accessor, PathRole, Permission};
        use crate::object::{EnvOverlay, EnvValue, Snapshot, TreeEntry, EntryKind};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [42u8; 32];

        let blob_id = repo.objects.put_blob(Bytes::from("code")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/main.rs".into(), TreeEntry { id: blob_id, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "test".into(),
            created_at: 1, message: "m".into(),
        }).await.unwrap();

        let secret_id = repo.objects.put_secret(b"postgres://prod", &key).await.unwrap();
        let mut env_entries = BTreeMap::new();
        env_entries.insert("DB_URL".into(), EnvValue::Secret(secret_id));
        env_entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: env_entries }).await.unwrap();

        let accessor = Accessor::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });

        let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &accessor)
            .await.unwrap().unwrap();

        assert!(view.files.contains_key("src/main.rs"));
        assert_eq!(view.env.get("DB_URL").map(String::as_str), Some("postgres://prod"));
        assert_eq!(view.env.get("LOG_LEVEL").map(String::as_str), Some("info"));
    }

    // bole-l0i
    #[tokio::test]
    async fn compute_workspace_view_acl_filters_files() {
        use crate::acl::{Accessor, PathAcl};
        use crate::object::{EnvOverlay, Snapshot, TreeEntry, EntryKind};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [1u8; 32];

        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/app.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        entries.insert("src/config.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![], author: "t".into(),
            created_at: 1, message: "m".into(),
        }).await.unwrap();

        repo.acls.set_path_acl(PathAcl { glob: "src/config.rs".into() }).unwrap();

        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: BTreeMap::new() }).await.unwrap();

        let view = repo.compute_workspace_view(snap_id, overlay_id, &key, &Accessor::new())
            .await.unwrap().unwrap();

        assert!(view.files.contains_key("src/app.rs"));
        assert!(!view.files.contains_key("src/config.rs"));
        assert!(view.env.is_empty());
    }

    // bole-l0i
    #[tokio::test]
    async fn compute_workspace_view_returns_none_for_missing_snapshot() {
        use crate::acl::Accessor;
        use crate::object::{EnvOverlay, ObjectId};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let key = [1u8; 32];
        let missing = ObjectId::new([9u8; 32]);
        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries: BTreeMap::new() }).await.unwrap();
        let result = repo.compute_workspace_view(missing, overlay_id, &key, &Accessor::new())
            .await.unwrap();
        assert!(result.is_none());
    }

    // bole-u6p
    #[tokio::test]
    async fn find_common_ancestor_delegates_to_merge_lca() {
        use crate::object::Snapshot;
        use bytes::Bytes;

        let repo = Repository::memory();
        let root = repo.objects.put_blob(Bytes::from("root")).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot {
            root, parents: vec![], author: "t".into(), created_at: 0, message: "b".into(),
        }).await.unwrap();
        let root2 = repo.objects.put_blob(Bytes::from("a")).await.unwrap();
        let tip_a = repo.objects.put_snapshot(Snapshot {
            root: root2, parents: vec![base], author: "t".into(), created_at: 1, message: "a".into(),
        }).await.unwrap();
        let root3 = repo.objects.put_blob(Bytes::from("b")).await.unwrap();
        let tip_b = repo.objects.put_snapshot(Snapshot {
            root: root3, parents: vec![base], author: "t".into(), created_at: 2, message: "b2".into(),
        }).await.unwrap();
        let lca = repo.find_common_ancestor(tip_a, tip_b).await.unwrap();
        assert!(lca == Some(base));
    }

    #[tokio::test]
    async fn merge_timelines_requires_write_cap() {
        use crate::acl::{Accessor, TimelineRole, Permission};
        use crate::object::{Snapshot, TreeEntry, EntryKind};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use bytes::Bytes;

        let repo = Repository::memory();
        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("a.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into(),
        }).await.unwrap();

        let src = RefName::new("src").unwrap();
        let tgt = RefName::new("tgt").unwrap();
        repo.refs.create_timeline(src.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(tgt.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // no write cap → AccessDenied
        let err = repo.merge_timelines(&src, &tgt, &Accessor::new()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)), "got {err:?}");

        // with write cap → succeeds (clean merge, same snap)
        let writer = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "tgt".into(), permission: Permission::Write });
        let result = repo.merge_timelines(&src, &tgt, &writer).await.unwrap();
        assert!(result.is_clean());
    }

    #[tokio::test]
    async fn merge_timelines_three_way_diff() {
        use crate::acl::{Accessor, TimelineRole, Permission};
        use crate::object::{Snapshot, TreeEntry, EntryKind};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use bytes::Bytes;

        let repo = Repository::memory();

        // Base snapshot: file "shared.rs"
        let blob_base = repo.objects.put_blob(Bytes::from("base")).await.unwrap();
        let mut base_entries = BTreeMap::new();
        base_entries.insert("shared.rs".into(), TreeEntry { id: blob_base, kind: EntryKind::Blob });
        let base_tree = repo.objects.put_tree(base_entries).await.unwrap();
        let base_snap = repo.objects.put_snapshot(Snapshot {
            root: base_tree, parents: vec![], author: "t".into(), created_at: 0, message: "base".into(),
        }).await.unwrap();

        // Source snapshot: changes "shared.rs"
        let blob_src = repo.objects.put_blob(Bytes::from("src-change")).await.unwrap();
        let mut src_entries = BTreeMap::new();
        src_entries.insert("shared.rs".into(), TreeEntry { id: blob_src, kind: EntryKind::Blob });
        let src_tree = repo.objects.put_tree(src_entries).await.unwrap();
        let src_snap = repo.objects.put_snapshot(Snapshot {
            root: src_tree, parents: vec![base_snap], author: "t".into(), created_at: 1, message: "src".into(),
        }).await.unwrap();

        // Target snapshot: adds "other.rs", keeps "shared.rs" at base
        let blob_other = repo.objects.put_blob(Bytes::from("other")).await.unwrap();
        let mut tgt_entries = BTreeMap::new();
        tgt_entries.insert("shared.rs".into(), TreeEntry { id: blob_base, kind: EntryKind::Blob });
        tgt_entries.insert("other.rs".into(), TreeEntry { id: blob_other, kind: EntryKind::Blob });
        let tgt_tree = repo.objects.put_tree(tgt_entries).await.unwrap();
        let tgt_snap = repo.objects.put_snapshot(Snapshot {
            root: tgt_tree, parents: vec![base_snap], author: "t".into(), created_at: 2, message: "tgt".into(),
        }).await.unwrap();

        let src = RefName::new("src").unwrap();
        let tgt = RefName::new("tgt").unwrap();
        repo.refs.create_timeline(src.clone(), src_snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(tgt.clone(), tgt_snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let writer = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "tgt".into(), permission: Permission::Write });
        let result = repo.merge_timelines(&src, &tgt, &writer).await.unwrap();

        // clean merge: theirs changed "shared.rs", ours added "other.rs"
        assert!(result.is_clean(), "conflicts: {:?}", result.conflicts);
        assert_eq!(result.merged.get("shared.rs"), Some(&blob_src));
        assert_eq!(result.merged.get("other.rs"), Some(&blob_other));
    }

    // bole-u6p
    #[tokio::test]
    async fn merge_conflicting_timelines() {
        use crate::acl::{Accessor, PathRole, TimelineRole, Permission};
        use crate::object::{Snapshot, TreeEntry, EntryKind};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use bytes::Bytes;

        let repo = Repository::memory();

        // Ancestor snapshot: shared.rs at blob v1
        let blob_v1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
        let mut anc_entries = BTreeMap::new();
        anc_entries.insert("shared.rs".into(), TreeEntry { id: blob_v1, kind: EntryKind::Blob });
        let anc_tree = repo.objects.put_tree(anc_entries).await.unwrap();
        let anc_snap = repo.objects.put_snapshot(Snapshot {
            root: anc_tree, parents: vec![], author: "t".into(), created_at: 0, message: "anc".into(),
        }).await.unwrap();

        // Source timeline: shared.rs → blob v2
        let blob_v2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
        let mut src_entries = BTreeMap::new();
        src_entries.insert("shared.rs".into(), TreeEntry { id: blob_v2, kind: EntryKind::Blob });
        let src_tree = repo.objects.put_tree(src_entries).await.unwrap();
        let src_snap = repo.objects.put_snapshot(Snapshot {
            root: src_tree, parents: vec![anc_snap], author: "t".into(), created_at: 1, message: "src".into(),
        }).await.unwrap();

        // Target timeline: shared.rs → blob v3
        let blob_v3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
        let mut tgt_entries = BTreeMap::new();
        tgt_entries.insert("shared.rs".into(), TreeEntry { id: blob_v3, kind: EntryKind::Blob });
        let tgt_tree = repo.objects.put_tree(tgt_entries).await.unwrap();
        let tgt_snap = repo.objects.put_snapshot(Snapshot {
            root: tgt_tree, parents: vec![anc_snap], author: "t".into(), created_at: 2, message: "tgt".into(),
        }).await.unwrap();

        let src = RefName::new("conflict-src").unwrap();
        let tgt = RefName::new("conflict-tgt").unwrap();
        repo.refs.create_timeline(src.clone(), src_snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(tgt.clone(), tgt_snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let full_write_accessor = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "*".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });

        let result = repo.merge_timelines(&src, &tgt, &full_write_accessor).await.unwrap();

        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, "shared.rs");
        // merge_timelines calls three_way_diff(&ancestor, &target_tree, &source_tree)
        // so ours = target's blob, theirs = source's blob
        assert_eq!(result.conflicts[0].ours, Some(blob_v3));
        assert_eq!(result.conflicts[0].theirs, Some(blob_v2));
    }

    #[tokio::test]
    async fn advance_timeline_requires_write_cap_on_timeline() {
        use crate::acl::Accessor;
        use crate::object::{Snapshot, TreeEntry, EntryKind};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use bytes::Bytes;

        let repo = Repository::memory();
        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("a.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![snap], author: "t".into(), created_at: 1, message: "m2".into(),
        }).await.unwrap();

        let err = repo.advance_timeline(&name, snap2, &Accessor::new()).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)));
    }

    #[tokio::test]
    async fn advance_timeline_requires_write_cap_on_paths() {
        use crate::acl::{Accessor, PathRole, TimelineRole, Permission};
        use crate::object::{Snapshot, TreeEntry, EntryKind};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use bytes::Bytes;

        let repo = Repository::memory();
        let blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("secrets/key".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![snap], author: "t".into(), created_at: 1, message: "m2".into(),
        }).await.unwrap();

        // has timeline write but no path write → AccessDenied on path
        let partial = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write });
        let err = repo.advance_timeline(&name, snap2, &partial).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)));

        // with both → succeeds
        let full = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });
        repo.advance_timeline(&name, snap2, &full).await.unwrap();
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, snap2);
    }

    // bole-u6p
    #[tokio::test]
    async fn advance_timeline_write_role_succeeds() {
        use crate::acl::{Accessor, PathRole, TimelineRole, Permission};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();

        // Initial snapshot with empty tree
        let empty_tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap1 = repo.objects.put_snapshot(Snapshot {
            root: empty_tree, parents: vec![], author: "t".into(), created_at: 0, message: "s1".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), snap1, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // Second snapshot parenting the first
        let snap2 = repo.objects.put_snapshot(Snapshot {
            root: empty_tree, parents: vec![snap1], author: "t".into(), created_at: 1, message: "s2".into(),
        }).await.unwrap();

        // Full-write accessor: timeline write + path write
        let full = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "main".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });

        repo.advance_timeline(&name, snap2, &full).await.unwrap();
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, snap2);
    }

    // bole-3w9
    /// Builds a repo with three snapshots: base, a descendant `child`, and an
    /// unrelated `sibling` (also rooted at base, so it is NOT a descendant of
    /// child). Returns (repo, base, child, sibling) plus a full-write accessor.
    #[cfg(test)]
    async fn policy_fixture() -> (Repository, ObjectId, ObjectId, ObjectId, crate::acl::Accessor) {
        use crate::acl::{Accessor, PathRole, TimelineRole, Permission};
        use crate::object::Snapshot;
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "base".into(),
        }).await.unwrap();
        let child = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "child".into(),
        }).await.unwrap();
        // sibling parents base too -> shares the ancestor but does not descend from child
        let sibling = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 2, message: "sibling".into(),
        }).await.unwrap();
        let full = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "**".into(), permission: Permission::Write })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Write });
        (repo, base, child, sibling, full)
    }

    // bole-3w9
    #[tokio::test]
    async fn fast_forward_only_accepts_descendant() {
        use crate::refs::RefName;
        let (repo, base, child, _sibling, full) = policy_fixture().await;
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), base, TimelinePolicy::FastForwardOnly, 0, "persistent".into(), None).unwrap();
        // child descends from base -> allowed.
        repo.advance_timeline(&name, child, &full).await.unwrap();
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, child);
    }

    // bole-3w9
    #[tokio::test]
    async fn fast_forward_only_rejects_non_descendant() {
        use crate::error::Error;
        use crate::refs::RefName;
        let (repo, base, child, sibling, full) = policy_fixture().await;
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), child, TimelinePolicy::FastForwardOnly, 0, "persistent".into(), None).unwrap();
        // sibling does not descend from child -> rejected, head unchanged.
        let err = repo.advance_timeline(&name, sibling, &full).await.unwrap_err();
        assert!(matches!(err, Error::PolicyViolation(_)), "expected PolicyViolation, got {err:?}");
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, child);
        let _ = base;
    }

    // bole-3w9
    #[tokio::test]
    async fn append_rejects_non_descendant() {
        use crate::error::Error;
        use crate::refs::RefName;
        let (repo, _base, child, sibling, full) = policy_fixture().await;
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), child, TimelinePolicy::Append, 0, "persistent".into(), None).unwrap();
        let err = repo.advance_timeline(&name, sibling, &full).await.unwrap_err();
        assert!(matches!(err, Error::PolicyViolation(_)), "expected PolicyViolation, got {err:?}");
    }

    // bole-3w9
    #[tokio::test]
    async fn unrestricted_allows_non_descendant() {
        use crate::refs::RefName;
        let (repo, _base, child, sibling, full) = policy_fixture().await;
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), child, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        // Unrestricted: any snapshot is a valid new head.
        repo.advance_timeline(&name, sibling, &full).await.unwrap();
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, sibling);
    }

    #[test]
    fn prune_timeline_removes_expired_with_no_tags() {
        use crate::object::ObjectId;
        use crate::refs::{RefName, TimelinePolicy};

        let repo = Repository::memory();
        let head = ObjectId::new([1u8; 32]);
        let name = RefName::new("exp").unwrap();
        repo.refs.create_timeline(
            name.clone(), head, TimelinePolicy::Unrestricted, 0,
            "ephemeral".into(), Some(100),
        ).unwrap();

        // not yet expired
        assert!(!repo.prune_timeline(&name, 99).unwrap());
        assert!(repo.refs.get_timeline(&name).unwrap().is_some());

        // now = 100 → expired, no tags → pruned
        assert!(repo.prune_timeline(&name, 100).unwrap());
        assert!(repo.refs.get_timeline(&name).unwrap().is_none());
    }

    #[test]
    fn prune_timeline_does_not_remove_when_tag_on_head() {
        use crate::object::ObjectId;
        use crate::refs::{RefName, TimelinePolicy};

        let repo = Repository::memory();
        let head = ObjectId::new([2u8; 32]);
        let tl_name = RefName::new("exp2").unwrap();
        repo.refs.create_timeline(
            tl_name.clone(), head, TimelinePolicy::Unrestricted, 0,
            "ephemeral".into(), Some(100),
        ).unwrap();
        // pin the head with a tag
        repo.refs.create_tag(RefName::new("pinned-v1").unwrap(), head, None, 0).unwrap();

        // expired but pinned → not pruned
        assert!(!repo.prune_timeline(&tl_name, 200).unwrap());
        assert!(repo.refs.get_timeline(&tl_name).unwrap().is_some());
    }

    #[test]
    fn prune_timeline_ignores_non_expired() {
        use crate::object::ObjectId;
        use crate::refs::{RefName, TimelinePolicy};

        let repo = Repository::memory();
        let head = ObjectId::new([3u8; 32]);
        let name = RefName::new("persistent").unwrap();
        // no expires_at
        repo.refs.create_timeline(
            name.clone(), head, TimelinePolicy::Unrestricted, 0,
            "persistent".into(), None,
        ).unwrap();
        assert!(!repo.prune_timeline(&name, 99999).unwrap());
        assert!(repo.refs.get_timeline(&name).unwrap().is_some());
    }

    // bole-fo2
    #[tokio::test]
    async fn confined_agent_cannot_advance_declassifying_snapshot() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::Label;
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::{Accessor, PathAcl};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let repo = Repository::memory();
        // The repo protects secrets/**; everything else is public (bottom).
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        // Snapshot writes a PUBLIC path (declassifying target for a confined,
        // protected-cleared agent).
        let blob = repo.objects.put_blob(Bytes::from("leak")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("public/notes.md".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let base = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "base".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let next = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "next".into(),
        }).await.unwrap();

        // A confined agent cleared to WRITE at `protected`, scoped to everything.
        let lattice = Arc::new(crate::acl::lattice::LabelLattice::two_point());
        let clr = ClearanceSet {
            clearances: vec![
                Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::WRITE,
                    scope: Some(ClearanceScope::Path("**".into())),
                },
                Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::WRITE,
                    scope: Some(ClearanceScope::Timeline("**".into())),
                },
            ],
            confined: true,
        };
        let agent = Accessor::from_parts(lattice, Arc::new(LabelRuleSet::default()), clr);

        // Writing the public path strictly below `protected` is a declassifying
        // write -> denied before the head moves.
        let err = repo.advance_timeline(&name, next, &agent).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)), "got {err:?}");
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, base);
    }

    // bole-fo2
    // bole-6i7
    #[tokio::test]
    async fn merge_into_release_requires_two_signed_approvals() {
        use crate::acl::attestation::{ApproverRegistry, AttestationSigner};
        use crate::acl::policy_object::HookSpec;
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use crate::MergeCheck;
        use std::collections::BTreeMap;

        let mut repo = Repository::memory();
        repo.register_hook(HookSpec {
            kind: "signed-approval".into(),
            pattern: "release/**".into(),
            params: BTreeMap::from([("needed".to_string(), 2u64)]),
        });

        // A clean source (no protected paths) merging into release/1.0. Both
        // timelines start at `snap`, so check_merge's result_head is `snap`.
        let blob = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("src/lib.rs".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
        let snap = repo.objects.put_snapshot(Snapshot {
            root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into(),
        }).await.unwrap();
        let source = RefName::new("feature/x").unwrap();
        let dest = RefName::new("release/1.0").unwrap();
        repo.refs.create_timeline(source.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(dest.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        let writer = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "release/**".into(), permission: Permission::Write });

        // Register the approver set; no attestations yet -> RequiresApproval.
        let alice = AttestationSigner::from_seed("alice", [1u8; 32]);
        let bob = AttestationSigner::from_seed("bob", [2u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        reg.add(bob.approver());
        repo.set_approvers(&reg).await.unwrap();

        let r1 = repo.check_merge(&source, &dest, &writer).await.unwrap();
        assert!(matches!(r1, MergeCheck::RequiresApproval(_)), "got {r1:?}");

        // Two distinct signed approvals of the exact result head -> Allowed.
        repo.add_attestation(&alice.attest("release/1.0", snap)).await.unwrap();
        repo.add_attestation(&bob.attest("release/1.0", snap)).await.unwrap();
        let r2 = repo.check_merge(&source, &dest, &writer).await.unwrap();
        assert_eq!(r2, MergeCheck::Allowed);

        // A forged approval by an UNREGISTERED key does not count.
        let mallory = AttestationSigner::from_seed("mallory", [9u8; 32]);
        let mut repo2 = Repository::memory();
        repo2.register_hook(HookSpec {
            kind: "signed-approval".into(),
            pattern: "release/**".into(),
            params: BTreeMap::from([("needed".to_string(), 1u64)]),
        });
        repo2.refs.create_timeline(source.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo2.refs.create_timeline(dest.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo2.objects.put_blob(Bytes::from("x")).await.unwrap();
        repo2.objects.put_tree(BTreeMap::from([("src/lib.rs".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob })])).await.unwrap();
        repo2.objects.put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() }).await.unwrap();
        repo2.set_approvers(&reg).await.unwrap(); // alice/bob registered, NOT mallory
        repo2.add_attestation(&mallory.attest("release/1.0", snap)).await.unwrap();
        let r3 = repo2.check_merge(&source, &dest, &writer).await.unwrap();
        assert!(matches!(r3, MergeCheck::RequiresApproval(_)), "forged approver must not count: {r3:?}");
    }

    // bole-6i7
    #[tokio::test]
    async fn approver_and_attestation_persistence_roundtrip() {
        use crate::acl::attestation::{ApproverRegistry, AttestationSigner};

        let repo = Repository::memory();
        // Empty by default.
        assert!(repo.approvers().await.unwrap().approvers.is_empty());
        assert!(repo.attestations().await.unwrap().is_empty());

        let alice = AttestationSigner::from_seed("alice", [1u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        repo.set_approvers(&reg).await.unwrap();
        let loaded = repo.approvers().await.unwrap();
        assert_eq!(loaded.approvers.len(), 1);
        assert!(loaded.find("alice").is_some());

        let head = crate::object::ObjectId::new([5u8; 32]);
        repo.add_attestation(&alice.attest("release/1.0", head)).await.unwrap();
        let atts = repo.attestations().await.unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].target, "release/1.0");
        assert_eq!(atts[0].head, head);

        // set_approvers overwrites the pin (idempotent single source of truth).
        reg.add(AttestationSigner::from_seed("bob", [2u8; 32]).approver());
        repo.set_approvers(&reg).await.unwrap();
        assert_eq!(repo.approvers().await.unwrap().approvers.len(), 2);
    }

    // bole-rdh
    #[tokio::test]
    async fn signed_approval_gates_direct_advance() {
        use crate::acl::attestation::{ApproverRegistry, AttestationSigner};
        use crate::acl::policy_object::HookSpec;
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let mut repo = Repository::memory();
        repo.register_hook(HookSpec {
            kind: "signed-approval".into(),
            pattern: "release/**".into(),
            params: BTreeMap::from([("needed".to_string(), 1u64)]),
        });
        // Empty-tree snapshots so advance_timeline's per-path check is a no-op.
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
            .await
            .unwrap();
        let child = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into() })
            .await
            .unwrap();
        let dest = RefName::new("release/1.0").unwrap();
        repo.refs
            .create_timeline(dest.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        let writer = Accessor::new().with_timeline_role(TimelineRole {
            pattern: "release/**".into(),
            permission: Permission::Write,
        });

        // Register an approver.
        let alice = AttestationSigner::from_seed("alice", [1u8; 32]);
        let mut reg = ApproverRegistry::new();
        reg.add(alice.approver());
        repo.set_approvers(&reg).await.unwrap();

        // No attestation for `child`: advancing the gated timeline is refused
        // (Advance event, signed-approval hook — bole-rdh + bole-6i7).
        let err = repo.advance_timeline(&dest, child, &writer).await.unwrap_err();
        assert!(
            matches!(err, crate::error::Error::PolicyViolation(_)),
            "expected the signed-approval gate to fire on a direct advance, got {err:?}"
        );
        assert_eq!(repo.refs.get_timeline(&dest).unwrap().unwrap().head, base, "head must not move");

        // An attestation of the WRONG head does not unlock this advance.
        repo.add_attestation(&alice.attest("release/1.0", base)).await.unwrap();
        assert!(repo.advance_timeline(&dest, child, &writer).await.is_err());

        // A signed approval of the EXACT head unlocks the advance.
        repo.add_attestation(&alice.attest("release/1.0", child)).await.unwrap();
        repo.advance_timeline(&dest, child, &writer).await.unwrap();
        assert_eq!(repo.refs.get_timeline(&dest).unwrap().unwrap().head, child);
    }

    // bole-tgr8
    /// `ref_served` is the point-lookup twin of `list_refs_served`: same label
    /// gate, same structural scoped-collab exclusion, no O(all-refs) scan.
    #[tokio::test]
    async fn ref_served_matches_list_semantics() {
        use crate::acl::{Accessor, Permission, TimelineAcl, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();
        let public = RefName::new("main").unwrap();
        let hidden = RefName::new("private/x").unwrap();
        repo.refs.create_timeline(public.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(hidden.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.acls.set_timeline_acl(TimelineAcl { pattern: "private/**".into() }).unwrap();
        let scoped = RefName::new(format!("{}profile/x", crate::repo::collab::COLLAB_SCOPED_PREFIX)).unwrap();
        let mut tx = repo.refs.transaction();
        tx.set(scoped.clone(), crate::refs::Ref::Tag(crate::refs::Tag { target: snap, created_at: 0, message: None }));
        tx.commit().unwrap();

        let anon = Accessor::new();
        let cleared = Accessor::new().with_timeline_role(TimelineRole {
            pattern: "private/**".into(),
            permission: Permission::Read,
        });

        // Point lookups agree with list membership for every case.
        for accessor in [&anon, &cleared, &Accessor::privileged()] {
            let listed = repo.list_refs_served("", accessor).unwrap();
            for name in [&public, &hidden, &scoped] {
                assert_eq!(
                    repo.ref_served(name, accessor).unwrap(),
                    listed.contains(name),
                    "ref_served({}) diverges from list membership",
                    name.as_str()
                );
            }
        }
        // Spot-check the interesting cases directly.
        assert!(repo.ref_served(&public, &anon).unwrap());
        assert!(!repo.ref_served(&hidden, &anon).unwrap());
        assert!(repo.ref_served(&hidden, &cleared).unwrap());
        assert!(!repo.ref_served(&scoped, &Accessor::privileged()).unwrap(), "scoped is structural");
    }

    // bole-au0t
    #[tokio::test]
    async fn policy_root_pin_round_trip() {
        use crate::acl::policy_object::{HookSpec, PolicyRoot};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        assert!(repo.policy_root().await.unwrap().is_none(), "fresh repo has no pinned root");

        let root = PolicyRoot {
            lattice: ObjectId::from_content(b"lattice"),
            rules: ObjectId::from_content(b"rules"),
            parent: None,
            hooks: vec![HookSpec {
                kind: "timeline-policy".into(),
                pattern: "**".into(),
                params: BTreeMap::new(),
            }],
        };
        let id = repo.set_policy_root(&root).await.unwrap();
        let (got_id, got) = repo.policy_root().await.unwrap().expect("pinned root");
        assert_eq!(got_id, id);
        assert_eq!(got, root);

        // Re-pinning replaces the tip.
        let root2 = PolicyRoot { parent: Some(id), hooks: vec![], ..root.clone() };
        let id2 = repo.set_policy_root(&root2).await.unwrap();
        let (got_id2, got2) = repo.policy_root().await.unwrap().expect("re-pinned root");
        assert_eq!(got_id2, id2);
        assert_eq!(got2, root2);
    }

    // bole-au0t
    #[tokio::test]
    async fn unknown_hook_kind_in_pinned_root_fails_closed_on_advance() {
        use crate::acl::policy_object::{HookSpec, PolicyRoot};
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
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

        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
            .await
            .unwrap();
        let child = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into() })
            .await
            .unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs
            .create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        let writer = Accessor::new().with_timeline_role(TimelineRole {
            pattern: "**".into(),
            permission: Permission::Write,
        });

        // A replica that does not recognize the pinned root's hook kind must
        // refuse the advance (fail-closed, WS1-O5), not skip the hook.
        let err = repo.advance_timeline(&name, child, &writer).await.unwrap_err();
        assert!(
            matches!(&err, crate::error::Error::PolicyViolation(r) if r.contains("unknown policy hook kind")),
            "expected fail-closed unknown-kind rejection, got {err:?}"
        );
        assert_eq!(repo.refs.get_timeline(&name).unwrap().unwrap().head, base, "head must not move");
    }

    // bole-au0t
    #[tokio::test]
    async fn unknown_hook_kind_in_pinned_root_fails_closed_on_check_merge() {
        use crate::acl::policy_object::{HookSpec, PolicyRoot};
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();
        let source = RefName::new("feature/x").unwrap();
        let dest = RefName::new("main").unwrap();
        repo.refs.create_timeline(source.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        repo.refs.create_timeline(dest.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        repo.set_policy_root(&PolicyRoot {
            lattice: ObjectId::from_content(b"lattice"),
            rules: ObjectId::from_content(b"rules"),
            parent: None,
            hooks: vec![HookSpec { kind: "quantum-approval".into(), pattern: "**".into(), params: BTreeMap::new() }],
        })
        .await
        .unwrap();

        let writer = Accessor::new().with_timeline_role(TimelineRole {
            pattern: "**".into(),
            permission: Permission::Write,
        });
        let err = repo.check_merge(&source, &dest, &writer).await.unwrap_err();
        assert!(
            matches!(&err, crate::error::Error::PolicyViolation(r) if r.contains("unknown policy hook kind")),
            "check_merge must fail closed on an unknown pinned hook kind, got {err:?}"
        );
    }

    // bole-au0t
    #[tokio::test]
    async fn malformed_policy_root_ref_fails_closed() {
        use crate::acl::policy_object::POLICY_ROOT_REF;
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        // (a) A timeline squatting the policy-root ref name is an error, not an
        // absent policy.
        let repo = Repository::memory();
        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();
        let root_ref = RefName::new(POLICY_ROOT_REF).unwrap();
        repo.refs
            .create_timeline(root_ref.clone(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        assert!(repo.policy_root().await.is_err(), "timeline at refs/policy/root must fail closed");
        assert!(repo.policy_registry().await.is_err(), "registry must refuse a malformed pin");

        // (b) A tag pointing at a non-PolicyRoot object is an error too.
        let repo2 = Repository::memory();
        let blob = repo2.objects.put_blob(Bytes::from("junk")).await.unwrap();
        let mut tx = repo2.refs.transaction();
        tx.set(root_ref.clone(), crate::refs::Ref::Tag(crate::refs::Tag { target: blob, created_at: 0, message: None }));
        tx.commit().unwrap();
        assert!(repo2.policy_root().await.is_err(), "non-root target must fail closed");

        // (c) A dangling tag target is an error too.
        let repo3 = Repository::memory();
        let mut tx = repo3.refs.transaction();
        tx.set(root_ref, crate::refs::Ref::Tag(crate::refs::Tag { target: ObjectId::from_content(b"missing"), created_at: 0, message: None }));
        tx.commit().unwrap();
        assert!(repo3.policy_root().await.is_err(), "dangling target must fail closed");
    }

    // bole-au0t
    #[tokio::test]
    async fn pinned_policy_root_hooks_gate_advance() {
        use crate::acl::policy_object::{HookSpec, PolicyRoot};
        use crate::acl::{Accessor, Permission, TimelineRole};
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;

        // No register_hook: the ONLY binding channel is the pinned policy root,
        // i.e. state a replica receives via sync. The hook must still gate.
        let repo = Repository::memory();
        repo.set_policy_root(&PolicyRoot {
            lattice: ObjectId::from_content(b"lattice"),
            rules: ObjectId::from_content(b"rules"),
            parent: None,
            hooks: vec![HookSpec {
                kind: "signed-approval".into(),
                pattern: "release/**".into(),
                params: BTreeMap::from([("needed".to_string(), 1u64)]),
            }],
        })
        .await
        .unwrap();

        let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
        let base = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
            .await
            .unwrap();
        let child = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "c".into() })
            .await
            .unwrap();
        let dest = RefName::new("release/1.0").unwrap();
        repo.refs
            .create_timeline(dest.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        let writer = Accessor::new().with_timeline_role(TimelineRole {
            pattern: "release/**".into(),
            permission: Permission::Write,
        });

        let err = repo.advance_timeline(&dest, child, &writer).await.unwrap_err();
        assert!(
            matches!(err, crate::error::Error::PolicyViolation(_)),
            "pinned-root signed-approval hook must gate the advance, got {err:?}"
        );
        assert_eq!(repo.refs.get_timeline(&dest).unwrap().unwrap().head, base, "head must not move");
    }

    // bole-wy4
    #[tokio::test]
    async fn deep_tree_chain_is_rejected_not_overflow() {
        use crate::acl::Accessor;
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use std::collections::BTreeMap;

        let repo = Repository::memory();
        // Build a chain of single-entry nested trees deeper than the cap,
        // bottom-up (a loop, so building itself never recurses).
        let leaf = repo.objects.put_blob(Bytes::from("x")).await.unwrap();
        let mut child = {
            let mut e = BTreeMap::new();
            e.insert("f".to_string(), TreeEntry { id: leaf, kind: EntryKind::Blob });
            repo.objects.put_tree(e).await.unwrap()
        };
        for _ in 0..(super::MAX_TREE_DEPTH + 2) {
            let mut e = BTreeMap::new();
            e.insert("d".to_string(), TreeEntry { id: child, kind: EntryKind::Tree });
            child = repo.objects.put_tree(e).await.unwrap();
        }
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: child, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();

        // A filtered read must return a handled Error at the depth cap, not
        // recurse to a stack overflow (SIGABRT).
        let err = repo.get_snapshot_filtered(snap, &Accessor::privileged()).await.unwrap_err();
        assert!(
            matches!(err, crate::error::Error::Storage(_)) && format!("{err}").contains("depth"),
            "expected a depth-limit error, got {err:?}"
        );
    }

    // bole-qj4
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_advance_timeline_exactly_one_winner() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::Accessor;
        use crate::object::Snapshot;
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let writer = || {
            let clr = ClearanceSet {
                clearances: vec![Clearance {
                    ceiling: Label::protected(),
                    cap: Capability::WRITE,
                    scope: None,
                }],
                confined: false,
            };
            Accessor::from_parts(Arc::new(LabelLattice::two_point()), Arc::new(LabelRuleSet::default()), clr)
        };

        for _ in 0..100 {
            let repo = Arc::new(Repository::memory());
            let tree = repo.objects.put_tree(BTreeMap::new()).await.unwrap();
            let base = repo
                .objects
                .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
                .await
                .unwrap();
            let a = repo
                .objects
                .put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 1, message: "a".into() })
                .await
                .unwrap();
            let b = repo
                .objects
                .put_snapshot(Snapshot { root: tree, parents: vec![base], author: "t".into(), created_at: 2, message: "b2".into() })
                .await
                .unwrap();
            // FastForwardOnly + sibling children (a, b both descend only base,
            // neither descends the other). Under concurrency the loser either
            // sees a CAS conflict (read base, head already moved) or an ff
            // rejection (read the winner's head, its target isn't a descendant) —
            // both fail, so with the CAS fix exactly one advance ever succeeds,
            // deterministically. Without the CAS an advance evaluated against a
            // stale base head slips through unconditionally (oks == 2), a
            // fast-forward-policy bypass.
            let main = RefName::new("main").unwrap();
            repo.refs
                .create_timeline(main.clone(), base, TimelinePolicy::FastForwardOnly, 0, "persistent".into(), None)
                .unwrap();

            let (r1, r2) = {
                let (rp1, rp2) = (repo.clone(), repo.clone());
                let (m1, m2) = (main.clone(), main.clone());
                let (w1, w2) = (writer(), writer());
                let t1 = tokio::spawn(async move { rp1.advance_timeline(&m1, a, &w1).await });
                let t2 = tokio::spawn(async move { rp2.advance_timeline(&m2, b, &w2).await });
                (t1.await.unwrap(), t2.await.unwrap())
            };

            let oks = [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count();
            assert_eq!(oks, 1, "exactly one winner; got r1={r1:?} r2={r2:?}");
            let head = repo.refs.get_timeline(&main).unwrap().unwrap().head;
            assert!(head == a || head == b);
        }
    }

    // bole-48r
    #[tokio::test]
    async fn advance_denies_deleting_unwritable_protected_path() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::{Accessor, PathAcl};
        use crate::object::{EntryKind, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let repo = Repository::memory();
        repo.acls.set_path_acl(PathAcl { glob: "secrets/**".into() }).unwrap();

        // Old head has a protected secret plus a public doc.
        let sec = repo.objects.put_blob(Bytes::from("k")).await.unwrap();
        let doc = repo.objects.put_blob(Bytes::from("d")).await.unwrap();
        let mut old_entries = BTreeMap::new();
        old_entries.insert("secrets/prod.key".into(), TreeEntry { id: sec, kind: EntryKind::Blob });
        old_entries.insert("docs/a".into(), TreeEntry { id: doc, kind: EntryKind::Blob });
        let old_tree = repo.objects.put_tree(old_entries).await.unwrap();
        let base = repo
            .objects
            .put_snapshot(Snapshot { root: old_tree, parents: vec![], author: "t".into(), created_at: 0, message: "b".into() })
            .await
            .unwrap();

        // New snapshot DROPS secrets/prod.key, keeps docs/a.
        let mut new_entries = BTreeMap::new();
        new_entries.insert("docs/a".into(), TreeEntry { id: doc, kind: EntryKind::Blob });
        let new_tree = repo.objects.put_tree(new_entries).await.unwrap();
        let child = repo
            .objects
            .put_snapshot(Snapshot { root: new_tree, parents: vec![base], author: "t".into(), created_at: 1, message: "drop".into() })
            .await
            .unwrap();

        let main = RefName::new("main").unwrap();
        repo.refs
            .create_timeline(main.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        // Actor: write on public paths + write on timelines, but NOT on the
        // protected secrets/** label (public ceiling doesn't dominate protected).
        let clr = ClearanceSet {
            clearances: vec![
                Clearance { ceiling: Label::public(), cap: Capability::WRITE, scope: Some(ClearanceScope::Path("**".into())) },
                Clearance { ceiling: Label::protected(), cap: Capability::WRITE, scope: Some(ClearanceScope::Timeline("**".into())) },
            ],
            confined: false,
        };
        let actor = Accessor::from_parts(Arc::new(LabelLattice::two_point()), Arc::new(LabelRuleSet::default()), clr);

        // Dropping the protected path must be refused (it's a write to secrets/**
        // the actor cannot perform). Before bole-48r only the new tree was walked,
        // so the deletion slipped through.
        let err = repo.advance_timeline(&main, child, &actor).await.unwrap_err();
        assert!(
            matches!(err, crate::error::Error::AccessDenied(_)),
            "deleting a protected path must be denied, got {err:?}"
        );
    }

    // bole-9mz
    #[tokio::test]
    async fn resolve_overlay_gates_secrets_by_clearance() {
        use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
        use crate::acl::lattice::{Label, LabelLattice};
        use crate::acl::rules::LabelRuleSet;
        use crate::acl::{Accessor, SecretAcl};
        use crate::crypto::key_provider::{LocalKeyProvider, ProviderChain};
        use crate::object::{EnvOverlay, EnvValue, SecretAad};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let repo = Repository::memory();
        repo.acls.set_secret_acl(SecretAcl { name: "DB_URL".into() }).unwrap();

        let provider = LocalKeyProvider::new([7u8; 32], "env");
        let secret_id = repo
            .objects
            .put_secret_enveloped(b"postgres://secret", &provider, SecretAad::v2(Some(Label::protected())))
            .await
            .unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("LOG_LEVEL".into(), EnvValue::Plain("info".into()));
        entries.insert("DB_URL".into(), EnvValue::Secret(secret_id));
        let overlay_id = repo.objects.put_overlay(EnvOverlay { entries }).await.unwrap();

        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([7u8; 32], "env")));

        // Uncleared → fail closed, and the value never leaks into the error.
        let none = Accessor::new();
        let err = repo.resolve_overlay(&overlay_id, &chain, &none, false).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::AccessDenied(_)), "got {err:?}");
        assert!(!format!("{err}").contains("postgres"));

        // skip_unauthorized → omit the secret var, keep the plain var.
        let skipped = repo.resolve_overlay(&overlay_id, &chain, &none, true).await.unwrap();
        assert_eq!(skipped.get("LOG_LEVEL").map(String::as_str), Some("info"));
        assert!(!skipped.contains_key("DB_URL"));

        // Cleared (Read up to protected, secret-scoped) → full resolution.
        let lat = Arc::new(LabelLattice::two_point());
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: Label::protected(),
                cap: Capability::READ,
                scope: Some(ClearanceScope::Secret("**".into())),
            }],
            confined: false,
        };
        let cleared = Accessor::from_parts(lat, Arc::new(LabelRuleSet::default()), clr);
        let env = repo.resolve_overlay(&overlay_id, &chain, &cleared, false).await.unwrap();
        assert_eq!(env.get("DB_URL").map(String::as_str), Some("postgres://secret"));
        assert_eq!(env.get("LOG_LEVEL").map(String::as_str), Some("info"));
    }

    // bole-9mz
    #[tokio::test]
    async fn rekey_rotates_master_key_preserving_plaintext() {
        use crate::crypto::key_provider::{LocalKeyProvider, ProviderChain};
        use crate::object::SecretAad;

        let repo = Repository::memory();
        let mk_a = LocalKeyProvider::new([1u8; 32], "env");
        let id = repo.objects.put_secret_enveloped(b"val", &mk_a, SecretAad::v2(None)).await.unwrap();

        let old = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([1u8; 32], "env")));
        let mk_b = LocalKeyProvider::new([2u8; 32], "env");
        let mapping = repo.rekey(&[id], &old, &mk_b).await.unwrap();
        assert_eq!(mapping.len(), 1);
        let (old_id, new_id) = mapping[0];
        assert_eq!(old_id, id);
        assert_ne!(new_id, id);

        let new_chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([2u8; 32], "env")));
        assert_eq!(
            repo.objects.get_secret_resolved(&new_id, &new_chain).await.unwrap().unwrap(),
            b"val"
        );
        // The old master key can no longer open the rekeyed object.
        assert!(repo.objects.get_secret_resolved(&new_id, &old).await.is_err());
    }

    // bole-9mz
    #[tokio::test]
    async fn rekey_upgrades_v1_to_v2() {
        use crate::crypto::key_provider::{LocalKeyProvider, ProviderChain};
        use crate::object::Object;

        let repo = Repository::memory();
        let legacy = [4u8; 32];
        let id = repo.objects.put_secret(b"legacy", &legacy).await.unwrap();

        let mut old = ProviderChain::new();
        old.push_legacy_key(legacy);
        let mk = LocalKeyProvider::new([5u8; 32], "env");
        let mapping = repo.rekey(&[id], &old, &mk).await.unwrap();
        let (_, new_id) = mapping[0];

        match repo.objects.get(&new_id).await.unwrap().unwrap() {
            Object::SecretV2(_) => {}
            other => panic!("expected v2, got {other:?}"),
        }
        let chain = ProviderChain::with_provider(Box::new(LocalKeyProvider::new([5u8; 32], "env")));
        assert_eq!(
            repo.objects.get_secret_resolved(&new_id, &chain).await.unwrap().unwrap(),
            b"legacy"
        );
    }

    // bole-81z
    #[tokio::test]
    async fn gc_keeps_reachable_and_collects_garbage() {
        use crate::object::{EntryKind, EnvOverlay, EnvValue, Snapshot, TreeEntry};
        use crate::refs::{RefName, TimelinePolicy};
        use bytes::Bytes;
        use std::collections::BTreeMap;

        // Disk repo → PackedDiskBackend, so GC exercises the pack-rewrite path.
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::disk(dir.path()).await.unwrap();

        // Reachable graph: blob → tree → snapshot → timeline head.
        let blob = repo.objects.put_blob(Bytes::from("live")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("f".into(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(entries).await.unwrap();
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
        let name = RefName::new("main").unwrap();
        repo.refs
            .create_timeline(name, snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();

        // A secret reachable ONLY through an overlay (extra root).
        let secret = repo.objects.put_secret(b"s", &[3u8; 32]).await.unwrap();
        let mut oe = BTreeMap::new();
        oe.insert("DB".into(), EnvValue::Secret(secret));
        let overlay = repo.objects.put_overlay(EnvOverlay { entries: oe }).await.unwrap();

        // Pure garbage: an unreferenced blob.
        let orphan = repo.objects.put_blob(Bytes::from("garbage")).await.unwrap();

        // GC with the overlay as an extra root, grace disabled (now huge).
        let removed = repo.gc(&[overlay], 0, u64::MAX).await.unwrap();
        assert!(removed >= 1, "should collect the orphan");

        // Reachable objects survive.
        assert!(repo.objects.get(&blob).await.unwrap().is_some());
        assert!(repo.objects.get(&tree).await.unwrap().is_some());
        assert!(repo.objects.get(&snap).await.unwrap().is_some());
        // The overlay→secret edge kept the secret alive.
        assert!(repo.objects.get(&overlay).await.unwrap().is_some());
        assert!(repo.objects.get(&secret).await.unwrap().is_some());
        // Garbage is gone.
        assert!(repo.objects.get(&orphan).await.unwrap().is_none());
    }

    // bole-81z
    #[tokio::test]
    async fn gc_grace_window_protects_recent_objects() {
        use bytes::Bytes;
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::disk(dir.path()).await.unwrap();
        let orphan = repo.objects.put_blob(Bytes::from("fresh")).await.unwrap();
        // now ~= object mtime, large grace → the just-written orphan is protected.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let removed = repo.gc(&[], 3600, now).await.unwrap();
        assert_eq!(removed, 0);
        assert!(repo.objects.get(&orphan).await.unwrap().is_some());
    }
}
