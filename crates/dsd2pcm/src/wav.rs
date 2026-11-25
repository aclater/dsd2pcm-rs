//! WAV (RIFF/WAVE) reader and a small writer.
//!
//! The reader accepts PCM integer samples of 16, 24 or 32 bits and IEEE float samples of 32
//! or 64 bits, in plain or `WAVE_FORMAT_EXTENSIBLE` form, 1 to 6 channels. Samples are
//! returned as `f64` with digital full scale at +-1.0; non-finite float samples read as 0.
//! A `data` chunk size of 0 or 0xFFFFFFFF (left by streaming writers and by some encoders
//! past 4 GiB) means the samples run to the end of the file, unless other chunks follow it,
//! which is an error.

use crate::error::{
    format_err, unsupported, ArgumentKind, FormatErrorKind, Result, UnsupportedKind,
};
use crate::io::{le_u16, le_u32, read_exact_arr, seek_to, PcmInfo, PcmRead, MAX_CHUNKS};
use std::io::{Read, Seek, SeekFrom, Write};

/// Largest number of frames one `read_frames` call returns.
pub const MAX_CHUNK: usize = 1 << 20;

/// Sample encoding of a WAV file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WavFormat {
    /// Signed integer, 16 bits.
    I16,
    /// Signed integer, 24 bits.
    I24,
    /// Signed integer, 32 bits.
    I32,
    /// IEEE float, 32 bits.
    F32,
    /// IEEE float, 64 bits.
    F64,
}

impl WavFormat {
    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        match self {
            WavFormat::I16 => 2,
            WavFormat::I24 => 3,
            WavFormat::I32 | WavFormat::F32 => 4,
            WavFormat::F64 => 8,
        }
    }
    fn is_float(self) -> bool {
        matches!(self, WavFormat::F32 | WavFormat::F64)
    }
}

/// Bytes 2 to 15 of every `KSDATAFORMAT_SUBTYPE_*` GUID of a `WAVE_FORMAT_*` tag
/// (`XXXXXXXX-0000-0010-8000-00AA00389B71`, as stored).
const KSDATAFORMAT_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// A GUID as stored in a file, in its usual text form.
fn guid(b: &[u8]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}",
        le_u32(b, 0),
        le_u16(b, 4),
        le_u16(b, 6),
        b[8],
        b[9],
        b[10..16]
            .iter()
            .map(|x| format!("{x:02X}"))
            .collect::<String>()
    )
}

/// WAV reader.
///
/// Rules for files that bend the format, each accepted as real tools write or read them:
/// - A `data` size of 0 or `0xFFFFFFFF` (a placeholder left by streaming writers and some
///   encoders past 4 GiB) is read to the end of the file, if no chunk follows.
/// - A `data` size larger than the rest of the file (a recording or download cut short) is
///   read to the end of the file: [`PcmInfo::frames`] counts the whole frames present, and
///   [`WavReader::declared_frames`] gives what the header declared.
/// - `WAVE_FORMAT_EXTENSIBLE`: a valid-bits count of 0 is taken as the container's width,
///   and fewer valid bits than the container (20 bits in 24, as `flac -d` writes them) read
///   at the container's width, the samples being left-justified. More valid bits than the
///   container is [`crate::FormatErrorKind::Malformed`]; a sub-format GUID other than
///   `KSDATAFORMAT`'s PCM and IEEE float is [`crate::UnsupportedKind::Encoding`].
#[derive(Debug)]
pub struct WavReader<R> {
    r: R,
    info: PcmInfo,
    format: WavFormat,
    channel_mask: Option<u32>,
    declared_frames: u64,
    remaining: u64,
    buf: Vec<u8>,
}

impl<R: Read + Seek> WavReader<R> {
    /// Parses the headers and positions the reader at the first sample.
    pub fn new(mut r: R) -> Result<Self> {
        let file_len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let hdr: [u8; 12] = read_exact_arr(&mut r)?;
        if &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
            return format_err(
                FormatErrorKind::NotRecognised,
                "not a WAV file (no RIFF/WAVE header)",
            );
        }
        let mut pos = 12u64;
        let mut fmt: Option<(WavFormat, u16, u32, Option<u32>)> = None;
        let mut chunks = 0;
        loop {
            if chunks == MAX_CHUNKS {
                return format_err(
                    FormatErrorKind::LimitExceeded,
                    "WAV file has too many chunks before 'data'",
                );
            }
            chunks += 1;
            if pos + 8 > file_len {
                return format_err(FormatErrorKind::Malformed, "WAV file has no 'data' chunk");
            }
            let ck: [u8; 8] = read_exact_arr(&mut r)?;
            let size = u64::from(le_u32(&ck, 4));
            let body = pos + 8;
            match &ck[0..4] {
                b"fmt " => {
                    if !(16..=1024).contains(&size) || body + size > file_len {
                        return format_err(
                            FormatErrorKind::Malformed,
                            "WAV 'fmt ' chunk size is invalid",
                        );
                    }
                    let mut f = vec![0u8; size as usize];
                    r.read_exact(&mut f)?;
                    let mut tag = le_u16(&f, 0);
                    let channels = le_u16(&f, 2);
                    let rate = le_u32(&f, 4);
                    let align = le_u16(&f, 12);
                    let bits = le_u16(&f, 14);
                    let mut mask = None;
                    if tag == 0xFFFE {
                        if size < 40 {
                            return format_err(
                                FormatErrorKind::Malformed,
                                "WAV extensible 'fmt ' chunk is too small",
                            );
                        }
                        // wValidBitsPerSample: the samples' bits, left-justified in the
                        // container, so they read right at the container's width. 0 is
                        // "not given" (older writers); more than the container is invalid.
                        let valid = le_u16(&f, 18);
                        if valid > bits {
                            return format_err(
                                FormatErrorKind::Malformed,
                                format!("WAV valid bits {valid} exceed the {bits}-bit container"),
                            );
                        }
                        mask = Some(le_u32(&f, 20));
                        // The sub-format GUID: a format tag in the first two bytes, then
                        // KSDATAFORMAT's fixed tail. Any other GUID is another format (for
                        // example Ambisonic B-format), not PCM or IEEE float.
                        if f[26..40] != KSDATAFORMAT_TAIL {
                            return unsupported(
                                UnsupportedKind::Encoding,
                                format!("WAV extensible sub-format GUID {}", guid(&f[24..40])),
                            );
                        }
                        tag = le_u16(&f, 24);
                    }
                    let format = match (tag, bits) {
                        (1, 16) => WavFormat::I16,
                        (1, 24) => WavFormat::I24,
                        (1, 32) => WavFormat::I32,
                        (3, 32) => WavFormat::F32,
                        (3, 64) => WavFormat::F64,
                        _ => {
                            return unsupported(
                                UnsupportedKind::Encoding,
                                format!("WAV format tag {tag} with {bits} bits"),
                            )
                        }
                    };
                    if !(1..=6).contains(&channels) {
                        return format_err(
                            FormatErrorKind::Malformed,
                            format!("WAV channel count {channels} is not 1..6"),
                        );
                    }
                    if rate == 0 {
                        return format_err(FormatErrorKind::Malformed, "WAV sample rate is 0");
                    }
                    if usize::from(align) != usize::from(channels) * format.bytes() {
                        return format_err(
                            FormatErrorKind::Malformed,
                            "WAV block alignment does not match the format",
                        );
                    }
                    fmt = Some((format, channels, rate, mask));
                }
                b"data" => {
                    let Some((format, channels, rate, channel_mask)) = fmt else {
                        return format_err(
                            FormatErrorKind::Malformed,
                            "WAV 'data' chunk before 'fmt '",
                        );
                    };
                    // Streamed files and some encoders past 4 GiB leave a placeholder
                    // size of 0 or 0xFFFFFFFF; the samples then run to the end of the file.
                    // A placeholder followed by other chunks is an error, never 0 frames.
                    let avail = file_len - body;
                    let len = if size == 0 || size == 0xFFFF_FFFF {
                        if chunks_fill(&mut r, body, file_len)? {
                            return format_err(
                                FormatErrorKind::Malformed,
                                format!(
                                    "WAV 'data' chunk size is {size:#x} and other chunks follow it"
                                ),
                            );
                        }
                        avail
                    } else {
                        size.min(avail)
                    };
                    let frame = (usize::from(channels) * format.bytes()) as u64;
                    let frames = len / frame;
                    let declared_frames = if size == 0 || size == 0xFFFF_FFFF {
                        frames
                    } else {
                        size / frame
                    };
                    return Ok(WavReader {
                        r,
                        info: PcmInfo {
                            channels,
                            sample_rate: rate,
                            frames,
                        },
                        format,
                        channel_mask,
                        declared_frames,
                        remaining: frames,
                        buf: Vec::new(),
                    });
                }
                _ => {}
            }
            pos = body + size + (size & 1);
            seek_to(&mut r, pos)?;
        }
    }

    /// Sample encoding.
    pub fn format(&self) -> WavFormat {
        self.format
    }

    /// The `dwChannelMask` of a `WAVE_FORMAT_EXTENSIBLE` file, as it is (0 means no
    /// speaker assignment); `None` for a plain `fmt ` chunk, which declares none.
    pub fn channel_mask(&self) -> Option<u32> {
        self.channel_mask
    }

    /// The frames the `data` chunk's size declares (whole frames). More than
    /// [`PcmInfo::frames`] when the file is cut short; the same otherwise, and for a
    /// placeholder size (0 or `0xFFFFFFFF`).
    pub fn declared_frames(&self) -> u64 {
        self.declared_frames
    }
}

/// Whether `from..file_len` holds one or more RIFF chunks exactly (printable ids, sizes that
/// end at the end of the file, with or without a final pad byte). Leaves the reader at `from`.
fn chunks_fill<R: Read + Seek>(r: &mut R, from: u64, file_len: u64) -> Result<bool> {
    let mut pos = from;
    let mut chunks = 0;
    let fill = loop {
        if chunks > 0 && (pos == file_len || pos == file_len + 1) {
            break true;
        }
        if chunks == MAX_CHUNKS {
            break true; // that many well-formed chunk headers are not sample data
        }
        if pos + 8 > file_len {
            break false;
        }
        seek_to(r, pos)?;
        let ck: [u8; 8] = read_exact_arr(r)?;
        if !ck[0..4].iter().all(|b| (0x20..=0x7E).contains(b)) {
            break false;
        }
        let size = u64::from(le_u32(&ck, 4));
        pos += 8 + size + (size & 1);
        chunks += 1;
    };
    seek_to(r, from)?;
    Ok(fill)
}

/// Decodes little-endian samples of `format` from `bytes` and appends them to `out` as `f64`,
/// by the same rules as [`WavReader`]: integers scaled so that digital full scale is +-1.0
/// (2^-15, 2^-23, 2^-31), floats as they are, non-finite floats as 0. The order of the
/// samples (interleaved or not) is kept. A trailing partial sample is ignored. Returns the
/// number of samples appended.
pub fn decode(format: WavFormat, bytes: &[u8], out: &mut Vec<f64>) -> usize {
    let n = bytes.len() / format.bytes();
    out.reserve(n);
    out.extend(
        bytes
            .chunks_exact(format.bytes())
            .map(|b| sample(format, b)),
    );
    n
}

fn sample(format: WavFormat, b: &[u8]) -> f64 {
    let v = match format {
        WavFormat::I16 => f64::from(i16::from_le_bytes([b[0], b[1]])) * (1.0 / 32768.0),
        WavFormat::I24 => {
            f64::from(i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) * (1.0 / 8_388_608.0)
        }
        WavFormat::I32 => {
            f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) * (1.0 / 2_147_483_648.0)
        }
        WavFormat::F32 => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        WavFormat::F64 => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
    };
    if format.is_float() && !v.is_finite() {
        0.0
    } else {
        v
    }
}

impl<R: Read + Seek> crate::io::sealed::Sealed for WavReader<R> {}

impl<R: Read + Seek> PcmRead for WavReader<R> {
    fn info(&self) -> PcmInfo {
        self.info
    }

    fn read_frames(&mut self, max_frames: usize, out: &mut [Vec<f64>]) -> Result<usize> {
        let ch = usize::from(self.info.channels);
        if out.len() != ch {
            return Err(crate::Error::invalid(
                ArgumentKind::Buffer,
                "output channel count mismatch",
            ));
        }
        let n = (max_frames.clamp(1, MAX_CHUNK) as u64).min(self.remaining) as usize;
        if n == 0 {
            return Ok(0);
        }
        let bs = self.format.bytes();
        self.buf.resize(n * ch * bs, 0);
        self.r.read_exact(&mut self.buf)?;
        for frame in self.buf.chunks_exact(ch * bs) {
            for (c, o) in out.iter_mut().enumerate() {
                o.push(sample(self.format, &frame[c * bs..(c + 1) * bs]));
            }
        }
        self.remaining -= n as u64;
        Ok(n)
    }

    fn channel_mask(&self) -> Option<u32> {
        self.channel_mask
    }
}

/// WAV writer for interleaved sample bytes already in the target encoding.
///
/// Mono and stereo 16-bit files get a 16-byte `fmt ` chunk (format tag 1). Everything else
/// (more than 16 bits, floats, more than two channels, or a channel mask other than the
/// count's) gets `WAVE_FORMAT_EXTENSIBLE`, as Microsoft's WAVE rules require, with the PCM
/// or IEEE-float subtype and the channel mask of the channel count ([`channel_mask`]) or
/// the one given to [`WavWriter::with_channel_mask`]. Samples start at
/// [`WavWriter::data_offset`].
#[derive(Debug)]
pub struct WavWriter<W: Write + Seek> {
    w: W,
    data_len: u64,
    /// Offset of the data chunk's size field.
    size_at: u64,
    finished: bool,
}

/// Bytes before the first sample in a file from [`WavWriter::new`] of `format` and
/// `channels`: 44, or 68 with `WAVE_FORMAT_EXTENSIBLE`. For [`WavWriter::with_channel_mask`]
/// it is a lower bound: a mask other than the count's ([`channel_mask`]) always gives 68;
/// [`WavWriter::data_offset`] gives the real value.
pub fn header_len(format: WavFormat, channels: u16) -> u64 {
    if channels > 2 || format.bytes() > 2 {
        68
    } else {
        44
    }
}

/// `KSDATAFORMAT_SUBTYPE_PCM` and `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` share these 14 bytes
/// after their 2-byte format code.
const SUBTYPE_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// The `WAVE_FORMAT_EXTENSIBLE` channel mask for `channels` channels in SACD order:
/// mono FC; stereo FL FR; 3 FL FR FC; 4 FL FR BL BR; 5 FL FR FC BL BR; 6 FL FR FC LFE BL BR.
pub fn channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x33,
        5 => 0x37,
        6 => 0x3F,
        _ => 0,
    }
}

impl<W: Write + Seek> WavWriter<W> {
    /// Starts a WAV file, with the channel mask of the channel count ([`channel_mask`]).
    pub fn new(w: W, format: WavFormat, channels: u16, rate: u32) -> Result<Self> {
        Self::with_channel_mask(w, format, channels, rate, channel_mask(channels))
    }

    /// Starts a WAV file whose channels are the speakers of `mask` (the
    /// `WAVE_FORMAT_EXTENSIBLE` bits, the channels in the order of the bits). `mask` must
    /// have one bit per channel, or be 0 (no speaker assignment).
    pub fn with_channel_mask(
        mut w: W,
        format: WavFormat,
        channels: u16,
        rate: u32,
        mask: u32,
    ) -> Result<Self> {
        if !(1..=6).contains(&channels) || rate == 0 {
            return Err(crate::Error::invalid(
                ArgumentKind::Channels,
                "WAV writer: 1..6 channels, rate > 0",
            ));
        }
        if mask != 0 && mask.count_ones() != u32::from(channels) {
            return Err(crate::Error::invalid(
                ArgumentKind::Channels,
                format!("WAV writer: channel mask {mask:#x} does not have {channels} speakers"),
            ));
        }
        let bs = format.bytes() as u16;
        let byte_rate = rate
            .checked_mul(u32::from(channels) * u32::from(bs))
            .ok_or_else(|| {
                crate::Error::invalid(ArgumentKind::Rate, "WAV writer: rate too high")
            })?;
        let code: u16 = if format.is_float() { 3 } else { 1 };
        let extensible = header_len(format, channels) == 68 || mask != channel_mask(channels);
        let mut h = Vec::with_capacity(68);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&(if extensible { 40u32 } else { 16 }).to_le_bytes());
        h.extend_from_slice(&(if extensible { 0xFFFEu16 } else { code }).to_le_bytes());
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&byte_rate.to_le_bytes());
        h.extend_from_slice(&(channels * bs).to_le_bytes());
        h.extend_from_slice(&(bs * 8).to_le_bytes());
        if extensible {
            h.extend_from_slice(&22u16.to_le_bytes());
            h.extend_from_slice(&(bs * 8).to_le_bytes());
            h.extend_from_slice(&mask.to_le_bytes());
            h.extend_from_slice(&code.to_le_bytes());
            h.extend_from_slice(&SUBTYPE_TAIL);
        }
        h.extend_from_slice(b"data");
        let size_at = h.len() as u64;
        h.extend_from_slice(&0u32.to_le_bytes());
        w.write_all(&h)?;
        Ok(WavWriter {
            w,
            data_len: 0,
            size_at,
            finished: false,
        })
    }

    /// Appends interleaved sample bytes. Fails, writing nothing, if the data would pass the
    /// 4 GiB RIFF limit.
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        let data_len = self.data_len + bytes.len() as u64;
        if self.size_at - 4 + data_len + (data_len & 1) > u64::from(u32::MAX) {
            return unsupported(
                UnsupportedKind::OutputSize,
                "WAV output over 4 GiB (RF64 is not supported yet)",
            );
        }
        self.w.write_all(bytes)?;
        self.data_len += bytes.len() as u64;
        Ok(())
    }

    /// Patches the chunk sizes. Fails if the data exceeds the 4 GiB RIFF limit.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        if self.data_len % 2 == 1 {
            self.w.write_all(&[0])?;
        }
        let riff = self.size_at - 4 + self.data_len + (self.data_len & 1);
        if riff > u64::from(u32::MAX) {
            return unsupported(
                UnsupportedKind::OutputSize,
                "WAV output over 4 GiB (RF64 is not supported yet)",
            );
        }
        self.w.seek(SeekFrom::Start(4))?;
        self.w.write_all(&(riff as u32).to_le_bytes())?;
        self.w.seek(SeekFrom::Start(self.size_at))?;
        self.w.write_all(&(self.data_len as u32).to_le_bytes())?;
        self.w.seek(SeekFrom::End(0))?;
        self.w.flush()?;
        self.finished = true;
        Ok(())
    }

    /// Offset of the first sample byte in the file (44, or 68 with `WAVE_FORMAT_EXTENSIBLE`).
    pub fn data_offset(&self) -> u64 {
        self.size_at + 4
    }

    /// Returns the inner writer (after [`WavWriter::finish`]).
    pub fn into_inner(self) -> W {
        self.w
    }
}
