// bole-49r
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("codec error: {0}")]
    Codec(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
