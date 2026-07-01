// bole-49r
// bole-s5y
// bole-mhs
// bole-hto
use thiserror::Error;

// bole-p8u
/// The top-level error type returned by all fallible bole operations.
#[derive(Debug, Error)]
pub enum Error {
    // bole-p8u
    /// Serialisation or deserialisation encountered malformed or unexpected data.
    #[error("codec error: {0}")] Codec(String),
    // bole-p8u
    /// A storage backend operation failed (e.g. a backend put or get returned an error).
    #[error("storage error: {0}")] Storage(String),
    // bole-p8u
    /// An I/O error propagated from the filesystem or network layer.
    #[error("io error: {0}")] Io(#[from] std::io::Error),
    // bole-p8u
    /// A supplied string violated the `RefName` naming rules.
    #[error("invalid ref name: {0}")] InvalidRefName(String),
    // bole-p8u
    /// A ref operation targeted a ref of the wrong kind (e.g. advancing a tag's head).
    #[error("wrong ref kind: {0}")] WrongRefKind(String),
    // bole-p8u
    /// A requested operation was denied by the ACL policy for this accessor.
    #[error("access denied: {0}")] AccessDenied(String),
    // bole-p8u
    /// Decryption failed, most likely due to a wrong key or corrupted ciphertext.
    #[error("decryption failed")] DecryptionFailed,
    // bole-p8u
    /// A decrypted secret's bytes are not valid UTF-8 and cannot be used as a string.
    #[error("secret value is not valid UTF-8")] SecretNotUtf8,
    // bole-6bd
    // bole-p8u
    /// A Git projection step failed (e.g. the target path is not a valid bare repo).
    #[error("git projection failed: {0}")] GitProjection(String),
    // bole-3w9
    /// A timeline advance was rejected because it violates the timeline's [`TimelinePolicy`](crate::refs::TimelinePolicy).
    #[error("policy violation: {0}")] PolicyViolation(String),
    // bole-sk6
    #[error("transaction conflict: {0}")] TransactionConflict(String),
}

// bole-p8u
/// Convenience alias that pins the error type to [`enum@Error`].
pub type Result<T> = std::result::Result<T, Error>;
