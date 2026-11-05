use std::path::PathBuf;

/// Errors produced by the read-only Git engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Git(#[from] git2::Error),

    #[error("not a Git repository: {0}")]
    NotARepository(PathBuf),

    #[error("path does not exist: {0}")]
    PathNotFound(PathBuf),

    #[error("commit {0} was not found")]
    CommitNotFound(String),

    #[error("path is not valid UTF-8")]
    NonUtf8Path,

    #[error("reference is not valid UTF-8: {0}")]
    NonUtf8Reference(String),

    #[error("repository has no HEAD")]
    NoHead,

    #[error("operation is not supported for this repository: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
