//! This crate's one error enum (ADR 0034): environment failures only.

use std::fmt;
use std::path::PathBuf;

/// A failure of the file system while writing safely.
#[derive(Debug)]
pub enum Error {
    /// A file operation failed. `step` names what was being done.
    Io { step: &'static str, path: PathBuf, source: std::io::Error },
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
        }
    }
}
