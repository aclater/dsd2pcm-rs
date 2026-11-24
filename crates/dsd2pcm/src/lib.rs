//! DSD to PCM conversion with DSF, DFF and WAV reading and writing, and FLAC reading.
//!
//! - [`dsf`], [`dff`], [`wav`]: container readers and writers. Parsers are bounds-checked
//!   and return [`Error`] on bad input; nothing panics.
//! - `flac` (feature `flac`, default): FLAC reader.
//! - [`decimator`]: streaming DSD64 to 24-bit PCM at 88.2, 176.4 or 352.8 kHz, with the
//!   filters of scarletbook-pcm 0.7.0 and identical output samples.
//! - [`Filter`]: the filter choice: scarletbook-pcm's (default), the precise filter
//!   ([`Filter::Precise`], 150 dB stopbands, exact like the default) or pysacd's
//!   ([`Filter::Pysacd`], 88.2 and 176.4 kHz, dithered).
//! - [`convert`]: whole-file conversion.
//! - [`frames`]: Super Audio CD frame runs (with neighbour frames) to PCM, with the
//!   interface of scarletbook-pcm 0.7.0's `Converter`, single-threaded per call.
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
    Progress, ProgressHook, Report,
};
pub use decimator::{Cutoff, PcmRate};
pub use error::{Error, Result};
pub use filters::Filter;
