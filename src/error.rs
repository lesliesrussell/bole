// bole-49r
// bole-s5y
// bole-mhs
// bole-hto
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("codec error: {0}")] Codec(String),
    #[error("storage error: {0}")] Storage(String),
    #[error("io error: {0}")] Io(#[from] std::io::Error),
    #[error("invalid ref name: {0}")] InvalidRefName(String),
    #[error("wrong ref kind: {0}")] WrongRefKind(String),
    #[error("access denied: {0}")] AccessDenied(String),
    #[error("decryption failed")] DecryptionFailed,
    #[error("secret value is not valid UTF-8")] SecretNotUtf8,
    // bole-6bd
    #[error("git projection failed: {0}")] GitProjection(String),
}

pub type Result<T> = std::result::Result<T, Error>;
