// bole-ht9
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

// bole-p8u
/// A validated, slash-delimited hierarchical name for a ref (tag or timeline).
///
/// `RefName` enforces a minimal set of naming rules: segments must be non-empty,
/// must not start with `.`, must not contain null bytes, and the name must not
/// begin or end with `/`.  These rules mirror Git's ref naming restrictions to
/// keep projection straightforward.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct RefName(String);

// bole-daf
/// Manual `Deserialize` so a decoded `RefName` (e.g. from an untrusted sync wire
/// message) is validated through [`RefName::new`], not constructed raw. The
/// derived impl skipped validation, letting a peer inject a name containing
/// `..`/leading-`.`/NUL that `ref_path` would then join into a filesystem path
/// escaping `<root>/refs` (arbitrary-file write). Every deserialized `RefName`
/// now satisfies the same invariants as one built by `new`.
impl<'de> Deserialize<'de> for RefName {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        RefName::new(s).map_err(serde::de::Error::custom)
    }
}

impl RefName {
    // bole-p8u
    /// Validates and wraps `s` as a `RefName`.
    ///
    /// Returns [`crate::error::Error::InvalidRefName`] if the string violates
    /// any naming rule.
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        if s.is_empty() {
            return Err(Error::InvalidRefName("name must not be empty".into()));
        }
        if s.starts_with('/') {
            return Err(Error::InvalidRefName("name must not start with '/'".into()));
        }
        if s.ends_with('/') {
            return Err(Error::InvalidRefName("name must not end with '/'".into()));
        }
        for segment in s.split('/') {
            if segment.is_empty() {
                return Err(Error::InvalidRefName(
                    "consecutive slashes produce empty segment".into(),
                ));
            }
            if segment.starts_with('.') {
                return Err(Error::InvalidRefName(
                    "segment must not start with '.'".into(),
                ));
            }
            if segment.contains('\0') {
                return Err(Error::InvalidRefName("null byte in segment".into()));
            }
        }
        Ok(Self(s))
    }

    // bole-p8u
    /// Returns the name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    // bole-p8u
    /// Returns the portion of the name before the last `/`, or an empty string if there is no `/`.
    pub fn prefix(&self) -> &str {
        match self.0.rfind('/') {
            Some(i) => &self.0[..i],
            None => "",
        }
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::RefName;

    #[test]
    fn valid_simple() {
        let n = RefName::new("v1").unwrap();
        assert_eq!(n.as_str(), "v1");
    }

    #[test]
    fn valid_hierarchical() {
        let n = RefName::new("experiment/foo").unwrap();
        assert_eq!(n.as_str(), "experiment/foo");
        assert_eq!(n.prefix(), "experiment");
    }

    #[test]
    fn prefix_no_slash() {
        let n = RefName::new("main").unwrap();
        assert_eq!(n.prefix(), "");
    }

    #[test]
    fn rejects_empty() {
        assert!(RefName::new("").is_err());
    }

    #[test]
    fn rejects_leading_slash() {
        assert!(RefName::new("/foo").is_err());
    }

    #[test]
    fn rejects_trailing_slash() {
        assert!(RefName::new("foo/").is_err());
    }

    #[test]
    fn rejects_consecutive_slashes() {
        assert!(RefName::new("a//b").is_err());
    }

    #[test]
    fn rejects_dotdot() {
        assert!(RefName::new("../etc/passwd").is_err());
    }

    #[test]
    fn display() {
        let n = RefName::new("leslie/exp-foo").unwrap();
        assert_eq!(n.to_string(), "leslie/exp-foo");
    }

    #[test]
    fn rejects_single_dot() {
        assert!(RefName::new(".").is_err());
        assert!(RefName::new("a/./b").is_err());
    }

    #[test]
    fn rejects_leading_dot_segment() {
        assert!(RefName::new(".foo").is_err());
        assert!(RefName::new("a/.foo").is_err());
        assert!(RefName::new(".foo/bar").is_err());
    }

    // bole-daf
    #[test]
    fn deserialize_rejects_path_traversal() {
        // A wire payload that encodes a raw traversal string as a RefName must be
        // rejected at decode — the derived Deserialize used to accept it, which is
        // how a pushed ref name could escape <root>/refs on disk.
        for evil in ["../../etc/passwd", "/abs", "a/../../b", "a//b", ".hidden", "x\0y"] {
            let bytes = postcard::to_allocvec(&evil.to_string()).unwrap();
            let decoded: std::result::Result<RefName, _> = postcard::from_bytes(&bytes);
            assert!(decoded.is_err(), "RefName deserialize must reject {evil:?}");
        }
        // A valid name still round-trips through the wire codec.
        let ok = RefName::new("release/1.0").unwrap();
        let bytes = postcard::to_allocvec(&ok).unwrap();
        assert_eq!(postcard::from_bytes::<RefName>(&bytes).unwrap(), ok);
    }
}
