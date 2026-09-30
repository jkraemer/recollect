//! The library's error type; the CLI prints each variant as one line.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("memory {0} not found")]
    NotFound(i64),
    #[error("invalid project name {0:?}: allowed are a-z 0-9 . _ -")]
    InvalidProject(String),
    #[error("invalid tag {0:?}: tags must not be empty")]
    InvalidTag(String),
    #[error("content must not be empty")]
    EmptyContent,
    #[error("search query must contain at least one term")]
    EmptyQuery,
    #[error("invalid date {0:?}: use YYYY-MM-DD or an RFC 3339 timestamp")]
    InvalidDate(String),
    #[error("embedding unavailable: {0}")]
    EmbeddingUnavailable(String),
    #[error(
        "stored vectors come from {stored}, but the model is {current}; run recollect reindex --all"
    )]
    ModelMismatch { stored: String, current: String },
    #[error(
        "database was created by a newer recollect (schema version {found}, this binary supports up to {supported})"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("invalid config {path}: {message}")]
    Config { path: String, message: String },
    #[error("HOME is not set; set RECOLLECT_DATA_DIR to choose the data directory")]
    NoDataDir,
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {message}", path.display())]
    RubyData { path: PathBuf, message: String },
}

impl Error {
    /// Wraps an i/o failure on `path`, for `map_err`.
    pub fn file(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
        move |source| Error::File {
            path: path.to_path_buf(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
