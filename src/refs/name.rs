// bole-ht9
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefName(String);

impl RefName {
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
            if segment == ".." {
                return Err(Error::InvalidRefName("'..' segment not allowed".into()));
            }
            if segment.contains('\0') {
                return Err(Error::InvalidRefName("null byte in segment".into()));
            }
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

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
}
