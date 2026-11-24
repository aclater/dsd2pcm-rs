//! DSF (DSD Stream File) reader and writer.
//!
//! Layout (Sony "DSF File Format Specification" v1.01): a 28-byte `DSD ` chunk, a 52-byte
//! `fmt ` chunk, a `data` chunk of per-channel blocks (4096 bytes each, channel 0's block,
//! channel 1's block, ...), then an optional ID3v2 tag pointed to by the `DSD ` chunk. All
//! integers are little-endian and sizes are 64-bit. With 1 bit per sample the earliest
//! sample is the least significant bit of each byte.

use crate::error::{
    format_err, unsupported, ArgumentKind, FormatErrorKind, Result, UnsupportedKind,
};
use crate::io::{
    check_dsd_rate, le_u32, le_u64, read_exact_arr, seek_to, skip, DsdInfo, DsdRead, DsdWrite,
};
use std::io::{Read, Seek, SeekFrom, Write};

/// Block size per channel that this writer uses (the only value the specification allows).
pub const BLOCK_SIZE: usize = 4096;
/// Largest metadata block carried over from the input (larger tags are dropped).
pub const MAX_METADATA: u64 = 16 << 20;
/// DSD idle (silence) pattern, MSB-first.
const IDLE: u8 = 0x69;
/// Largest block size accepted from a reader.
const MAX_BLOCK: u32 = 1 << 20;

/// DSF reader over any seekable byte source.
///
/// When the sample count is not a multiple of 8, the bits of the last byte past
/// [`DsdInfo::samples`] are padding; the reader delivers them as the bits of the idle
/// pattern `0x69` at those positions, whatever the file holds, so the padding converts
/// as silence.
#[derive(Debug)]
pub struct DsfReader<R> {
    r: R,
    info: DsdInfo,
    block: usize,
    lsb_first: bool,
    channel_type: u32,
    /// Bytes per channel still to deliver.
    remaining: u64,
    metadata: Option<Vec<u8>>,
    buf: Vec<u8>,
}

impl<R: Read + Seek> DsfReader<R> {
    /// Parses the headers and positions the reader at the first audio block.
    pub fn new(mut r: R) -> Result<Self> {
        let file_len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let hdr: [u8; 28] = read_exact_arr(&mut r)?;
        if &hdr[0..4] != b"DSD " {
            return format_err(
                FormatErrorKind::NotRecognised,
                "not a DSF file (no 'DSD ' chunk)",
            );
        }
        if le_u64(&hdr, 4) != 28 {
            return format_err(
                FormatErrorKind::Malformed,
                "DSF 'DSD ' chunk size is not 28",
            );
        }
        let meta_ptr = le_u64(&hdr, 20);

        let fmt: [u8; 12] = read_exact_arr(&mut r)?;
        if &fmt[0..4] != b"fmt " {
            return format_err(FormatErrorKind::Malformed, "DSF 'fmt ' chunk missing");
        }
        let fmt_size = le_u64(&fmt, 4);
        if !(52..=4096).contains(&fmt_size) {
            return format_err(
                FormatErrorKind::Malformed,
                format!("DSF 'fmt ' chunk size {fmt_size} is invalid"),
            );
        }
        let body: [u8; 40] = read_exact_arr(&mut r)?;
        skip(&mut r, fmt_size - 52)?;
        let version = le_u32(&body, 0);
        let format_id = le_u32(&body, 4);
        let channel_type = le_u32(&body, 8);
        let channels = le_u32(&body, 12);
        let rate = le_u32(&body, 16);
        let bits = le_u32(&body, 20);
        let samples = le_u64(&body, 24);
        let block = le_u32(&body, 32);
        if version != 1 {
            return unsupported(
                UnsupportedKind::Version,
                format!("DSF format version {version}"),
            );
        }
        if format_id != 0 {
            return unsupported(
                UnsupportedKind::Version,
                format!("DSF format id {format_id} (only raw DSD is defined)"),
            );
        }
        if !(1..=6).contains(&channels) {
            return format_err(
                FormatErrorKind::Malformed,
                format!("DSF channel count {channels} is not 1..6"),
            );
        }
        check_dsd_rate(rate)?;
        let lsb_first = match bits {
            1 => true,
            8 => false,
            _ => {
                return format_err(
                    FormatErrorKind::Malformed,
                    format!("DSF bits per sample {bits} is not 1 or 8"),
                )
            }
        };
        if block == 0 || block > MAX_BLOCK {
            return format_err(
                FormatErrorKind::Malformed,
                format!("DSF block size {block} is invalid"),
            );
        }

        let data: [u8; 12] = read_exact_arr(&mut r)?;
        if &data[0..4] != b"data" {
            return format_err(FormatErrorKind::Malformed, "DSF 'data' chunk missing");
        }
        let data_size = le_u64(&data, 4);
        if data_size < 12 {
            return format_err(
                FormatErrorKind::Malformed,
                "DSF 'data' chunk size is too small",
            );
        }
        let data_len = data_size - 12;
        let data_start = r.stream_position()?;
        if data_start
            .checked_add(data_len)
            .is_none_or(|end| end > file_len)
        {
            return format_err(
                FormatErrorKind::Truncated,
                "DSF 'data' chunk runs past the end of the file",
            );
        }
        let bytes_per_ch = samples.div_ceil(8);
        let blocks = bytes_per_ch.div_ceil(u64::from(block));
        let needed = blocks
            .checked_mul(u64::from(block))
            .and_then(|v| v.checked_mul(u64::from(channels)));
        match needed {
            Some(n) if n <= data_len => {}
            _ => {
                return format_err(
                    FormatErrorKind::Truncated,
                    "DSF sample count exceeds the audio data present",
                )
            }
        }

        let metadata = if meta_ptr != 0 && meta_ptr >= data_start + data_len && meta_ptr < file_len
        {
            let len = file_len - meta_ptr;
            if len <= MAX_METADATA {
                seek_to(&mut r, meta_ptr)?;
                let mut m = vec![0u8; len as usize];
                r.read_exact(&mut m)?;
                seek_to(&mut r, data_start)?;
                Some(m)
            } else {
                None
            }
        } else {
            None
        };

        Ok(DsfReader {
            r,
            info: DsdInfo {
                channels: channels as u16,
                sample_rate: rate,
                samples,
            },
            block: block as usize,
            lsb_first,
            channel_type,
            remaining: bytes_per_ch,
            metadata,
            buf: Vec::new(),
        })
    }
}

impl<R: Read + Seek> DsdRead for DsfReader<R> {
    fn info(&self) -> DsdInfo {
        self.info
    }

    fn read_bytes(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let ch = usize::from(self.info.channels);
        if out.len() != ch {
            return Err(crate::Error::invalid(
                ArgumentKind::Buffer,
                "output channel count mismatch",
            ));
        }
        let groups = (max_bytes.min(crate::io::MAX_CHUNK) / self.block).max(1);
        let groups = groups.min(self.remaining.div_ceil(self.block as u64) as usize);
        let total = groups * self.block * ch;
        self.buf.resize(total, 0);
        self.r.read_exact(&mut self.buf)?;
        let deliver = ((groups * self.block) as u64).min(self.remaining) as usize;
        for (c, o) in out.iter_mut().enumerate() {
            o.reserve(deliver);
            let mut left = deliver;
            for g in 0..groups {
                if left == 0 {
                    break;
                }
                let start = (g * ch + c) * self.block;
                let n = left.min(self.block);
                let src = &self.buf[start..start + n];
                if self.lsb_first {
                    o.extend(src.iter().map(|b| b.reverse_bits()));
                } else {
                    o.extend_from_slice(src);
                }
                left -= n;
            }
        }
        self.remaining -= deliver as u64;
        if self.remaining == 0 {
            // Bits past `samples` in the last byte are padding, often zeros, which decode
            // as a full-scale step. Set them to the idle pattern 0x69's bits instead.
            let pad = (self.info.samples.div_ceil(8) * 8 - self.info.samples) as u32;
            if pad > 0 {
                let mask = ((1u16 << pad) - 1) as u8;
                for o in out.iter_mut() {
                    if let Some(last) = o.last_mut() {
                        *last = (*last & !mask) | (IDLE & mask);
                    }
                }
            }
        }
        Ok(deliver)
    }

    fn metadata(&self) -> Option<&[u8]> {
        self.metadata.as_deref()
    }

    fn channel_type(&self) -> Option<u32> {
        Some(self.channel_type)
    }
}

/// The channel masks of DSF channel types 1 to 7 (DSF specification 1.01) in the
/// `WAVE_FORMAT_EXTENSIBLE` bits (FL 0x1, FR 0x2, FC 0x4, LFE 0x8, BL 0x10, BR 0x20). Each
/// type's channel order is the order of its bits.
const TYPE_MASKS: [u32; 7] = [
    0x04, // 1: mono, C
    0x03, // 2: stereo, FL FR
    0x07, // 3: 3 channels, FL FR C
    0x33, // 4: quad, FL FR BL BR
    0x0F, // 5: 4 channels, FL FR C LFE
    0x37, // 6: 5 channels, FL FR C BL BR
    0x3F, // 7: 5.1, FL FR C LFE BL BR
];

/// The channel mask of DSF channel type `channel_type` (1 to 7) for a file of `channels`
/// channels, or `None` if the type is unknown or has another number of channels.
pub fn channel_mask_for_type(channel_type: u32, channels: u16) -> Option<u32> {
    let i = usize::try_from(channel_type.checked_sub(1)?).ok()?;
    let mask = *TYPE_MASKS.get(i)?;
    (mask.count_ones() == u32::from(channels)).then_some(mask)
}

/// The DSF channel type of a channel mask for `channels` channels: the type with exactly
/// that mask, or [`default_channel_type`] of the count when no type has it.
pub fn channel_type_for_mask(mask: u32, channels: u16) -> u32 {
    match TYPE_MASKS.iter().position(|&m| m == mask) {
        Some(i) if mask.count_ones() == u32::from(channels) => i as u32 + 1,
        _ => default_channel_type(channels),
    }
}

/// DSF channel-type code for a channel count, as the specification lists them.
pub fn default_channel_type(channels: u16) -> u32 {
    match channels {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        _ => 7,
    }
}

/// DSF writer. Writes placeholder headers, then patches sizes in [`DsdWrite::finish`].
#[derive(Debug)]
pub struct DsfWriter<W: Write + Seek> {
    w: W,
    channels: usize,
    rate: u32,
    channel_type: u32,
    pending: Vec<Vec<u8>>,
    blocks: u64,
    metadata: Option<Vec<u8>>,
    finished: bool,
}

impl<W: Write + Seek> DsfWriter<W> {
    /// Starts a DSF file with `channels` channels (1..=6) at `rate` Hz.
    pub fn new(mut w: W, channels: u16, rate: u32, channel_type: Option<u32>) -> Result<Self> {
        if !(1..=6).contains(&channels) {
            return Err(crate::Error::invalid(
                ArgumentKind::Channels,
                format!("{channels} channels: 1 to 6 are supported"),
            ));
        }
        check_dsd_rate(rate)?;
        let ct = match channel_type {
            Some(t @ 1..=7) => t,
            _ => default_channel_type(channels),
        };
        w.write_all(&[0u8; 28 + 52 + 12])?;
        Ok(DsfWriter {
            w,
            channels: usize::from(channels),
            rate,
            channel_type: ct,
            pending: vec![Vec::new(); usize::from(channels)],
            blocks: 0,
            metadata: None,
            finished: false,
        })
    }

    /// Sets the channel type from a `WAVE_FORMAT_EXTENSIBLE` channel mask
    /// ([`channel_type_for_mask`]): the DSF type with that mask, or the type of the channel
    /// count when none has it (or it does not match the count). The channels must already
    /// be in the type's order, which is the order of the mask's bits.
    pub fn set_channel_mask(&mut self, mask: u32) {
        self.channel_type = channel_type_for_mask(mask, self.channels as u16);
    }

    /// Sets an ID3v2 tag to append after the audio data.
    pub fn set_metadata(&mut self, tag: Option<Vec<u8>>) {
        self.metadata = tag;
    }

    /// Returns the inner writer (after [`DsdWrite::finish`]).
    pub fn into_inner(self) -> W {
        self.w
    }

    fn flush_blocks(&mut self, pad: bool) -> Result<()> {
        // Whole blocks from `at` on; the written bytes are dropped once at the end, so that
        // writing a long buffer stays linear.
        let mut at = 0;
        let mut out = Vec::with_capacity(BLOCK_SIZE * self.channels);
        let result = loop {
            let have = self.pending[0].len() - at;
            if have == 0 || (have < BLOCK_SIZE && !pad) {
                break Ok(());
            }
            out.clear();
            for p in &self.pending {
                let n = (p.len() - at).min(BLOCK_SIZE);
                out.extend(p[at..at + n].iter().map(|b| b.reverse_bits()));
                out.resize(out.len() + (BLOCK_SIZE - n), 0);
            }
            if let Err(e) = self.w.write_all(&out) {
                break Err(e.into());
            }
            at += have.min(BLOCK_SIZE);
            self.blocks += 1;
        };
        for p in &mut self.pending {
            p.drain(..at);
        }
        result
    }
}

impl<W: Write + Seek> crate::io::sealed::Sealed for DsfWriter<W> {}

impl<W: Write + Seek> DsdWrite for DsfWriter<W> {
    fn write_bytes(&mut self, data: &[Vec<u8>]) -> Result<()> {
        if data.len() != self.channels || data.iter().any(|d| d.len() != data[0].len()) {
            return Err(crate::Error::invalid(
                ArgumentKind::Buffer,
                "unequal channel data",
            ));
        }
        for (p, d) in self.pending.iter_mut().zip(data) {
            p.extend_from_slice(d);
        }
        self.flush_blocks(false)
    }

    /// Pads the last block with zero bytes and patches the headers. When `samples` is not a
    /// multiple of 8, the bits of the last byte past `samples` are written as the caller
    /// gave them; set them to the idle pattern `0x69`'s bits at those positions, as
    /// [`DsfReader`] delivers them.
    fn finish(&mut self, samples: u64) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.flush_blocks(true)?;
        if samples.div_ceil(8) > self.blocks * BLOCK_SIZE as u64 {
            return Err(crate::Error::invalid(
                ArgumentKind::Buffer,
                "sample count exceeds data written",
            ));
        }
        let data_len = self.blocks * (BLOCK_SIZE * self.channels) as u64;
        let data_end = 92 + data_len;
        let (meta_ptr, meta_len) = match &self.metadata {
            Some(m) if !m.is_empty() => {
                self.w.write_all(m)?;
                (data_end, m.len() as u64)
            }
            _ => (0, 0),
        };
        let mut h = Vec::with_capacity(92);
        h.extend_from_slice(b"DSD ");
        h.extend_from_slice(&28u64.to_le_bytes());
        h.extend_from_slice(&(data_end + meta_len).to_le_bytes());
        h.extend_from_slice(&meta_ptr.to_le_bytes());
        h.extend_from_slice(b"fmt ");
        h.extend_from_slice(&52u64.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(&self.channel_type.to_le_bytes());
        h.extend_from_slice(&(self.channels as u32).to_le_bytes());
        h.extend_from_slice(&self.rate.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&samples.to_le_bytes());
        h.extend_from_slice(&(BLOCK_SIZE as u32).to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&(12 + data_len).to_le_bytes());
        self.w.seek(SeekFrom::Start(0))?;
        self.w.write_all(&h)?;
        self.w.seek(SeekFrom::End(0))?;
        self.w.flush()?;
        self.finished = true;
        Ok(())
    }
}
