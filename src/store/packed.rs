// bole-81z
//! `PackedDiskBackend`: a loose `DiskBackend` composed with a set of immutable
//! packs. Reads are loose-first then packs (newest-first); writes always land
//! loose (`put` is idempotent, no transaction). Packs are produced only by
//! `repack`/GC and are never mutated in place.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::store::backend::StorageBackend;
use crate::store::disk::DiskBackend;
use crate::store::pack::{decode_frame_public, PackBuilder, PackIndex};

// bole-81z
/// One loaded pack: its parsed index plus the path to its `.pack` file.
pub(crate) struct Pack {
    pub(crate) idx: PackIndex,
    pub(crate) pack_path: PathBuf,
}

// bole-81z
/// A composed backend: loose objects plus immutable packs.
pub struct PackedDiskBackend {
    root: PathBuf,
    loose: DiskBackend,
    // Newest packs first (probed in order). Guarded for repack/GC swaps.
    packs: std::sync::RwLock<Vec<Pack>>,
}

impl PackedDiskBackend {
    /// Opens (or creates) a packed repo rooted at `root`, loading every pack in
    /// `packs/` at startup. A repo with no `packs/` behaves like a loose repo.
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let loose = DiskBackend::open(&root).await?;
        let packs = load_packs(&root)?;
        Ok(Self { root, loose, packs: std::sync::RwLock::new(packs) })
    }

    pub(crate) fn packs_dir(&self) -> PathBuf {
        self.root.join("packs")
    }

    #[cfg(test)]
    pub(crate) fn loose(&self) -> &DiskBackend {
        &self.loose
    }

    /// Re-scans `packs/` and replaces the in-memory pack set (after repack/GC).
    pub(crate) fn reload_packs(&self) -> Result<()> {
        let packs = load_packs(&self.root)?;
        *self.packs.write().unwrap() = packs;
        Ok(())
    }

    /// Consolidates all current loose objects into one immutable pack, then
    /// deletes the packed loose copies. Crash-safe: a crash before the pack+idx
    /// rename leaves only ignored tmp files; a crash after leaves objects in both
    /// forms (harmless — reads are loose-first and ids are content addresses).
    /// Returns the new pack digest, or `None` if there were no loose objects.
    pub async fn repack(&self) -> Result<Option<[u8; 32]>> {
        self.repack_keeping(None).await
    }

    /// The repack engine. If `keep` is `Some(set)`, only ids in `set` are copied
    /// forward (GC's copy-forward); loose objects are removed regardless of
    /// membership when `keep` is `None`. Existing packs' reachable objects are
    /// always folded in so a GC repack supersedes them.
    pub(crate) async fn repack_keeping(
        &self,
        keep: Option<&std::collections::HashSet<ObjectId>>,
    ) -> Result<Option<[u8; 32]>> {
        // Gather the object set: loose + already-packed, filtered by `keep`.
        let mut builder = PackBuilder::new();
        let mut sources: Vec<ObjectId> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let loose_ids = self.loose.list().await?;
        let packed_ids: Vec<ObjectId> = {
            let packs = self.packs.read().unwrap();
            packs.iter().flat_map(|p| p.idx.ids().copied().collect::<Vec<_>>()).collect()
        };
        for id in loose_ids.iter().chain(packed_ids.iter()) {
            if !seen.insert(*id) {
                continue;
            }
            if let Some(keep) = keep {
                if !keep.contains(id) {
                    continue;
                }
            }
            if let Some(bytes) = self.get(id).await? {
                builder.add(*id, bytes.to_vec());
                sources.push(*id);
            }
        }
        if builder.is_empty() {
            // Nothing to keep: retire any old packs (GC) and drop loose garbage.
            if keep.is_some() {
                self.retire_all_packs()?;
            }
            return Ok(None);
        }

        let (pack_bytes, entries, digest) = builder.finish()?;
        let idx_bytes = PackIndex::build(entries, digest).encode();
        let packs_dir = self.packs_dir();
        let hex = hex32(&digest);
        let old_packs = {
            let packs = self.packs.read().unwrap();
            packs.iter().map(|p| p.pack_path.clone()).collect::<Vec<_>>()
        };
        tokio::task::spawn_blocking(move || {
            write_pack_atomic(&packs_dir, &hex, &pack_bytes, &idx_bytes)?;
            // Retire superseded packs only after the new pack is durable.
            for p in &old_packs {
                let _ = std::fs::remove_file(p);
                let _ = std::fs::remove_file(p.with_extension("idx"));
            }
            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| Error::Storage(e.to_string()))??;

        self.reload_packs()?;
        // Only now remove the loose copies that are safely in the new pack.
        for id in &sources {
            self.loose.delete(id).await?;
        }
        Ok(Some(digest))
    }

    /// Unlinks every current pack (used by GC when nothing is reachable).
    fn retire_all_packs(&self) -> Result<()> {
        let paths = {
            let packs = self.packs.read().unwrap();
            packs.iter().map(|p| p.pack_path.clone()).collect::<Vec<_>>()
        };
        for p in &paths {
            let _ = std::fs::remove_file(p);
            let _ = std::fs::remove_file(p.with_extension("idx"));
        }
        self.reload_packs()
    }

    /// True if any loaded pack contains `id`.
    fn in_any_pack(&self, id: &ObjectId) -> Option<(PathBuf, u64, u64)> {
        let packs = self.packs.read().unwrap();
        for p in packs.iter() {
            if let Some((off, len)) = p.idx.lookup(id) {
                return Some((p.pack_path.clone(), off, len));
            }
        }
        None
    }
}

// bole-81z
/// Loads every `packs/pack-*.idx` and pairs it with its `.pack` sibling.
fn load_packs(root: &Path) -> Result<Vec<Pack>> {
    let dir = root.join("packs");
    let mut packs = Vec::new();
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(packs),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut idx_paths: Vec<PathBuf> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(Error::Io)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("idx") {
            idx_paths.push(path);
        }
    }
    // Newest-first by filename (pack-<digest>); load order is a probe order, not
    // a correctness property (ids are unique across packs by content).
    idx_paths.sort();
    idx_paths.reverse();
    for idx_path in idx_paths {
        let bytes = std::fs::read(&idx_path).map_err(Error::Io)?;
        let idx = PackIndex::parse(&bytes)?;
        let pack_path = idx_path.with_extension("pack");
        if !pack_path.exists() {
            // An index with no pack is unusable; skip it.
            continue;
        }
        packs.push(Pack { idx, pack_path });
    }
    Ok(packs)
}

// bole-81z
/// Lowercase-hex of a 32-byte digest (for content-addressed pack file names).
fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// bole-81z
/// Writes `pack-<hex>.{pack,idx}` durably: tmp write + fsync + rename each, then
/// fsync the packs dir. A crash before the renames leaves only invisible tmp
/// files; the `.idx` is renamed last so a pack is never visible without its index.
fn write_pack_atomic(packs_dir: &Path, hex: &str, pack: &[u8], idx: &[u8]) -> Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(packs_dir).map_err(Error::Io)?;
    let write_synced = |path: &Path, data: &[u8]| -> Result<()> {
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::File::create(&tmp).map_err(Error::Io)?;
        f.write_all(data).map_err(Error::Io)?;
        f.sync_all().map_err(Error::Io)?;
        std::fs::rename(&tmp, path).map_err(Error::Io)?;
        Ok(())
    };
    let pack_path = packs_dir.join(format!("pack-{hex}.pack"));
    let idx_path = packs_dir.join(format!("pack-{hex}.idx"));
    write_synced(&pack_path, pack)?;
    write_synced(&idx_path, idx)?; // idx last: pack is invisible without it
    if let Ok(dir) = std::fs::File::open(packs_dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

// bole-81z
/// Reads `len` bytes at `offset` from a pack file and decodes+verifies the frame.
fn read_pack_frame(pack_path: PathBuf, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(&pack_path).map_err(Error::Io)?;
    f.seek(SeekFrom::Start(offset)).map_err(Error::Io)?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).map_err(Error::Io)?;
    let (_id, canonical) = decode_frame_public(&buf)?;
    Ok(canonical)
}

#[async_trait]
impl StorageBackend for PackedDiskBackend {
    async fn put(&self, id: &ObjectId, data: &[u8]) -> Result<()> {
        // Idempotent: a copy already in a pack means no loose write is needed.
        if self.in_any_pack(id).is_some() {
            return Ok(());
        }
        self.loose.put(id, data).await
    }

    async fn get(&self, id: &ObjectId) -> Result<Option<Bytes>> {
        // Loose-first (read-after-write, recent writes).
        if let Some(b) = self.loose.get(id).await? {
            return Ok(Some(b));
        }
        if let Some((path, off, len)) = self.in_any_pack(id) {
            let canonical =
                tokio::task::spawn_blocking(move || read_pack_frame(path, off, len))
                    .await
                    .map_err(|e| Error::Storage(e.to_string()))??;
            return Ok(Some(Bytes::from(canonical)));
        }
        Ok(None)
    }

    async fn exists(&self, id: &ObjectId) -> Result<bool> {
        if self.loose.exists(id).await? {
            return Ok(true);
        }
        Ok(self.in_any_pack(id).is_some())
    }

    async fn delete(&self, id: &ObjectId) -> Result<()> {
        // Loose only; packed objects are immutable and leave via repack/GC.
        self.loose.delete(id).await
    }

    async fn list(&self) -> Result<Vec<ObjectId>> {
        use std::collections::BTreeSet;
        let mut ids: BTreeSet<ObjectId> = self.loose.list().await?.into_iter().collect();
        {
            let packs = self.packs.read().unwrap();
            for p in packs.iter() {
                for id in p.idx.ids() {
                    ids.insert(*id);
                }
            }
        }
        Ok(ids.into_iter().collect())
    }

    async fn count(&self) -> Result<u64> {
        // Σ pack counts + loose objects not already present in a pack (exact,
        // and cheap because the loose set is small after a repack).
        let loose = self.loose.list().await?;
        let packs = self.packs.read().unwrap();
        let packed: u64 = packs.iter().map(|p| p.idx.count() as u64).sum();
        let loose_extra = loose
            .iter()
            .filter(|id| !packs.iter().any(|p| p.idx.lookup(id).is_some()))
            .count() as u64;
        Ok(packed + loose_extra)
    }

    async fn sweep(
        &self,
        reachable: &std::collections::HashSet<ObjectId>,
        grace_secs: u64,
        now: u64,
    ) -> Result<u64> {
        let before = self.count().await?;
        // 1. Rewrite packs keeping only reachable (drops packed garbage and
        //    consolidates; reachable loose are folded into the new pack).
        self.repack_keeping(Some(reachable)).await?;
        // 2. Delete unreachable loose objects older than the grace window.
        for id in self.loose.list().await? {
            if reachable.contains(&id) {
                continue;
            }
            if let Some(mtime) = loose_mtime_secs(&self.root, &id) {
                if now < mtime.saturating_add(grace_secs) {
                    continue; // within grace: protect the write-before-ref race
                }
            }
            self.loose.delete(&id).await?;
        }
        let after = self.count().await?;
        Ok(before.saturating_sub(after))
    }
}

// bole-81z
/// The loose on-disk path for `id` (mirrors `DiskBackend::object_path`).
fn loose_object_path(root: &Path, id: &ObjectId) -> PathBuf {
    let hex = id.to_string();
    root.join("objects").join(&hex[..2]).join(&hex[2..])
}

// bole-81z
/// The mtime of a loose object in unix seconds, if it exists.
fn loose_mtime_secs(root: &Path, id: &ObjectId) -> Option<u64> {
    let md = std::fs::metadata(loose_object_path(root, id)).ok()?;
    md.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(bytes: &[u8]) -> ObjectId {
        ObjectId::from_content(bytes)
    }

    #[tokio::test]
    async fn behaves_like_loose_when_no_packs() {
        let dir = tempfile::TempDir::new().unwrap();
        let b = PackedDiskBackend::open(dir.path()).await.unwrap();
        let id = oid(b"hello");
        assert!(!b.exists(&id).await.unwrap());
        b.put(&id, b"hello").await.unwrap();
        assert!(b.exists(&id).await.unwrap());
        assert_eq!(b.get(&id).await.unwrap().unwrap().as_ref(), b"hello");
        assert_eq!(b.count().await.unwrap(), 1);
        b.delete(&id).await.unwrap();
        assert!(!b.exists(&id).await.unwrap());
    }

    #[tokio::test]
    async fn repack_moves_loose_into_pack() {
        let dir = tempfile::TempDir::new().unwrap();
        let b = PackedDiskBackend::open(dir.path()).await.unwrap();
        let objs: Vec<_> = (0u8..6)
            .map(|n| {
                let data = vec![n; (n as usize) + 4];
                (oid(&data), data)
            })
            .collect();
        for (id, data) in &objs {
            b.put(id, data).await.unwrap();
        }

        let digest = b.repack().await.unwrap();
        assert!(digest.is_some());
        // Loose store is now empty; everything lives in the pack.
        assert!(b.loose().list().await.unwrap().is_empty());
        for (id, data) in &objs {
            assert!(b.exists(id).await.unwrap());
            assert_eq!(b.get(id).await.unwrap().unwrap().as_ref(), data.as_slice());
        }
        assert_eq!(b.count().await.unwrap(), 6);

        // put of a packed id is a no-op (stays out of loose).
        b.put(&objs[0].0, &objs[0].1).await.unwrap();
        assert!(b.loose().list().await.unwrap().is_empty());

        // Reopening reloads the pack.
        let b2 = PackedDiskBackend::open(dir.path()).await.unwrap();
        assert_eq!(b2.count().await.unwrap(), 6);
        assert_eq!(b2.get(&objs[3].0).await.unwrap().unwrap().as_ref(), objs[3].1.as_slice());
    }
}
