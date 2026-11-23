//! Container-independent stream types and small binary helpers.
//!
//! DSD data moves between readers, the converter and writers as one byte vector per
//! channel, **MSB-first**: the earliest sample of each byte is bit 7, and a 1 bit means +1.
//! This is DSDIFF's native order; the DSF reader and writer reverse bits as needed.

use crate::error::{format_err, Result};
use std::io::Read;

/// Description of a DSD stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsdInfo {
    /// Number of channels (1..=6).
    pub channels: u16,
    /// Sample rate in Hz (for example 2 822 400 for DSD64).
    pub sample_rate: u32,
    /// Samples (bits) per channel.
    pub samples: u64,
}

/// Description of a PCM stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmInfo {
    /// Number of channels (1..=6).
    pub channels: u16,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Frames (samples per channel).
    pub frames: u64,
}

/// A source of DSD data.
pub trait DsdRead {
    /// Stream description.
    fn info(&self) -> DsdInfo;
    /// Appends up to about `max_bytes` bytes per channel (MSB-first) to `out[ch]`.
    /// Returns the number of bytes appended per channel; 0 means end of stream.
    /// Every channel receives the same count. The final byte may hold padding bits
    /// beyond [`DsdInfo::samples`].
    fn read_bytes(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize>;
    /// Raw metadata block (an ID3v2 tag in DSF) to carry over to the output, if any.
    fn metadata(&self) -> Option<&[u8]> {
        None
    }
    /// DSF channel-type code, if the source declared one.
    fn channel_type(&self) -> Option<u32> {
        None
    }
}

/// A sink for DSD data.
pub trait DsdWrite {
    /// Writes the same number of MSB-first bytes for every channel.
    fn write_bytes(&mut self, data: &[Vec<u8>]) -> Result<()>;
    /// Writes trailing structures and fixes up header sizes. `samples` is the exact
    /// number of samples per channel (at most 8 times the bytes written).
    fn finish(&mut self, samples: u64) -> Result<()>;
}

/// A source of PCM data, normalised so that digital full scale is +-1.0.
pub trait PcmRead {
    /// Stream description.
    fn info(&self) -> PcmInfo;
    /// Appends up to `max_frames` frames to `out[ch]`. Returns frames appended; 0 = end.
    fn read_frames(&mut self, max_frames: usize, out: &mut [Vec<f64>]) -> Result<usize>;
}

/// Reverses the bit order of every byte: LSB-first <-> MSB-first.
pub(crate) static BIT_REVERSE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};

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
        return format_err("file is truncated");
    }
    Ok(())
}

/// Checks that a DSD rate is a multiple of 44.1 kHz or 48 kHz between DSD64 and DSD1024.
pub(crate) fn check_dsd_rate(rate: u32) -> Result<()> {
    for base in [44_100u32, 48_000] {
        for mult in [64u32, 128, 256, 512, 1024] {
            if rate == base * mult {
                return Ok(());
            }
        }
    }
    format_err(format!("DSD sample rate {rate} Hz is not DSD64..DSD1024"))
}
