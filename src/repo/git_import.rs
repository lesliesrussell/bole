// bole-mtq
//! Git import (git → bole), the inverse of [`super::git_projection::project_to_git`].
//!
//! Translates a git repository's branches and tags into bole timelines/tags and
//! their object closure into bole Blobs/Trees/Snapshots, keeping a persisted
//! [`IdentityMap`] sidecar so round-trips and incremental re-imports are stable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::object::ObjectId;

// bole-mtq
/// A persisted bidirectional map between git object ids (SHA-1/SHA-256 bytes) and
/// bole `ObjectId`s (BLAKE3), used to make import/export idempotent and
/// incremental.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMap {
    git_to_bole: HashMap<Vec<u8>, [u8; 32]>,
    bole_to_git: HashMap<[u8; 32], Vec<u8>>,
}

impl IdentityMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a git↔bole correspondence (both directions).
    pub fn insert(&mut self, git: Vec<u8>, bole: ObjectId) {
        self.git_to_bole.insert(git.clone(), *bole.as_bytes());
        self.bole_to_git.insert(*bole.as_bytes(), git);
    }

    /// The bole id a git id was translated to, if known.
    pub fn bole_for_git(&self, git: &[u8]) -> Option<ObjectId> {
        self.git_to_bole.get(git).map(|b| ObjectId::new(*b))
    }

    /// The git id a bole id was translated from/to, if known.
    pub fn git_for_bole(&self, bole: &ObjectId) -> Option<&[u8]> {
        self.bole_to_git.get(bole.as_bytes()).map(|v| v.as_slice())
    }

    pub fn len(&self) -> usize {
        self.git_to_bole.len()
    }

    pub fn is_empty(&self) -> bool {
        self.git_to_bole.is_empty()
    }

    /// Loads the sidecar for `source` under `dir` (empty map if absent).
    pub fn load(dir: &Path, source: &Path) -> Result<Self> {
        let path = map_path(dir, source);
        match std::fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).map_err(|e| Error::Codec(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Persists the sidecar for `source` under `dir` (atomic tmp+rename).
    pub fn save(&self, dir: &Path, source: &Path) -> Result<()> {
        let path = map_path(dir, source);
        std::fs::create_dir_all(path.parent().expect("map path has a parent")).map_err(Error::Io)?;
        let bytes = postcard::to_allocvec(self).map_err(|e| Error::Codec(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(Error::Io)?;
        std::fs::rename(&tmp, &path).map_err(Error::Io)?;
        Ok(())
    }
}

// bole-mtq
/// The sidecar path for a source: `<dir>/git-map/<fingerprint>.postcard`.
fn map_path(dir: &Path, source: &Path) -> PathBuf {
    dir.join("git-map").join(format!("{}.postcard", fingerprint(source)))
}

// bole-mtq
/// A deterministic per-source fingerprint: hex of BLAKE3 over the canonical
/// absolute path bytes (the spec's SHA-256 role — a stable filename key).
fn fingerprint(source: &Path) -> String {
    let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex().to_string()
}

// bole-mtq
/// Sanitizes a git ref name into a bole-legal `RefName` string, or `None` if it
/// is empty after sanitization. Deterministic: `.hidden` → `_hidden`,
/// `feature/.foo` → `feature/_foo`; NUL bytes stripped; consecutive slashes
/// collapsed. Slashes are otherwise preserved.
pub fn sanitize_ref_name(git_name: &str) -> Option<String> {
    let segments: Vec<String> = git_name
        .split('/')
        .filter(|s| !s.is_empty()) // collapse consecutive/leading/trailing slashes
        .map(|s| {
            let s: String = s.chars().filter(|c| *c != '\0').collect();
            if let Some(rest) = s.strip_prefix('.') {
                format!("_{rest}")
            } else {
                s
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_roundtrip_postcard() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = dir.path().join("some-git-repo");
        std::fs::create_dir_all(&source).unwrap();

        let mut map = IdentityMap::new();
        let bole = ObjectId::from_content(b"snapshot");
        map.insert(vec![0xab; 20], bole);
        map.save(dir.path(), &source).unwrap();

        let loaded = IdentityMap::load(dir.path(), &source).unwrap();
        assert_eq!(loaded, map);
        assert_eq!(loaded.bole_for_git(&[0xab; 20]), Some(bole));
        assert_eq!(loaded.git_for_bole(&bole), Some(&[0xab; 20][..]));
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_missing_map_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let map = IdentityMap::load(dir.path(), Path::new("/nonexistent/repo")).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn sanitize_dot_prefixed_segments() {
        assert_eq!(sanitize_ref_name(".hidden").as_deref(), Some("_hidden"));
        assert_eq!(sanitize_ref_name("feature/.foo").as_deref(), Some("feature/_foo"));
        assert_eq!(sanitize_ref_name("main").as_deref(), Some("main"));
        assert_eq!(sanitize_ref_name("a//b").as_deref(), Some("a/b"));
        assert_eq!(sanitize_ref_name("release/1.0").as_deref(), Some("release/1.0"));
    }

    #[test]
    fn sanitize_empty_is_none() {
        assert_eq!(sanitize_ref_name(""), None);
        assert_eq!(sanitize_ref_name("///"), None);
    }

    #[test]
    fn different_sources_get_different_fingerprints() {
        let a = fingerprint(Path::new("/tmp/repo-a"));
        let b = fingerprint(Path::new("/tmp/repo-b"));
        assert_ne!(a, b);
        // Deterministic for the same path.
        assert_eq!(a, fingerprint(Path::new("/tmp/repo-a")));
    }
}
