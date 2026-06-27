// bole-1vi
pub mod materialize;

use std::collections::BTreeMap;
use std::path::Path;
use crate::acl::disk::DiskAclBackend;
use crate::acl::memory::MemoryAclBackend;
use crate::acl::{Accessor, AclStore, PathAcl, PathRole, Permission};
use crate::error::Result;
use crate::object::{EntryKind, Object, ObjectId};
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
        let source_head = match self.refs.get_timeline(source)? {
            Some(tl) => tl.head,
            None => return Ok(MergeCheck::Allowed),
        };
        // bole-4j3
        let source_tree = match self.objects.get(&source_head).await? {
            Some(Object::Snapshot(s)) => s.root,
            _ => return Ok(MergeCheck::Allowed),
        };
        let mut visible = BTreeMap::new();
        // bole-hc1
        let privileged = Accessor::new()
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });
        walk_tree_filtered(&self.objects, &self.acls, source_tree, "", &privileged, &mut visible).await?;
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
        from.create_timeline(RefName::new("main").unwrap(), id, TimelinePolicy::Unrestricted, 3).unwrap();
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
}
