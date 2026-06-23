//! Crate-wide error type.

use std::path::PathBuf;

/// Result alias used throughout bpkg-core.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    IoBare(#[from] std::io::Error),

    #[error("not a valid .bpkg file (bad magic header)")]
    BadMagic,

    #[error("unsupported .bpkg format version: {0} (this build supports {1})")]
    UnsupportedVersion(u16, u16),

    #[error("the package is corrupt: {0}")]
    Corrupt(String),

    #[error("integrity check failed for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("failed to parse installer.toml: {0}")]
    Config(#[from] toml::de::Error),

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("compression error: {0}")]
    Compression(String),

    #[error("{0} is not supported on this platform")]
    UnsupportedPlatform(&'static str),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Attach a path to an I/O error for friendlier messages.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
