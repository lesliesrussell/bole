// bole-1vi
pub mod materialize;

use std::path::Path;
use crate::error::Result;
use crate::refs::{DiskRefBackend, MemoryRefBackend, RefStore};
use crate::store::{disk::DiskBackend, memory::MemoryBackend, ObjectStore};

pub struct Repository {
    pub objects: ObjectStore,
    pub refs: RefStore,
}

// bole-1vi
impl Repository {
    pub fn memory() -> Self {
        Self {
            objects: ObjectStore::new(MemoryBackend::new()),
            refs: RefStore::new(MemoryRefBackend::new()),
        }
    }

    pub async fn disk(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Ok(Self {
            objects: ObjectStore::new(DiskBackend::open(root).await?),
            refs: RefStore::new(DiskRefBackend::open(root)?),
        })
    }

    pub async fn copy_to(&self, dest: &Repository) -> Result<()> {
        copy_objects(&self.objects, &dest.objects).await?;
        copy_refs(&self.refs, &dest.refs)?;
        Ok(())
    }
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
        from.create_timeline(
            RefName::new("main").unwrap(),
            id,
            TimelinePolicy::Unrestricted,
            3,
        )
        .unwrap();
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
}
