// bole-fkt
use crate::error::{Error, Result};
use crate::refs::{backend::RefBackend, Ref, RefName};
use std::fs;
use std::path::{Path, PathBuf};

// bole-0x3: process-wide monotonic counter for unique journal filenames.
static JOURNAL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct DiskRefBackend {
    root: PathBuf,
}

impl DiskRefBackend {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let backend = Self { root };
        // bole-sk6: complete any transaction interrupted before its journal was deleted.
        backend.recover()?;
        Ok(backend)
    }

    // bole-sk6
    fn txn_dir(&self) -> PathBuf {
        self.root.join("refs").join(".txn")
    }

    fn ref_path(&self, name: &RefName) -> PathBuf {
        let mut path = self.root.join("refs");
        for segment in name.as_str().split('/') {
            // bole-daf: defense-in-depth. RefName::new / its Deserialize already
            // reject `.`/`..`/empty/NUL segments, so this never fires for a valid
            // RefName; the guard makes store-escape impossible even if a future
            // construction path regresses.
            debug_assert!(
                !segment.is_empty() && segment != "." && segment != ".." && !segment.contains('\0'),
                "ref_path given an unsafe segment: {segment:?}"
            );
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
                // bole-sk6: skip hidden dirs (e.g. the .txn journal dir).
                if path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false) {
                    continue;
                }
                self.walk_refs(&path, root, prefix, acc)?;
            } else {
                if path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(false)
                {
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
            ".{}.tmp",
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

    // bole-sk6
    /// Write-ahead journal commit: record the plan's absolute final values,
    /// fsync (the commit point), apply each set/delete, then delete the journal.
    /// A crash before the fsync leaves no durable journal (the txn never
    /// happened); a crash after it is completed idempotently by `recover`.
    fn apply_atomic(&self, plan: &[(RefName, Option<Ref>)]) -> Result<()> {
        if plan.is_empty() {
            return Ok(());
        }
        let txn_dir = self.txn_dir();
        fs::create_dir_all(&txn_dir)?;
        let bytes = postcard::to_allocvec(&plan.to_vec()).map_err(|e| Error::Codec(e.to_string()))?;
        let txid = blake3::hash(&bytes).to_hex().to_string();
        // bole-0x3: make the journal/temp names unique per commit. Deriving them
        // from the plan-content hash alone means two *identical* plans committed
        // concurrently (across processes — one process is serialized by the
        // RefStore lock, bole-bti) collide on the journal + temp paths: they
        // clobber each other's temp file, and one can delete the other's journal
        // before its owner finishes applying. A (pid, sequence) nonce makes each
        // commit's paths unique; `recover` still matches any `*.journal`.
        let seq = JOURNAL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = format!("{}-{}", std::process::id(), seq);
        let journal = txn_dir.join(format!("{txid}-{nonce}.journal"));
        let tmp = txn_dir.join(format!(".{txid}-{nonce}.journal.tmp"));
        {
            use std::io::Write as _;
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?; // commit point once renamed
        }
        fs::rename(&tmp, &journal)?;
        if let Ok(d) = fs::File::open(&txn_dir) {
            let _ = d.sync_all();
        }
        apply_plan(self, plan)?;
        fs::remove_file(&journal)?;
        Ok(())
    }

    // bole-sk6
    fn recover(&self) -> Result<()> {
        let txn_dir = self.txn_dir();
        let rd = match fs::read_dir(&txn_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in rd {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("journal") {
                continue;
            }
            let bytes = fs::read(&path)?;
            // Absolute-value plan → idempotent replay (overwrite partial state).
            let plan: Vec<(RefName, Option<Ref>)> =
                postcard::from_bytes(&bytes).map_err(|e| Error::Codec(e.to_string()))?;
            apply_plan(self, &plan)?;
            fs::remove_file(&path)?;
        }
        Ok(())
    }
}

// bole-sk6
/// Applies each ref set/delete in a resolved plan (no journal — the caller owns
/// journaling/recovery).
fn apply_plan(backend: &DiskRefBackend, plan: &[(RefName, Option<Ref>)]) -> Result<()> {
    for (name, val) in plan {
        match val {
            Some(r) => backend.set(name, r)?,
            None => backend.delete(name)?,
        }
    }
    Ok(())
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
    fn ref_named_dot_tmp_appears_in_list() {
        let dir = TempDir::new().unwrap();
        let b = DiskRefBackend::open(dir.path()).unwrap();
        let id = ObjectId::new([1u8; 32]);
        b.set(&name("foo.tmp"), &tag(id)).unwrap();
        let names = b.list("").unwrap();
        assert!(names.iter().any(|n| n.as_str() == "foo.tmp"));
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
