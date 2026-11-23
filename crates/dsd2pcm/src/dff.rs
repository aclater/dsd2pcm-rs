//! DSDIFF (DFF) reader and writer for uncompressed DSD.
//!
//! Layout (Philips "DSDIFF File Format Specification" 1.5): a `FRM8` form of type `DSD `
//! holding `FVER`, a `PROP` chunk of type `SND ` (with `FS  `, `CHNL`, `CMPR` and optional
//! sub-chunks), then a `DSD ` chunk with the samples interleaved byte by byte, the earliest
//! sample in the most significant bit. Integers are big-endian, chunk sizes 64-bit, and
//! chunks are padded to an even length. DST-compressed files are rejected.

use crate::error::{format_err, unsupported, Result};
use crate::io::{
    check_dsd_rate, read_exact_arr, seek_to, skip, DsdInfo, DsdRead, DsdWrite, MAX_CHUNKS,
};
use std::io::{Read, Seek, SeekFrom, Write};

fn be_u64(b: &[u8; 12]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[4..12]);
    u64::from_be_bytes(a)
}

/// DFF reader over any seekable byte source.
#[derive(Debug)]
pub struct DffReader<R> {
    r: R,
    info: DsdInfo,
    remaining: u64,
    buf: Vec<u8>,
}

impl<R: Read + Seek> DffReader<R> {
    /// Parses the headers and positions the reader at the first audio byte.
    pub fn new(mut r: R) -> Result<Self> {
        let file_len = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(0))?;
        let hdr: [u8; 12] = read_exact_arr(&mut r)?;
        if &hdr[0..4] != b"FRM8" {
            return format_err("not a DSDIFF file (no 'FRM8' chunk)");
        }
        let form_type: [u8; 4] = read_exact_arr(&mut r)?;
        if &form_type != b"DSD " {
            return format_err("DSDIFF form type is not 'DSD '");
        }
        let form_end = 12u64.saturating_add(be_u64(&hdr)).min(file_len);
        let mut pos = 16u64;
        let mut rate = None;
        let mut channels = None;
        let mut have_cmpr = false;
        let mut chunks = 0;
        loop {
            if chunks == MAX_CHUNKS {
                return format_err("DSDIFF file has too many chunks");
            }
            chunks += 1;
            if pos + 12 > form_end {
                return format_err("DSDIFF file has no 'DSD ' audio chunk");
            }
            let ck: [u8; 12] = read_exact_arr(&mut r)?;
            let size = be_u64(&ck);
            let body = pos + 12;
            let end = body.checked_add(size).filter(|&e| e <= form_end);
            let id = &ck[0..4];
            match id {
                b"DSD " => {
                    let Some(end) = end else {
                        return format_err("DSDIFF 'DSD ' chunk runs past the end of the file");
                    };
                    let (Some(rate), Some(ch)) = (rate, channels) else {
                        return format_err(
                            "DSDIFF 'DSD ' chunk before the sample rate and channels",
                        );
                    };
                    if !have_cmpr {
                        return format_err("DSDIFF file has no 'CMPR' chunk");
                    }
                    let len = end - body;
                    let per_ch = len / u64::from(ch);
                    let Some(samples) = per_ch.checked_mul(8) else {
                        return format_err("DSDIFF 'DSD ' chunk is too large");
                    };
                    return Ok(DffReader {
                        r,
                        info: DsdInfo {
                            channels: ch,
                            sample_rate: rate,
                            samples,
                        },
                        remaining: per_ch,
                        buf: Vec::new(),
                    });
                }
                b"DST " => return unsupported("DST-compressed DSDIFF; decompress it first"),
                b"PROP" => {
                    let Some(end) = end else {
                        return format_err("DSDIFF 'PROP' chunk runs past the end of the file");
                    };
                    let pt: [u8; 4] = read_exact_arr(&mut r)?;
                    if &pt != b"SND " {
                        return format_err("DSDIFF 'PROP' chunk is not of type 'SND '");
                    }
                    let mut p = body + 4;
                    let mut subs = 0;
                    while p + 12 <= end {
                        if subs == MAX_CHUNKS {
                            return format_err("DSDIFF 'PROP' chunk has too many property chunks");
                        }
                        subs += 1;
                        let sc: [u8; 12] = read_exact_arr(&mut r)?;
                        let ssize = be_u64(&sc);
                        let sbody = p + 12;
                        let Some(send) = sbody.checked_add(ssize).filter(|&e| e <= end) else {
                            return format_err("DSDIFF property chunk runs past its 'PROP' chunk");
                        };
                        match &sc[0..4] {
                            b"FS  " => {
                                if ssize < 4 {
                                    return format_err("DSDIFF 'FS  ' chunk is too small");
                                }
                                let v: [u8; 4] = read_exact_arr(&mut r)?;
                                let fs = u32::from_be_bytes(v);
                                check_dsd_rate(fs)?;
                                rate = Some(fs);
                                skip(&mut r, ssize - 4)?;
                            }
                            b"CHNL" => {
                                if ssize < 2 {
                                    return format_err("DSDIFF 'CHNL' chunk is too small");
                                }
                                let v: [u8; 2] = read_exact_arr(&mut r)?;
                                let n = u16::from_be_bytes(v);
                                if !(1..=6).contains(&n) {
                                    return format_err(format!(
                                        "DSDIFF channel count {n} is not 1..6"
                                    ));
                                }
                                if ssize < 2 + 4 * u64::from(n) {
                                    return format_err("DSDIFF 'CHNL' chunk is too small");
                                }
                                channels = Some(n);
                                skip(&mut r, ssize - 2)?;
                            }
                            b"CMPR" => {
                                if ssize < 4 {
                                    return format_err("DSDIFF 'CMPR' chunk is too small");
                                }
                                let v: [u8; 4] = read_exact_arr(&mut r)?;
                                match &v {
                                    b"DSD " => {}
                                    b"DST " => {
                                        return unsupported(
                                            "DST-compressed DSDIFF; decompress it first",
                                        )
                                    }
                                    _ => return unsupported("DSDIFF compression type"),
                                }
                                have_cmpr = true;
                                skip(&mut r, ssize - 4)?;
                            }
                            _ => skip(&mut r, ssize)?,
                        }
                        p = (send + (ssize & 1)).min(end);
                        seek_to(&mut r, p)?;
                    }
                    seek_to(&mut r, end)?;
                    pos = end;
                }
                b"FVER" => {
                    if size < 4 || end.is_none() {
                        return format_err("DSDIFF 'FVER' chunk is invalid");
                    }
                    let v: [u8; 4] = read_exact_arr(&mut r)?;
                    if v[0] != 1 {
                        return unsupported(format!("DSDIFF version {}.{}", v[0], v[1]));
                    }
                    skip(&mut r, size - 4)?;
                    pos = body + size;
                }
                _ => {
                    let Some(end) = end else {
                        return format_err("DSDIFF chunk runs past the end of the file");
                    };
                    seek_to(&mut r, end)?;
                    pos = end;
                }
            }
            if pos & 1 == 1 && pos < form_end {
                r.seek_relative(1)?;
                pos += 1;
            }
        }
    }
}

impl<R: Read + Seek> DsdRead for DffReader<R> {
    fn info(&self) -> DsdInfo {
        self.info
    }

    fn read_bytes(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize> {
        let ch = usize::from(self.info.channels);
        if out.len() != ch {
            return Err(crate::Error::InvalidArgument(
                "output channel count mismatch".into(),
            ));
        }
        let n = (max_bytes.clamp(1, crate::io::MAX_CHUNK) as u64).min(self.remaining) as usize;
        if n == 0 {
            return Ok(0);
        }
        self.buf.resize(n * ch, 0);
        self.r.read_exact(&mut self.buf)?;
        for (c, o) in out.iter_mut().enumerate() {
            o.extend(self.buf.iter().skip(c).step_by(ch));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// DFF writer. Writes placeholder sizes, then patches them in [`DsdWrite::finish`].
#[derive(Debug)]
pub struct DffWriter<W: Write + Seek> {
    w: W,
    channels: usize,
    data_start: u64,
    bytes_per_ch: u64,
    finished: bool,
}

impl<W: Write + Seek> DffWriter<W> {
    /// Starts a DFF file with `channels` channels (1..=6) at `rate` Hz.
    pub fn new(mut w: W, channels: u16, rate: u32) -> Result<Self> {
        if !(1..=6).contains(&channels) {
            return Err(crate::Error::InvalidArgument(format!(
                "{channels} channels: 1 to 6 are supported"
            )));
        }
        check_dsd_rate(rate)?;
        let ids: &[&[u8; 4]] = match channels {
            1 => &[b"C   "],
            2 => &[b"SLFT", b"SRGT"],
            5 => &[b"MLFT", b"MRGT", b"C   ", b"LS  ", b"RS  "],
            6 => &[b"MLFT", b"MRGT", b"C   ", b"LFE ", b"LS  ", b"RS  "],
            3 => &[b"C000", b"C001", b"C002"],
            _ => &[b"C000", b"C001", b"C002", b"C003"],
        };
        let mut chnl = Vec::new();
        chnl.extend_from_slice(&channels.to_be_bytes());
        for id in ids {
            chnl.extend_from_slice(*id);
        }
        // pstring: count byte, text, pad byte so the total length is even
        let mut cmpr = b"DSD ".to_vec();
        let name = b"not compressed";
        cmpr.push(name.len() as u8);
        cmpr.extend_from_slice(name);
        if cmpr.len() % 2 == 1 {
            cmpr.push(0);
        }
        let mut prop = b"SND ".to_vec();
        for (id, body) in [
            (b"FS  ", rate.to_be_bytes().to_vec()),
            (b"CHNL", chnl),
            (b"CMPR", cmpr),
        ] {
            prop.extend_from_slice(id);
            prop.extend_from_slice(&(body.len() as u64).to_be_bytes());
            prop.extend_from_slice(&body);
            if body.len() % 2 == 1 {
                prop.push(0);
            }
        }
        let mut h = Vec::new();
        h.extend_from_slice(b"FRM8");
        h.extend_from_slice(&0u64.to_be_bytes());
        h.extend_from_slice(b"DSD ");
        h.extend_from_slice(b"FVER");
        h.extend_from_slice(&4u64.to_be_bytes());
        h.extend_from_slice(&0x0105_0000u32.to_be_bytes());
        h.extend_from_slice(b"PROP");
        h.extend_from_slice(&(prop.len() as u64).to_be_bytes());
        h.extend_from_slice(&prop);
        h.extend_from_slice(b"DSD ");
        h.extend_from_slice(&0u64.to_be_bytes());
        w.write_all(&h)?;
        Ok(DffWriter {
            w,
            channels: usize::from(channels),
            data_start: h.len() as u64,
            bytes_per_ch: 0,
            finished: false,
        })
    }

    /// Returns the inner writer (after [`DsdWrite::finish`]).
    pub fn into_inner(self) -> W {
        self.w
    }
}

impl<W: Write + Seek> DsdWrite for DffWriter<W> {
    fn write_bytes(&mut self, data: &[Vec<u8>]) -> Result<()> {
        if data.len() != self.channels || data.iter().any(|d| d.len() != data[0].len()) {
            return Err(crate::Error::InvalidArgument("unequal channel data".into()));
        }
        let n = data[0].len();
        let mut out = Vec::with_capacity(n * self.channels);
        for i in 0..n {
            for d in data {
                out.push(d[i]);
            }
        }
        self.w.write_all(&out)?;
        self.bytes_per_ch += n as u64;
        Ok(())
    }

    fn finish(&mut self, _samples: u64) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        let data_len = self.bytes_per_ch * self.channels as u64;
        let mut end = self.data_start + data_len;
        if data_len % 2 == 1 {
            self.w.write_all(&[0])?;
            end += 1;
        }
        self.w.seek(SeekFrom::Start(4))?;
        self.w.write_all(&(end - 12).to_be_bytes())?;
        self.w.seek(SeekFrom::Start(self.data_start - 8))?;
        self.w.write_all(&data_len.to_be_bytes())?;
        self.w.seek(SeekFrom::End(0))?;
        self.w.flush()?;
        self.finished = true;
        Ok(())
    }
}
