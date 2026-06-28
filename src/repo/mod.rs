// bole-1vi
pub mod materialize;
// bole-l0i
pub mod workspace;
// bole-9lj
pub mod merge;
// bole-6bd
pub mod git_projection;

// bole-1vi
use std::collections::BTreeMap;
use std::path::Path;
use crate::acl::disk::DiskAclBackend;
use crate::acl::memory::MemoryAclBackend;
use crate::acl::{Accessor, AclStore, PathAcl, PathRole, Permission};
use crate::error::{Error, Result};
use crate::object::{EntryKind, Object, ObjectId};
// bole-u6p
use merge::{MergeResult, three_way_diff, find_common_ancestor as lca};
use crate::refs::Ref;
// bole-l0i
use crate::object::EnvValue;
use workspace::WorkspaceView;
use crate::refs::{DiskRefBackend, MemoryRefBackend, RefName, RefStore};
use crate::store::{disk::DiskBackend, memory::MemoryBackend, ObjectStore};

// bole-9by
#[derive(Debug, Clone)]
pub struct FilteredSnapshot {
    pub id: ObjectId,
    pub author: String,
    pub created_at: u64,
    pub message: String,
    pub parents: Vec<ObjectId>,
    pub visible_paths: BTreeMap<String, ObjectId>,
}

// bole-9by
#[derive(Debug, Clone, PartialEq)]
pub enum MergeCheck {
    Allowed,
    RequiresApproval(Vec<PathAcl>),
    Rejected(Vec<PathAcl>),
}

// bole-1vi
pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
    // bole-9by
    pub acls: AclStore,
}

// bole-1vi
impl Repository {
    pub fn memory() -> Self {
        Self {
            objects: ObjectStore::new(MemoryBackend::new()),
            refs: RefStore::new(MemoryRefBackend::new()),
            // bole-9by
            acls: AclStore::new(MemoryAclBackend::new()),
        }
    }

    pub async fn disk(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            objects: ObjectStore::new(DiskBackend::open(root).await?),
            refs: RefStore::new(DiskRefBackend::open(root)?),
            // bole-9by
            acls: AclStore::new(DiskAclBackend::open(root)?),
        })
    }

    pub async fn copy_to(&self, dest: &Repository) -> Result<()> {
        copy_objects(&self.objects, &dest.objects).await?;
        copy_refs(&self.refs, &dest.refs)?;
        Ok(())
    }

    // bole-9by
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
        walk_tree_filtered(&self.objects, &self.acls, snap.root, "", accessor, &mut visible_paths).await?;
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
    pub fn list_refs_filtered(&self, prefix: &str, accessor: &Accessor) -> Result<Vec<RefName>> {
        let all = self.refs.list(prefix)?;
        let mut out = Vec::new();
        for name in all {
            if self.acls.timeline_is_protected(name.as_str())? {
                if accessor.can_read_timeline(name.as_str()) {
                    out.push(name);
                }
            } else {
                out.push(name);
            }
        }
        Ok(out)
    }

    // bole-9by
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
        walk_tree_filtered(&self.objects, &self.acls, source_tree, "", &Accessor::privileged(), &mut visible).await?;
        // Find all paths in source that are protected but dest doesn't enforce them
        let mut leaking: Vec<PathAcl> = Vec::new();
        let path_acls = self.acls.list_path_acls()?;
        for acl in &path_acls {
            let any_match = visible.keys().any(|p| crate::acl::glob::glob_matches(&acl.glob, p));
            if any_match && !self.acls.timeline_is_protected(dest.as_str())?
                && !leaking.iter().any(|l| l.glob == acl.glob)
            {
                leaking.push(acl.clone());
            }
        }
        if leaking.is_empty() {
            Ok(MergeCheck::Allowed)
        } else if accessor.can_write_timeline(dest.as_str()) {
            Ok(MergeCheck::RequiresApproval(leaking))
        } else {
            Ok(MergeCheck::Rejected(leaking))
        }
    }

    // bole-u6p
    pub async fn find_common_ancestor(&self, a: ObjectId, b: ObjectId) -> Result<Option<ObjectId>> {
        lca(&self.objects, a, b).await
    }

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

    pub async fn advance_timeline(
        &self,
        name: &RefName,
        snapshot_id: ObjectId,
        accessor: &Accessor,
    ) -> Result<()> {
        if !accessor.can_write_timeline(name.as_str()) {
            return Err(Error::AccessDenied(format!(
                "write denied on timeline: {}",
                name.as_str()
            )));
        }
        let snap = match self.objects.get(&snapshot_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => return Err(Error::Storage(format!("snapshot not found: {}", snapshot_id))),
        };
        let mut paths = BTreeMap::new();
        walk_tree_filtered(
            &self.objects,
            &self.acls,
            snap.root,
            "",
            &Accessor::privileged(),
            &mut paths,
        )
        .await?;
        for path in paths.keys() {
            if !accessor.can_write_path(path) {
                return Err(Error::AccessDenied(format!(
                    "write denied on path: {}",
                    path
                )));
            }
        }
        self.refs.advance_head(name, snapshot_id)?;
        Ok(())
    }

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
        walk_tree_filtered(
            &self.objects,
            &self.acls,
            tree_id,
            "",
            &Accessor::privileged(),
            &mut map,
        )
        .await?;
        Ok(map)
    }

    // bole-l0i
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
}

// bole-9by
async fn walk_tree_filtered(
    objects: &ObjectStore,
    acls: &AclStore,
    tree_id: ObjectId,
    prefix: &str,
    accessor: &Accessor,
    out: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    let tree = match objects.get(&tree_id).await? {
        Some(Object::Tree(t)) => t,
        _ => return Ok(()),
    };
    for (name, entry) in &tree.entries {
        let full_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        match entry.kind {
            EntryKind::Blob => {
                if acls.path_is_protected(&full_path)? {
                    if accessor.can_read_path(&full_path) {
                        out.insert(full_path, entry.id);
                    }
                } else {
                    out.insert(full_path, entry.id);
                }
            }
            EntryKind::Tree => {
                Box::pin(walk_tree_filtered(objects, acls, entry.id, &full_path, accessor, out)).await?;
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
pub async fn copy_objects(from: &ObjectStore, to: &ObjectStore) -> Result<()> {
    for id in from.list().await? {
        if let Some(obj) = from.get(&id).await? {
            to.put(&obj).await?;
        }
    }
    Ok(())
}

// bole-1vi
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
        use crate::object::{ObjectId, Snapshot, TreeEntry, EntryKind};
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
        use crate::object::{ObjectId, Snapshot, TreeEntry, EntryKind};
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
        use crate::object::{ObjectId, Snapshot, TreeEntry, EntryKind};
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
}
