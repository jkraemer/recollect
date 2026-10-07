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
    #[error(
        "this is a development build; set RECOLLECT_DATA_DIR to a scratch directory (it does not open the default data directory, whose database it would migrate)"
    )]
    DevelopmentBuildWithoutDataDir,
    #[error(
        "this is a development build; set RECOLLECT_DOWNLOAD_BASE to the releases it may look up and install"
    )]
    DevelopmentBuildWithoutDownloadBase,
    #[error("could not look up the latest release at {url}: {reason}")]
    ReleaseLookup { url: String, reason: String },
    #[error("invalid hook input: {0}")]
    HookInput(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {message}", path.display())]
    RubyData { path: PathBuf, message: String },
    #[error("no peer named {0:?}")]
    UnknownPeer(String),
    #[error(
        "a peer named {0:?} already exists; remove it first with: recollect peer remove {0}, or, if it is a different machine, give one of the two another name (name in the [sync] table of config.toml)"
    )]
    PeerExists(String),
    #[error(
        "this machine is already paired with that key, as peer {0:?}; to pair again, remove it first with: recollect peer remove {0}"
    )]
    AlreadyPaired(String),
    #[error("this invite is not valid (expired or already used)")]
    InvalidInvite,
    #[error("invalid invite: {0}")]
    MalformedInvite(String),
    #[error("invalid address {0:?}: use host:port, such as foehn:7327")]
    InvalidAddress(String),
    #[error("{}: {message}", path.display())]
    Identity { path: PathBuf, message: String },
    #[error(
        "peer {peer} speaks sync protocol {theirs}, this recollect speaks {ours}; upgrade the older one"
    )]
    ProtocolMismatch {
        peer: String,
        theirs: u32,
        ours: u32,
    },
    /// A sync failure that needs no variant of its own; the text is the whole message.
    #[error("{0}")]
    Sync(String),
    /// Another machine could not be reached: its address did not resolve or
    /// nothing accepted the connection. The text is the whole message.
    #[error("{0}")]
    Unreachable(String),
    /// A sync round that failed after this machine had stored what the peer
    /// sent: `received` live memories, which still need embedding. Reads as
    /// the failure itself.
    #[error("{source}")]
    RoundFailedAfterApply { received: usize, source: Box<Error> },
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
