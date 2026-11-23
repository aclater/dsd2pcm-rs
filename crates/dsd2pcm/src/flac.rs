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
//! against the file length, so nothing in them is allocated. Only the Vorbis comment block
//! is looked into, for a `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` field: its length fields are
//! checked against the block, comments longer than 64 bytes are skipped unread, and a
//! malformed block is ignored as before. `claxon` then sees
//! a minimal stream (marker and STREAMINFO) followed by the audio frames, and only decodes
//! frames. Every frame must match STREAMINFO's channel count and sample range.
//!
//! The frame count comes from STREAMINFO. When STREAMINFO gives 0 (unknown), the reader
//! decodes the stream once to count the frames and then starts again from the first
//! frame, the FLAC form of the WAV reader's read-to-end rule; a stream with no frames is
//! an error; if that pass fails and the file ends in an ID3v1 tag (`TAG` and 125 bytes),
//! it is counted again without those 128 bytes. A stream that ends before its declared
//! count is an error; frames beyond it are ignored.
//!
//! Every read from the caller's reader goes through a private adapter, which never reports more
//! bytes than the buffer holds: claxon's byte reader indexes its buffer unchecked with the
//! count `read` returns, so a `Read` implementation that over-reports must not reach it.

use crate::error::{format_err, unsupported, Error, Result};
use crate::io::{read_exact_arr, seek_to, PcmInfo, PcmRead};
use std::io::{Chain, Cursor, Read, Seek, SeekFrom, Take};

/// Largest number of frames one `read_frames` call returns.
pub const MAX_CHUNK: usize = 1 << 20;

/// Most ID3v2 tags skipped before the `fLaC` marker.
const MAX_ID3_TAGS: usize = 16;
/// Most metadata blocks, STREAMINFO included.
const MAX_METADATA_BLOCKS: usize = 4096;
/// The Vorbis comment field that carries a channel mask, as the reference encoder writes
/// it from a `WAVE_FORMAT_EXTENSIBLE` input (matched without regard to case).
const MASK_FIELD: &[u8] = b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=";
/// Longest Vorbis comment read when looking for the mask; longer ones are skipped.
const MAX_MASK_COMMENT: usize = 64;

/// A reader whose `read` never returns more than the buffer length, whatever the inner
/// reader claims.
struct Honest<R>(R);

impl<R: Read> Read for Honest<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.0.read(buf)?;
        Ok(n.min(buf.len()))
    }
}

impl<R: Seek> Seek for Honest<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
    // Forwarded so that a buffered inner reader keeps its buffer.
    fn stream_position(&mut self) -> std::io::Result<u64> {
        self.0.stream_position()
    }
    fn seek_relative(&mut self, offset: i64) -> std::io::Result<()> {
        self.0.seek_relative(offset)
    }
}

/// claxon's input: a minimal header, then the audio frames of the file up to `end`.
type Inner<R> = claxon::FlacReader<Honest<Chain<Cursor<Vec<u8>>, Take<Honest<R>>>>>;

/// FLAC reader.
pub struct FlacReader<R: Read + Seek> {
    flac: Inner<R>,
    info: PcmInfo,
    bits: u32,
    channel_mask: Option<u32>,
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

/// A little-endian `u32` at `*pos` if it ends by `end`, advancing `*pos`.
fn le_u32_until<R: Read>(r: &mut R, pos: &mut u64, end: u64) -> Result<Option<u32>> {
    if *pos + 4 > end {
        return Ok(None);
    }
    let b: [u8; 4] = read_exact_arr(r)?;
    *pos += 4;
    Ok(Some(u32::from_le_bytes(b)))
}

/// The value of a `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` field: hexadecimal after `0x`, as the
/// reference encoder writes it, or decimal.
fn parse_mask(v: &[u8]) -> Option<u32> {
    let v = std::str::from_utf8(v).ok()?.trim();
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

/// The channel mask in the Vorbis comment block at `start..start + len` (the reader is at
/// `start`), or `None` if it has none or is malformed. Reads at most the length fields and
/// comments of up to [`MAX_MASK_COMMENT`] bytes; every length is checked against the block.
fn vorbis_channel_mask<R: Read + Seek>(r: &mut R, start: u64, len: u64) -> Result<Option<u32>> {
    let end = start + len;
    let mut pos = start;
    let Some(vendor) = le_u32_until(r, &mut pos, end)? else {
        return Ok(None);
    };
    if pos + u64::from(vendor) > end {
        return Ok(None);
    }
    pos += u64::from(vendor);
    seek_to(r, pos)?;
    let Some(count) = le_u32_until(r, &mut pos, end)? else {
        return Ok(None);
    };
    // Each comment takes at least its 4-byte length, so the loop ends within the block.
    for _ in 0..count {
        let Some(n) = le_u32_until(r, &mut pos, end)? else {
            return Ok(None);
        };
        let n = u64::from(n);
        if pos + n > end {
            return Ok(None);
        }
        if (MASK_FIELD.len() as u64..=MAX_MASK_COMMENT as u64).contains(&n) {
            let mut buf = [0u8; MAX_MASK_COMMENT];
            let c = &mut buf[..n as usize];
            r.read_exact(c)?;
            let (name, value) = c.split_at(MASK_FIELD.len());
            if name.eq_ignore_ascii_case(MASK_FIELD) {
                return Ok(parse_mask(value));
            }
        } else {
            seek_to(r, pos + n)?;
        }
        pos += n;
    }
    Ok(None)
}

/// Where the audio frames start, the 34-byte STREAMINFO body, and the channel mask of the
/// first Vorbis comment block.
fn parse_container<R: Read + Seek>(
    r: &mut R,
    file_len: u64,
) -> Result<(u64, [u8; 34], Option<u32>)> {
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
        seek_to(r, pos)?;
        tags += 1;
    }
    let mut streaminfo = None;
    let mut mask = None;
    let mut comments_seen = false;
    for n in 0.. {
        if n == MAX_METADATA_BLOCKS {
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
            // Only the first Vorbis comment block (FLAC allows one).
            (_, 4) if !comments_seen => {
                comments_seen = true;
                mask = vorbis_channel_mask(r, pos, len)?;
                seek_to(r, pos + len)?;
            }
            _ => {
                seek_to(r, pos + len)?;
            }
        }
        pos += len;
        if last {
            break;
        }
    }
    match streaminfo {
        Some(s) => Ok((pos, s, mask)),
        None => format_err("FLAC file has no STREAMINFO block"),
    }
}

/// A claxon reader over a minimal stream (marker and STREAMINFO) followed by bytes
/// `at..end` of `r`.
fn open_frames<R: Read + Seek>(
    mut r: Honest<R>,
    streaminfo: &[u8; 34],
    at: u64,
    end: u64,
) -> Result<Inner<R>> {
    r.seek(SeekFrom::Start(at))?;
    let frames = r.take(end.saturating_sub(at));
    let mut head = Vec::with_capacity(42);
    head.extend_from_slice(b"fLaC");
    head.extend_from_slice(&[0x80, 0, 0, 34]);
    head.extend_from_slice(streaminfo);
    let opts = claxon::FlacReaderOptions {
        metadata_only: false,
        read_vorbis_comment: false,
    };
    claxon::FlacReader::new_ext(Honest(Cursor::new(head).chain(frames)), opts).map_err(map_err)
}

/// The caller's reader back from a claxon reader.
fn release<R: Read + Seek>(flac: Inner<R>) -> Honest<R> {
    flac.into_inner().0.into_inner().1.into_inner()
}

/// Frames from the current position to the end of the stream.
fn count<R: Read + Seek>(flac: &mut Inner<R>, channels: u16, bits: u32) -> Result<u64> {
    let mut n = 0u64;
    let mut buf = Vec::new();
    while let Some((b, len)) = next_block(flac, channels, bits, buf)? {
        n += len as u64;
        buf = b;
    }
    Ok(n)
}

/// Whether the file ends in an ID3v1 tag (`TAG` and 125 bytes) after `start`.
fn has_id3v1<R: Read + Seek>(r: &mut Honest<R>, start: u64, file_len: u64) -> Result<bool> {
    if file_len < start + 128 {
        return Ok(false);
    }
    r.seek(SeekFrom::Start(file_len - 128))?;
    let tag: [u8; 3] = read_exact_arr(r)?;
    Ok(&tag == b"TAG")
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
    pub fn new(r: R) -> Result<Self> {
        let mut r = Honest(r);
        let file_len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let (frames_at, si, channel_mask) = parse_container(&mut r, file_len)?;
        let mut flac = open_frames(r, &si, frames_at, file_len)?;
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
                let counted = count(&mut flac, channels, bits);
                let mut r = release(flac);
                let (n, end) = match counted {
                    Ok(n) => (n, file_len),
                    Err(e) => {
                        if !has_id3v1(&mut r, frames_at, file_len)? {
                            return Err(e);
                        }
                        let end = file_len - 128;
                        let mut f = open_frames(r, &si, frames_at, end)?;
                        let n = count(&mut f, channels, bits)?;
                        r = release(f);
                        (n, end)
                    }
                };
                if n == 0 {
                    return format_err("FLAC stream has no audio frames");
                }
                flac = open_frames(r, &si, frames_at, end)?;
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
            channel_mask,
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

    /// The channel mask of the `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` Vorbis comment (as the
    /// reference encoder writes it for a `WAVE_FORMAT_EXTENSIBLE` input: `0x` and
    /// hexadecimal digits; decimal is accepted too), as it is; `None` without one, or if
    /// the comment block or the value is malformed.
    pub fn channel_mask(&self) -> Option<u32> {
        self.channel_mask
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

    fn channel_mask(&self) -> Option<u32> {
        self.channel_mask
    }
}
