//! FLAC reader (feature `flac`, on by default), decoding with `claxon`.
//!
//! Accepts 1 to 6 channels of 8, 12, 16, 20 or 24-bit samples at any rate FLAC can
//! declare. Samples are returned as `f64` with digital full scale at +-1.0: a `b`-bit value
//! `v` reads as `v / 2^(b-1)`, so 16 and 24-bit files give exactly what `WavReader` gives
//! for a WAV of the same samples.
//!
//! This module parses the container itself: an optional leading ID3v2 tag (skipped, as the
//! reference decoder does), the `fLaC` marker and the metadata blocks, of which only
//! STREAMINFO is kept; the others are skipped by seeking, after checking their sizes
//! against the file length, so nothing in them is allocated or parsed. `claxon` then sees
//! a minimal stream (marker and STREAMINFO) followed by the audio frames, and only decodes
//! frames. Every frame must match STREAMINFO's channel count and sample range.
//!
//! The frame count comes from STREAMINFO. When STREAMINFO gives 0 (unknown), the reader
//! decodes the stream once to count the frames and then starts again from the first
//! frame, the FLAC form of the WAV reader's read-to-end rule; a stream with no frames is
//! an error. A stream that ends before its declared count is an error; frames beyond it
//! are ignored.

use crate::error::{format_err, unsupported, Error, Result};
use crate::io::{read_exact_arr, PcmInfo, PcmRead};
use std::io::{Chain, Cursor, Read, Seek, SeekFrom};

/// Largest number of frames one `read_frames` call returns.
pub const MAX_CHUNK: usize = 1 << 20;

/// Most ID3v2 tags skipped before the `fLaC` marker.
const MAX_ID3_TAGS: usize = 16;
/// Most metadata blocks skipped after STREAMINFO.
const MAX_METADATA_BLOCKS: usize = 4096;

type Inner<R> = claxon::FlacReader<Chain<Cursor<Vec<u8>>, R>>;

/// FLAC reader.
pub struct FlacReader<R: Read + Seek> {
    flac: Inner<R>,
    info: PcmInfo,
    bits: u32,
    /// Decoded samples not yet returned: one block, channel after channel, `pend_len`
    /// frames each, from frame `pend_pos` on.
    pend: Vec<i32>,
    pend_len: usize,
    pend_pos: usize,
    remaining: u64,
}

impl<R: Read + Seek> std::fmt::Debug for FlacReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlacReader")
            .field("info", &self.info)
            .field("bits", &self.bits)
            .field("remaining", &self.remaining)
            .finish_non_exhaustive()
    }
}

fn map_err(e: claxon::Error) -> Error {
    match e {
        claxon::Error::IoError(e) => Error::from(e),
        claxon::Error::FormatError(m) => Error::Format(format!("FLAC: {m}")),
        claxon::Error::Unsupported(m) => Error::Unsupported(format!("FLAC: {m}")),
    }
}

/// Where the audio frames start, and the 34-byte STREAMINFO body.
fn parse_container<R: Read + Seek>(r: &mut R, file_len: u64) -> Result<(u64, [u8; 34])> {
    let mut pos = 0u64;
    let mut tags = 0;
    loop {
        if pos + 4 > file_len {
            return format_err("not a FLAC file (no 'fLaC' marker)");
        }
        let magic: [u8; 4] = read_exact_arr(r)?;
        if &magic == b"fLaC" {
            pos += 4;
            break;
        }
        if &magic[..3] != b"ID3" || tags == MAX_ID3_TAGS {
            return format_err("not a FLAC file (no 'fLaC' marker)");
        }
        // ID3v2 header: "ID3", version (2), flags, size (4 syncsafe bytes).
        let h: [u8; 6] = read_exact_arr(r)?;
        let sz = &h[2..6];
        if sz.iter().any(|&b| b & 0x80 != 0) {
            return format_err("ID3v2 tag size is invalid");
        }
        let size = sz.iter().fold(0u64, |a, &b| (a << 7) | u64::from(b));
        let footer = if h[1] & 0x10 != 0 { 10 } else { 0 };
        pos += 10 + size + footer;
        if pos > file_len {
            return format_err("ID3v2 tag runs past the end of the file");
        }
        r.seek(SeekFrom::Start(pos))?;
        tags += 1;
    }
    let mut streaminfo = None;
    for n in 0.. {
        if n > MAX_METADATA_BLOCKS {
            return format_err("FLAC file has too many metadata blocks");
        }
        if pos + 4 > file_len {
            return format_err("FLAC metadata is truncated");
        }
        let h: [u8; 4] = read_exact_arr(r)?;
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7F;
        let len = u64::from(u32::from_be_bytes([0, h[1], h[2], h[3]]));
        pos += 4;
        if pos + len > file_len {
            return format_err("FLAC metadata block runs past the end of the file");
        }
        match (n, kind) {
            (0, 0) => {
                if len != 34 {
                    return format_err("FLAC STREAMINFO block size is not 34");
                }
                streaminfo = Some(read_exact_arr::<34, _>(r)?);
            }
            (0, _) => return format_err("FLAC file does not start with STREAMINFO"),
            (_, 0) => return format_err("FLAC file has a second STREAMINFO block"),
            (_, 127) => return format_err("FLAC metadata block type 127 is invalid"),
            _ => {
                r.seek(SeekFrom::Start(pos + len))?;
            }
        }
        pos += len;
        if last {
            break;
        }
    }
    match streaminfo {
        Some(s) => Ok((pos, s)),
        None => format_err("FLAC file has no STREAMINFO block"),
    }
}

/// A claxon reader over a minimal stream (marker and STREAMINFO) followed by `r` from
/// its current position.
fn open_frames<R: Read + Seek>(r: R, streaminfo: &[u8; 34]) -> Result<Inner<R>> {
    let mut head = Vec::with_capacity(42);
    head.extend_from_slice(b"fLaC");
    head.extend_from_slice(&[0x80, 0, 0, 34]);
    head.extend_from_slice(streaminfo);
    let opts = claxon::FlacReaderOptions {
        metadata_only: false,
        read_vorbis_comment: false,
    };
    claxon::FlacReader::new_ext(Cursor::new(head).chain(r), opts).map_err(map_err)
}

/// Decodes the next block into `buf`; `Ok(None)` at the end of the stream. Checks the
/// block against the stream's channel count and sample range.
fn next_block<R: Read + Seek>(
    flac: &mut Inner<R>,
    channels: u16,
    bits: u32,
    buf: Vec<i32>,
) -> Result<Option<(Vec<i32>, usize)>> {
    let Some(block) = flac.blocks().read_next_or_eof(buf).map_err(map_err)? else {
        return Ok(None);
    };
    if block.channels() != u32::from(channels) {
        return format_err("FLAC frame channel count differs from STREAMINFO");
    }
    let len = block.duration() as usize;
    let buf = block.into_buffer();
    let lo = -(1i32 << (bits - 1));
    let hi = (1i32 << (bits - 1)) - 1;
    if buf.len() != len * usize::from(channels) {
        return format_err("FLAC frame size is inconsistent");
    }
    if buf.iter().any(|&v| v < lo || v > hi) {
        return format_err("FLAC sample outside the STREAMINFO bit depth");
    }
    Ok(Some((buf, len)))
}

impl<R: Read + Seek> FlacReader<R> {
    /// Parses the headers and positions the reader at the first frame. When STREAMINFO
    /// does not give the length, decodes the stream once to count the frames.
    pub fn new(mut r: R) -> Result<Self> {
        let file_len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let (frames_at, si) = parse_container(&mut r, file_len)?;
        r.seek(SeekFrom::Start(frames_at))?;
        let mut flac = open_frames(r, &si)?;
        let s = flac.streaminfo();
        if !(1..=6).contains(&s.channels) {
            return format_err(format!("FLAC channel count {} is not 1..6", s.channels));
        }
        let bits = s.bits_per_sample;
        if ![8, 12, 16, 20, 24].contains(&bits) {
            return unsupported(format!("FLAC with {bits} bits per sample"));
        }
        let channels = s.channels as u16;
        let frames = match s.samples {
            Some(n) => n,
            None => {
                let mut n = 0u64;
                let mut buf = Vec::new();
                while let Some((b, len)) = next_block(&mut flac, channels, bits, buf)? {
                    n += len as u64;
                    buf = b;
                }
                if n == 0 {
                    return format_err("FLAC stream has no audio frames");
                }
                let (_, mut r) = flac.into_inner().into_inner();
                r.seek(SeekFrom::Start(frames_at))?;
                flac = open_frames(r, &si)?;
                n
            }
        };
        Ok(FlacReader {
            flac,
            info: PcmInfo {
                channels,
                sample_rate: s.sample_rate,
                frames,
            },
            bits,
            pend: Vec::new(),
            pend_len: 0,
            pend_pos: 0,
            remaining: frames,
        })
    }

    /// Bits per sample.
    pub fn bits_per_sample(&self) -> u32 {
        self.bits
    }
}

impl<R: Read + Seek> PcmRead for FlacReader<R> {
    fn info(&self) -> PcmInfo {
        self.info
    }

    fn read_frames(&mut self, max_frames: usize, out: &mut [Vec<f64>]) -> Result<usize> {
        let ch = usize::from(self.info.channels);
        if out.len() != ch {
            return Err(Error::InvalidArgument(
                "output channel count mismatch".into(),
            ));
        }
        let want = (max_frames.clamp(1, MAX_CHUNK) as u64).min(self.remaining) as usize;
        let scale = 1.0 / f64::from(1u32 << (self.bits - 1));
        let mut done = 0;
        while done < want {
            if self.pend_pos == self.pend_len {
                let buf = std::mem::take(&mut self.pend);
                match next_block(&mut self.flac, self.info.channels, self.bits, buf)? {
                    Some((b, len)) => {
                        self.pend = b;
                        self.pend_len = len;
                        self.pend_pos = 0;
                    }
                    None => {
                        return format_err(format!(
                            "FLAC stream ended after {} of {} frames",
                            self.info.frames - self.remaining + done as u64,
                            self.info.frames
                        ))
                    }
                }
                continue;
            }
            let n = (want - done).min(self.pend_len - self.pend_pos);
            for (c, o) in out.iter_mut().enumerate() {
                let start = c * self.pend_len + self.pend_pos;
                o.extend(
                    self.pend[start..start + n]
                        .iter()
                        .map(|&v| f64::from(v) * scale),
                );
            }
            self.pend_pos += n;
            done += n;
        }
        self.remaining -= done as u64;
        Ok(done)
    }
}
