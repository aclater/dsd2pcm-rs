//! Container-independent stream types and small binary helpers.
//!
//! DSD data moves between readers, the converter and writers as one byte vector per
//! channel, **MSB-first**: the earliest sample of each byte is bit 7, and a 1 bit means +1.
//! This is DSDIFF's native order; the DSF reader and writer reverse bits as needed.

use crate::error::{format_err, FormatErrorKind, Result};
use std::io::{Read, Seek};

/// Largest number of bytes per channel one `DsdRead::read_bytes` call returns (larger
/// requests are reduced), which bounds the reader's buffer.
pub const MAX_CHUNK: usize = 16 << 20;

/// Description of a DSD stream. New fields may be added in minor versions; an
/// implementation of [`DsdRead`] makes one with [`DsdInfo::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DsdInfo {
    /// Number of channels (1..=6).
    pub channels: u16,
    /// Sample rate in Hz (for example 2 822 400 for DSD64).
    pub sample_rate: u32,
    /// Samples (bits) per channel.
    pub samples: u64,
}

impl DsdInfo {
    /// A description of `channels` channels of `samples` samples each at `sample_rate` Hz.
    pub fn new(channels: u16, sample_rate: u32, samples: u64) -> DsdInfo {
        DsdInfo {
            channels,
            sample_rate,
            samples,
        }
    }
}

/// Description of a PCM stream. New fields may be added in minor versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PcmInfo {
    /// Number of channels (1..=6).
    pub channels: u16,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Frames (samples per channel).
    pub frames: u64,
}

impl PcmInfo {
    /// A description of `channels` channels of `frames` frames at `sample_rate` Hz.
    pub fn new(channels: u16, sample_rate: u32, frames: u64) -> PcmInfo {
        PcmInfo {
            channels,
            sample_rate,
            frames,
        }
    }
}

pub(crate) mod sealed {
    /// Keeps [`super::DsdWrite`] and [`super::PcmRead`] implemented in this crate only, so
    /// methods can be added to them in minor versions.
    pub trait Sealed {}
}

/// A source of DSD data.
pub trait DsdRead {
    /// Stream description.
    fn info(&self) -> DsdInfo;
    /// Appends up to about `max_bytes` bytes per channel (MSB-first) to `out[ch]`.
    /// Returns the number of bytes appended per channel; 0 means end of stream.
    /// Every channel receives the same count. The final byte may hold padding bits
    /// beyond [`DsdInfo::samples`] (the DSF reader sets them to the idle pattern `0x69`'s
    /// bits).
    fn read_bytes(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize>;
    /// Raw metadata block (an ID3v2 tag in DSF) to carry over to the output, if any.
    fn metadata(&self) -> Option<&[u8]> {
        None
    }
    /// DSF channel-type code, if the source declared one.
    fn channel_type(&self) -> Option<u32> {
        None
    }
    /// The speakers of the channels as a `WAVE_FORMAT_EXTENSIBLE` channel mask (bit 0 front
    /// left, 1 front right, 2 front centre, 3 LFE, 4 back left, 5 back right; the channels
    /// in the order of the bits), if the source declares a layout that maps to one: a DSF
    /// channel type that matches the channel count, or DSDIFF channel IDs. By default, the
    /// mask of [`DsdRead::channel_type`] ([`crate::dsf::channel_mask_for_type`]).
    fn channel_mask(&self) -> Option<u32> {
        self.channel_type()
            .and_then(|t| crate::dsf::channel_mask_for_type(t, self.info().channels))
    }
}

/// A sink for DSD data: the DSF and DFF writers. Sealed: implemented in this crate only.
pub trait DsdWrite: sealed::Sealed {
    /// Writes the same number of MSB-first bytes for every channel.
    fn write_bytes(&mut self, data: &[Vec<u8>]) -> Result<()>;
    /// Writes trailing structures and fixes up header sizes. `samples` is the exact
    /// number of samples per channel (at most 8 times the bytes written).
    fn finish(&mut self, samples: u64) -> Result<()>;
}

/// A source of PCM data, normalised so that digital full scale is +-1.0: the WAV and FLAC
/// readers. Sealed: implemented in this crate only.
pub trait PcmRead: sealed::Sealed {
    /// Stream description.
    fn info(&self) -> PcmInfo;
    /// Appends up to `max_frames` frames to `out[ch]`. Returns frames appended; 0 = end.
    fn read_frames(&mut self, max_frames: usize, out: &mut [Vec<f64>]) -> Result<usize>;
    /// The channel mask the file declares (`WAVE_FORMAT_EXTENSIBLE`'s `dwChannelMask`,
    /// FLAC's `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment), as it is; `None` if it
    /// declares none.
    fn channel_mask(&self) -> Option<u32> {
        None
    }
}

pub(crate) fn read_exact_arr<const N: usize, R: Read>(r: &mut R) -> Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

pub(crate) fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub(crate) fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
pub(crate) fn le_u64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Skips `n` bytes of a stream by reading, so the reader need not support seeking.
pub(crate) fn skip<R: Read>(r: &mut R, n: u64) -> Result<()> {
    let got = std::io::copy(&mut r.by_ref().take(n), &mut std::io::sink())?;
    if got != n {
        return format_err(FormatErrorKind::Truncated, "file is truncated");
    }
    Ok(())
}

/// Moves forward to byte `target` by reading when it is at most this far ahead.
const SKIP_BY_READ: u64 = 64 << 10;

/// Moves to byte `target` of a chunk list without discarding a reader's buffer: a short
/// forward move reads through the bytes, anything else uses [`Seek::seek_relative`], which
/// `std::io::BufReader` serves from its buffer when the target is inside it. (A plain
/// `seek` empties the buffer, so walking many small chunks would refill a large buffer
/// once per chunk.)
pub(crate) fn seek_to<R: Read + Seek>(r: &mut R, target: u64) -> Result<()> {
    let cur = r.stream_position()?;
    if target >= cur && target - cur <= SKIP_BY_READ {
        // A short read means the end of the file; the caller's next read reports it.
        std::io::copy(&mut r.by_ref().take(target - cur), &mut std::io::sink())?;
        return Ok(());
    }
    let delta = i64::try_from(i128::from(target) - i128::from(cur))
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    r.seek_relative(delta)?;
    Ok(())
}

/// Most chunks a reader reads up to and including the audio data chunk (DFF top-level chunks
/// with `DSD `, WAV chunks with `data`), and most DFF property chunks in `PROP`.
pub(crate) const MAX_CHUNKS: usize = 4096;

/// Checks that a DSD rate is a multiple of 44.1 kHz or 48 kHz between DSD64 and DSD1024.
pub(crate) fn check_dsd_rate(rate: u32) -> Result<()> {
    for base in [44_100u32, 48_000] {
        for mult in [64u32, 128, 256, 512, 1024] {
            if rate == base * mult {
                return Ok(());
            }
        }
    }
    format_err(
        FormatErrorKind::Malformed,
        format!("DSD sample rate {rate} Hz is not DSD64..DSD1024"),
    )
}
