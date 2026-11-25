//! Error type for every fallible operation in the crate.
//!
//! [`Error`] says what went wrong in four classes (the file, an unsupported feature, the
//! caller's arguments, I/O) and each class carries a kind to match on and a readable
//! message: [`FormatError`], [`UnsupportedError`], [`ArgumentError`].

use std::borrow::Cow;
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
    Format(FormatError),
    /// The input is valid but uses a feature this version does not support
    /// (for example DST-compressed DFF or a 48 kHz-family rate).
    Unsupported(UnsupportedError),
    /// A caller-supplied option is out of range.
    InvalidArgument(ArgumentError),
    /// The caller's progress hook asked to stop the conversion.
    Cancelled,
}

/// What is wrong with an invalid file ([`Error::Format`]).
///
/// New kinds may be added in minor versions: match with a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatErrorKind {
    /// The file is not of the container format it was read as (its signature or form
    /// type differs), or of none this crate reads.
    NotRecognised,
    /// The file ends before the data its headers declare: a chunk, block or the audio runs
    /// past the end.
    Truncated,
    /// A header field is out of range or contradicts another, or the audio data is invalid.
    Malformed,
    /// The file has more chunks, metadata blocks or bytes in a chunk than the reader
    /// accepts (the limits keep its memory and time bounded).
    LimitExceeded,
}

/// What an unsupported input or setting uses ([`Error::Unsupported`]).
///
/// New kinds may be added in minor versions: match with a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedKind {
    /// A kind of input this operation does not take: PCM where DSD is needed, or FLAC in
    /// a build without the `flac` feature.
    Input,
    /// Compressed audio (DST in DSDIFF, or another compression type).
    Compression,
    /// A container version or format id this version does not read.
    Version,
    /// A sample encoding this version does not read (WAV format tag and bit depth, FLAC
    /// bit depth or stream feature).
    Encoding,
    /// A DSD rate the converter does not take (only DSD64 so far).
    SampleRate,
    /// A channel count or layout the operation does not take.
    Channels,
    /// An output larger than its container holds (WAV over 4 GiB).
    OutputSize,
}

/// Which caller-supplied argument is wrong ([`Error::InvalidArgument`]).
///
/// New kinds may be added in minor versions: match with a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArgumentKind {
    /// A gain outside -120 to +24 dB.
    Gain,
    /// A channel count outside 1 to 6, or a channel mask with another speaker count.
    Channels,
    /// A sample rate a writer cannot store.
    Rate,
    /// A rate, cutoff and filter combination that does not exist (for example the wide
    /// cutoff at 88.2 kHz).
    Filter,
    /// A buffer, slice or count that does not match the stream: an output buffer of the
    /// wrong length, per-channel data of unequal lengths, a sample count past the data.
    Buffer,
    /// An output path that cannot be written: a directory, a device, the input itself.
    Output,
}

macro_rules! detail {
    ($(#[$doc:meta])* $name:ident, $kind:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name {
            kind: $kind,
            message: Cow<'static, str>,
        }

        impl $name {
            pub(crate) fn new(kind: $kind, message: impl Into<Cow<'static, str>>) -> $name {
                $name {
                    kind,
                    message: message.into(),
                }
            }

            /// The kind, to match on.
            pub fn kind(&self) -> $kind {
                self.kind
            }

            /// The message, for people: what is wrong, with the values involved.
            pub fn message(&self) -> &str {
                &self.message
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.message)
            }
        }
    };
}

detail!(
    /// The details of [`Error::Format`]: a [`FormatErrorKind`] and a message.
    FormatError,
    FormatErrorKind
);
detail!(
    /// The details of [`Error::Unsupported`]: an [`UnsupportedKind`] and a message.
    UnsupportedError,
    UnsupportedKind
);
detail!(
    /// The details of [`Error::InvalidArgument`]: an [`ArgumentKind`] and a message.
    ArgumentError,
    ArgumentKind
);

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An [`Error::Format`].
    pub(crate) fn format(kind: FormatErrorKind, msg: impl Into<Cow<'static, str>>) -> Error {
        Error::Format(FormatError::new(kind, msg))
    }

    /// An [`Error::Unsupported`].
    pub(crate) fn unsupported(kind: UnsupportedKind, msg: impl Into<Cow<'static, str>>) -> Error {
        Error::Unsupported(UnsupportedError::new(kind, msg))
    }

    /// An [`Error::InvalidArgument`].
    pub(crate) fn invalid(kind: ArgumentKind, msg: impl Into<Cow<'static, str>>) -> Error {
        Error::InvalidArgument(ArgumentError::new(kind, msg))
    }
}

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
            Error::format(FormatErrorKind::Truncated, "file is truncated")
        } else {
            Error::Io(e)
        }
    }
}

pub(crate) fn format_err<T>(kind: FormatErrorKind, msg: impl Into<Cow<'static, str>>) -> Result<T> {
    Err(Error::format(kind, msg))
}

pub(crate) fn unsupported<T>(
    kind: UnsupportedKind,
    msg: impl Into<Cow<'static, str>>,
) -> Result<T> {
    Err(Error::unsupported(kind, msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_kinds() {
        let e = Error::format(FormatErrorKind::Truncated, "file is truncated");
        assert_eq!(e.to_string(), "invalid file: file is truncated");
        let Error::Format(d) = &e else { panic!() };
        assert_eq!(d.kind(), FormatErrorKind::Truncated);
        assert_eq!(d.message(), "file is truncated");
        let e = Error::unsupported(UnsupportedKind::Compression, format!("DST in {}", "DFF"));
        assert_eq!(e.to_string(), "unsupported: DST in DFF");
        let e = Error::invalid(ArgumentKind::Gain, "gain 30 dB");
        assert_eq!(e.to_string(), "invalid argument: gain 30 dB");
        let e: Error = std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into();
        assert!(matches!(e, Error::Format(d) if d.kind() == FormatErrorKind::Truncated));
    }
}
