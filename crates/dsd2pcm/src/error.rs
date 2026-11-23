//! Error type for every fallible operation in the crate.

use std::fmt;

/// Errors returned by readers, writers and the converter. No operation panics on bad input.
///
/// New variants may be added in minor versions: match with a wildcard arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Underlying I/O failure.
    Io(std::io::Error),
    /// The input is not a valid file of the expected container format.
    Format(String),
    /// The input is valid but uses a feature this version does not support
    /// (for example DST-compressed DFF or a 48 kHz-family rate).
    Unsupported(String),
    /// A caller-supplied option is out of range.
    InvalidArgument(String),
    /// The caller's progress hook asked to stop the conversion.
    Cancelled,
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Format(m) => write!(f, "invalid file: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Error::Cancelled => write!(f, "conversion cancelled"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Format("file is truncated".into())
        } else {
            Error::Io(e)
        }
    }
}

pub(crate) fn format_err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Format(msg.into()))
}

pub(crate) fn unsupported<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Unsupported(msg.into()))
}
