//! DSD to PCM conversion with DSF, DFF and WAV reading and writing, and FLAC reading.
//!
//! - [`dsf`], [`dff`], [`wav`]: container readers and writers. Parsers are bounds-checked
//!   and return [`Error`] on bad input; nothing panics.
//! - `flac` (feature `flac`, default): FLAC reader.
//! - [`decimator`]: streaming DSD64 to 24-bit PCM at 88.2, 176.4 or 352.8 kHz; its public
//!   [`decimator::Decimator`] has the filters of scarletbook-pcm 0.7.0 ([`Filter::Fast`])
//!   and identical output samples.
//! - [`Filter`]: the filter choice: the precise filter ([`Filter::Precise`], the default,
//!   150 dB stopbands, exact integer arithmetic), scarletbook-pcm's ([`Filter::Fast`], lower
//!   CPU, lower quality, byte-identical to scarletbook-pcm 0.7.0) or pysacd's
//!   ([`Filter::Pysacd`], 88.2 and 176.4 kHz, dithered).
//! - [`convert`]: whole-file conversion.
//! - [`frames`]: Super Audio CD frame runs (with neighbour frames) to PCM, with the
//!   interface of scarletbook-pcm 0.7.0's `Converter`, single-threaded per call.
//!
//! Output is deterministic within a release: the same version, input and settings give the
//! same bytes on every CPU, SIMD level and thread count. It may change between releases
//! when measured quality improves, each change listed with its measured effect in the
//! changelog; [`Filter::Fast`] keeps scarletbook-pcm 0.7.0's bytes.
//!
//! DSD data passes between modules as one byte vector per channel, earliest sample in the
//! most significant bit, 1 = +1 (see [`io`]).
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod convert;
pub mod decimator;
pub mod dff;
pub mod dsf;
pub mod error;
pub mod filters;
#[cfg(feature = "flac")]
pub mod flac;
pub mod frames;
pub mod io;
pub mod wav;

pub use convert::{
    convert_file, convert_file_to, convert_file_with, open_input, Input, OutputOptions, PcmOptions,
    Progress, ProgressHook, Report, Threads,
};
pub use decimator::{Cutoff, PcmRate};
pub use error::{
    ArgumentError, ArgumentKind, Error, FormatError, FormatErrorKind, Result, UnsupportedError,
    UnsupportedKind,
};
pub use filters::Filter;
