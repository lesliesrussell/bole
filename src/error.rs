// bole-49r
// bole-s5y
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("codec error: {0}")]
    Codec(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid ref name: {0}")]
    InvalidRefName(String),
    #[error("wrong ref kind: {0}")]
    WrongRefKind(String),
}

pub type Result<T> = std::result::Result<T, Error>;
