// bole-4fp
use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

// bole-0u9
fn sanitize(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F").replace('*', "%2A")
}


pub struct DiskAclBackend {
    root: PathBuf,
}

impl DiskAclBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("acls").join("paths"))?;
        fs::create_dir_all(root.join("acls").join("timelines"))?;
        Ok(Self { root })
    }

    fn path_acl_file(&self, glob: &str) -> PathBuf {
        self.root.join("acls").join("paths").join(sanitize(glob))
    }

    fn timeline_acl_file(&self, pattern: &str) -> PathBuf {
        self.root.join("acls").join("timelines").join(sanitize(pattern))
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> Result<()> {
        let tmp_name = format!(".{}.tmp",
            path.file_name().unwrap().to_string_lossy());
        let tmp = path.parent().unwrap().join(tmp_name);
        fs::write(&tmp, data)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    fn list_dir<T, F>(&self, dir: PathBuf, decode: F) -> Result<Vec<T>>
    where
        F: Fn(&str, &[u8]) -> Result<T>,
    {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') { continue; }
            let data = fs::read(entry.path())?;
            out.push(decode(&name, &data)?);
        }
        Ok(out)
    }
}

// bole-4fp
impl AclBackend for DiskAclBackend {
    fn get_path_acl(&self, glob: &str) -> Result<Option<PathAcl>> {
        let path = self.path_acl_file(glob);
        match fs::read(&path) {
            Ok(data) => Ok(Some(postcard::from_bytes(&data)
                .map_err(|e| Error::Codec(e.to_string()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set_path_acl(&self, acl: &PathAcl) -> Result<()> {
        let data = postcard::to_allocvec(acl).map_err(|e| Error::Codec(e.to_string()))?;
        self.atomic_write(&self.path_acl_file(&acl.glob), &data)
    }

    fn delete_path_acl(&self, glob: &str) -> Result<()> {
        match fs::remove_file(self.path_acl_file(glob)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list_path_acls(&self) -> Result<Vec<PathAcl>> {
        self.list_dir(
            self.root.join("acls").join("paths"),
            |_name, data| postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string())),
        )
    }

    fn get_timeline_acl(&self, pattern: &str) -> Result<Option<TimelineAcl>> {
        let path = self.timeline_acl_file(pattern);
        match fs::read(&path) {
            Ok(data) => Ok(Some(postcard::from_bytes(&data)
                .map_err(|e| Error::Codec(e.to_string()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn set_timeline_acl(&self, acl: &TimelineAcl) -> Result<()> {
        let data = postcard::to_allocvec(acl).map_err(|e| Error::Codec(e.to_string()))?;
        self.atomic_write(&self.timeline_acl_file(&acl.pattern), &data)
    }

    fn delete_timeline_acl(&self, pattern: &str) -> Result<()> {
        match fs::remove_file(self.timeline_acl_file(pattern)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn list_timeline_acls(&self) -> Result<Vec<TimelineAcl>> {
        self.list_dir(
            self.root.join("acls").join("timelines"),
            |_name, data| postcard::from_bytes(data).map_err(|e| Error::Codec(e.to_string())),
        )
    }
}

// bole-4fp
#[cfg(test)]
mod tests {
    use super::DiskAclBackend;
    use crate::acl::{backend::AclBackend, PathAcl, TimelineAcl};
    use tempfile::TempDir;

    #[test]
    fn path_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "secrets/**".into() };
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("secrets/**").unwrap(), Some(acl));
        b.delete_path_acl("secrets/**").unwrap();
        assert!(b.get_path_acl("secrets/**").unwrap().is_none());
    }

    #[test]
    fn timeline_acl_set_get_delete() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = TimelineAcl { pattern: "leslie/private/**".into() };
        b.set_timeline_acl(&acl).unwrap();
        assert_eq!(b.get_timeline_acl("leslie/private/**").unwrap(), Some(acl));
        b.delete_timeline_acl("leslie/private/**").unwrap();
        assert!(b.get_timeline_acl("leslie/private/**").unwrap().is_none());
    }

    #[test]
    fn persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let b = DiskAclBackend::open(dir.path()).unwrap();
            b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
            b.set_timeline_acl(&TimelineAcl { pattern: "private/**".into() }).unwrap();
        }
        let b2 = DiskAclBackend::open(dir.path()).unwrap();
        assert!(b2.get_path_acl("secrets/**").unwrap().is_some());
        assert!(b2.get_timeline_acl("private/**").unwrap().is_some());
    }

    #[test]
    fn list_returns_all() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
        b.set_path_acl(&PathAcl { glob: "notes/**".into() }).unwrap();
        let mut list = b.list_path_acls().unwrap();
        list.sort_by(|a, c| a.glob.cmp(&c.glob));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].glob, "notes/**");
        assert_eq!(list[1].glob, "secrets/**");
    }

    #[test]
    fn glob_with_slash_roundtrips() {
        let dir = TempDir::new().unwrap();
        let b = DiskAclBackend::open(dir.path()).unwrap();
        let acl = PathAcl { glob: "a/b/**".into() };
        b.set_path_acl(&acl).unwrap();
        assert_eq!(b.get_path_acl("a/b/**").unwrap(), Some(acl));
    }

    // bole-fo2
    #[test]
    fn old_disk_acls_project_into_two_point_ruleset() {
        use crate::acl::backend::AclBackend;
        use crate::acl::lattice::Label;
        use crate::acl::rules::LabelRule;
        let dir = TempDir::new().unwrap();
        {
            let b = DiskAclBackend::open(dir.path()).unwrap();
            b.set_path_acl(&PathAcl { glob: "secrets/**".into() }).unwrap();
            b.set_timeline_acl(&TimelineAcl { pattern: "leslie/private/**".into() }).unwrap();
        }
        let b2 = DiskAclBackend::open(dir.path()).unwrap();
        let rs = b2.get_label_ruleset().unwrap();
        assert!(rs.rules.iter().any(|r| matches!(
            r, LabelRule::Path { glob, label } if glob == "secrets/**" && *label == Label::protected()
        )));
        assert!(rs.rules.iter().any(|r| matches!(
            r, LabelRule::Timeline { pattern, label }
                if pattern == "leslie/private/**" && *label == Label::protected()
        )));
    }
}
