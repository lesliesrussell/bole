// bole-fkt
use crate::error::{Error, Result};
use crate::refs::{backend::RefBackend, Ref, RefName};
use std::fs;
use std::path::{Path, PathBuf};

pub struct DiskRefBackend {
    root: PathBuf,
}

impl DiskRefBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn ref_path(&self, name: &RefName) -> PathBuf {
        let mut path = self.root.join("refs");
        for segment in name.as_str().split('/') {
            path = path.join(segment);
        }
        path
    }

    fn walk_refs(&self, dir: &Path, root: &Path, prefix: &str, acc: &mut Vec<RefName>) -> Result<()> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.walk_refs(&path, root, prefix, acc)?;
            } else {
                if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                    continue;
                }
                let rel = path.strip_prefix(root).unwrap();
                let name_str: String = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if name_str.starts_with(prefix) {
                    if let Ok(ref_name) = RefName::new(name_str) {
                        acc.push(ref_name);
                    }
                }
            }
        }
        Ok(())
    }
}

impl RefBackend for DiskRefBackend {
    fn get(&self, name: &RefName) -> Result<Option<Ref>> {
        let path = self.ref_path(name);
        match fs::read(&path) {
            Ok(data) => {
                let r = postcard::from_bytes(&data).map_err(|e| Error::Codec(e.to_string()))?;
                Ok(Some(r))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set(&self, name: &RefName, r: &Ref) -> Result<()> {
        let path = self.ref_path(name);
        fs::create_dir_all(path.parent().expect("ref path always has a parent"))?;
        let data = postcard::to_allocvec(r).map_err(|e| Error::Codec(e.to_string()))?;
        let tmp = path.with_file_name(format!(
            "{}.tmp",
            path.file_name().unwrap().to_string_lossy()
        ));
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn delete(&self, name: &RefName) -> Result<()> {
        match fs::remove_file(self.ref_path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<RefName>> {
        let refs_root = self.root.join("refs");
        let mut names = Vec::new();
        self.walk_refs(&refs_root, &refs_root, prefix, &mut names)?;
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::DiskRefBackend;
    use crate::refs::{backend::RefBackend, Ref, RefName, Tag};
    use crate::object::ObjectId;
    use tempfile::TempDir;

    fn name(s: &str) -> RefName { RefName::new(s).unwrap() }
    fn tag(id: ObjectId) -> Ref { Ref::Tag(Tag { target: id, created_at: 1, message: None }) }

    #[test]
    fn set_then_get() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_some());
    }

    #[test]
    fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("nope")).unwrap().is_none());
    }

    #[test]
    fn delete_removes_entry() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("v1"), &tag(id)).unwrap();
        b.delete(&name("v1")).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_none());
    }

    #[test]
    fn list_by_prefix() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("leslie/a"), &tag(id)).unwrap();
        b.set(&name("leslie/b"), &tag(id)).unwrap();
        b.set(&name("v1"), &tag(id)).unwrap();
        let names = b.list("leslie/").unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n: &RefName| n.as_str() == "leslie/a"));
        assert!(names.iter().any(|n: &RefName| n.as_str() == "leslie/b"));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let id = ObjectId::new([1u8; 32]);
        {
            let b = DiskRefBackend::open(dir.path()).unwrap();
            b.set(&name("main"), &tag(id)).unwrap();
        }
        let b = DiskRefBackend::open(dir.path()).unwrap();
        assert!(b.get(&name("main")).unwrap().is_some());
    }

    #[test]
    fn hierarchical_name_stored_in_subdirectory() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("experiment/foo"), &tag(id)).unwrap();
        // verify the file exists at the expected path
        assert!(dir.path().join("refs/experiment/foo").exists());
    }

    #[test]
    fn dotted_ref_name_no_stem_collision() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        // "v1" and "v1.0" must be stored as distinct files
        b.set(&name("v1"), &tag(id)).unwrap();
        b.set(&name("v1.0"), &tag(id)).unwrap();
        assert!(b.get(&name("v1")).unwrap().is_some());
        assert!(b.get(&name("v1.0")).unwrap().is_some());
        // verify distinct paths on disk
        assert!(dir.path().join("refs/v1").exists());
        assert!(dir.path().join("refs/v1.0").exists());
    }
}
