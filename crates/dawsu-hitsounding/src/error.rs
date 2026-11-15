//! This crate's one error enum (ADR 0034, hitsound domain "Errors"): environment failures only.
//! Unsupported modes, save problems and cancels are outcomes, not errors. Decompile and compile
//! do no file work, so nothing in the crate returns this yet; it holds `dawsu-osu-format`'s error
//! unchanged for the functions that will read and patch through it.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// `dawsu-osu-format` failed; its error is held unchanged (ADR 0034 point 4).
    Format(dawsu_osu_format::Error),
}

impl From<dawsu_osu_format::Error> for Error {
    fn from(e: dawsu_osu_format::Error) -> Error {
        Error::Format(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Format(e) => Some(e),
        }
    }
}
