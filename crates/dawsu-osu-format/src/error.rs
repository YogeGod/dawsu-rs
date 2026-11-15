//! This crate's one error enum (ADR 0034): environment failures only. Refusals and
//! "changed on disk" are outcomes, and unreadable lines are warnings.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    /// A file operation of this crate failed.
    Io { step: &'static str, path: PathBuf, source: std::io::Error },
    /// The safe write failed; foundation's error is held unchanged (ADR 0034 point 4).
    Write(dawsu_foundation::Error),
}

impl Error {
    pub(crate) fn io(step: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io { step, path: path.into(), source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { step, path, source } => write!(f, "{step} failed for {}: {source}", path.display()),
            Error::Write(e) => write!(f, "saving failed: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Write(e) => Some(e),
        }
    }
}
