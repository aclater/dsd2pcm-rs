//! Release test (docs/release-test-plan.md): checks a WAV written by the `dsd2pcm` tool
//! against scarletbook-pcm 0.7.0, which converts the same DSF or DFF file here, read by
//! this program's own small parsers (not the library's readers).
//!
//! ```text
//! cargo run --release -p dsd2pcm --example real_identity -- IN.dsf|IN.dff OUT.wav RATE CUTOFF
//! ```
//!
//! RATE is 88200, 176400 or 352800 and CUTOFF 30, 40 or wide (default filter, gain 0). The
//! input streams through in runs of frames with their neighbours, so memory stays small
//! for files of any length. Checks, each a failure (exit status 1) when it does not hold:
//!
//! - OUT is a RIFF/WAVE file with one `fmt ` (24-bit PCM, the input's channel count, RATE)
//!   and one `data` chunk whose size fits the file and is a whole number of sample frames;
//! - every output sample of the input's whole 4704-byte frames equals scarletbook-pcm's,
//!   with the last whole frame left out when a partial frame follows it (its context then
//!   differs); a DSF sample count that is not a multiple of 8 has its last byte's padding
//!   bits set to the idle pattern 0x69's bits, as the library documents;
//! - the output has at least as many samples per channel as the whole frames give, and at
//!   most one more than the input's samples divided by the decimation;
//! - when every channel's last 4704 bytes are the idle pattern 0x69 (after that masking),
//!   the output's last 64 samples of every channel are exact zeros.
//!
//! Prints one line, starting `OK` or `FAIL`.

use scarletbook_pcm as sb;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::process::ExitCode;

const FRAME: usize = 4704;
const RUN: usize = 64;
const IDLE: u8 = 0x69;

/// A DSD64 input: channel count, bytes per channel, the bits of padding in the last byte,
/// and where its audio is.
struct Input {
    ch: usize,
    bytes: u64,
    pad: u32,
    /// DSF: the data offset and block size (bytes are LSB first, per-channel blocks).
    /// DFF: the data offset, block 0 (bytes are MSB first, interleaved byte by byte).
    offset: u64,
    block: usize,
    file: BufReader<File>,
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().unwrap())
}
fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}
fn u64be(b: &[u8]) -> u64 {
    u64::from_be_bytes(b[..8].try_into().unwrap())
}

fn open(path: &str) -> Result<Input, String> {
    let mut f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut head = vec![0u8; 1 << 16];
    let n = f.read(&mut head).map_err(|e| e.to_string())?;
    head.truncate(n);
    let file = BufReader::with_capacity(1 << 20, f);
    match &head[..4] {
        b"DSD " => {
            let fmt = 28usize;
            if &head[fmt..fmt + 4] != b"fmt " {
                return Err("DSF: no fmt chunk at 28".into());
            }
            let ch = u32le(&head[fmt + 24..]) as usize;
            let fs = u32le(&head[fmt + 28..]);
            let bits = u32le(&head[fmt + 32..]);
            let samples = u64le(&head[fmt + 36..]);
            let block = u32le(&head[fmt + 44..]) as usize;
            let data = fmt + u64le(&head[fmt + 4..]) as usize;
            if &head[data..data + 4] != b"data" || fs != 2_822_400 || bits != 1 {
                return Err(format!("DSF: not DSD64 LSB-first ({fs} Hz, {bits} bits)"));
            }
            Ok(Input {
                ch,
                bytes: samples.div_ceil(8),
                pad: (samples.div_ceil(8) * 8 - samples) as u32,
                offset: data as u64 + 12,
                block,
                file,
            })
        }
        b"FRM8" => {
            let (mut pos, mut ch, mut fs) = (16usize, 0usize, 0u32);
            while pos + 12 <= head.len() {
                let id = &head[pos..pos + 4];
                let size = u64be(&head[pos + 4..]);
                if id == b"PROP" {
                    let end = pos + 12 + size as usize;
                    let mut q = pos + 16;
                    while q + 12 <= end.min(head.len()) {
                        let s = u64be(&head[q + 4..]) as usize;
                        match &head[q..q + 4] {
                            b"FS  " => {
                                fs = u32::from_be_bytes(head[q + 12..q + 16].try_into().unwrap())
                            }
                            b"CHNL" => {
                                ch = u16::from_be_bytes([head[q + 12], head[q + 13]]) as usize
                            }
                            b"CMPR" if &head[q + 12..q + 16] != b"DSD " => {
                                return Err("DFF: compressed".into())
                            }
                            _ => {}
                        }
                        q += 12 + s + (s & 1);
                    }
                }
                if id == b"DSD " {
                    if fs != 2_822_400 || ch == 0 {
                        return Err(format!("DFF: not DSD64 ({fs} Hz, {ch} channels)"));
                    }
                    return Ok(Input {
                        ch,
                        bytes: size / ch as u64,
                        pad: 0,
                        offset: pos as u64 + 12,
                        block: 0,
                        file,
                    });
                }
                pos += 12 + size as usize + (size as usize & 1);
            }
            Err("DFF: no DSD chunk in the first 64 KiB".into())
        }
        _ => Err("not DSF or DFF".into()),
    }
}

impl Input {
    /// Bytes `[from, from + n)` of every channel, interleaved byte by byte, MSB first,
    /// with the padding of the very last byte set to 0x69's bits.
    fn interleaved(&mut self, from: u64, n: usize) -> Result<Vec<u8>, String> {
        let ch = self.ch;
        let mut out = vec![0u8; n * ch];
        if self.block == 0 {
            self.file
                .seek(SeekFrom::Start(self.offset + from * ch as u64))
                .map_err(|e| e.to_string())?;
            self.file.read_exact(&mut out).map_err(|e| e.to_string())?;
        } else {
            let bs = self.block as u64;
            let mut buf = vec![0u8; self.block];
            let mut i = from;
            while i < from + n as u64 {
                let (blk, off) = (i / bs, (i % bs) as usize);
                let take = (self.block - off).min((from + n as u64 - i) as usize);
                for c in 0..ch {
                    let at = self.offset + (blk * ch as u64 + c as u64) * bs + off as u64;
                    self.file
                        .seek(SeekFrom::Start(at))
                        .map_err(|e| e.to_string())?;
                    self.file
                        .read_exact(&mut buf[..take])
                        .map_err(|e| e.to_string())?;
                    for (k, &b) in buf[..take].iter().enumerate() {
                        out[((i - from) as usize + k) * ch + c] = b.reverse_bits();
                    }
                }
                i += take as u64;
            }
        }
        if self.pad > 0 && from + n as u64 == self.bytes {
            let mask = ((1u16 << self.pad) - 1) as u8;
            for c in 0..ch {
                let b = &mut out[(n - 1) * ch + c];
                *b = (*b & !mask) | (IDLE & mask);
            }
        }
        Ok(out)
    }
}

/// The `data` chunk of a 24-bit WAV: (offset, size).
fn wav(path: &str, ch: usize, rate: u32) -> Result<(BufReader<File>, u64, u64), String> {
    let mut f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let mut h = [0u8; 12];
    f.read_exact(&mut h).map_err(|e| e.to_string())?;
    if &h[..4] != b"RIFF" || &h[8..] != b"WAVE" {
        return Err("output: not RIFF/WAVE".into());
    }
    if u32le(&h[4..]) as u64 + 8 != len {
        return Err(format!(
            "output: RIFF size {} + 8 != file size {len}",
            u32le(&h[4..])
        ));
    }
    let (mut pos, mut fmt_ok, mut data) = (12u64, false, None);
    while pos + 8 <= len {
        let mut c = [0u8; 8];
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        f.read_exact(&mut c).map_err(|e| e.to_string())?;
        let size = u32le(&c[4..]) as u64;
        if pos + 8 + size > len {
            return Err(format!("output: chunk {:?} runs past the end", &c[..4]));
        }
        match &c[..4] {
            b"fmt " => {
                let mut b = vec![0u8; size as usize];
                f.read_exact(&mut b).map_err(|e| e.to_string())?;
                let tag = u16::from_le_bytes([b[0], b[1]]);
                let chans = u16::from_le_bytes([b[2], b[3]]) as usize;
                let sr = u32le(&b[4..]);
                let align = u16::from_le_bytes([b[12], b[13]]) as usize;
                let bits = u16::from_le_bytes([b[14], b[15]]);
                if !(tag == 1 || tag == 0xFFFE)
                    || chans != ch
                    || sr != rate
                    || bits != 24
                    || align != 3 * ch
                {
                    return Err(format!(
                        "output fmt: tag {tag:#x}, {chans} ch, {sr} Hz, {bits} bits, align {align}"
                    ));
                }
                fmt_ok = true;
            }
            b"data" if data.is_none() => data = Some((pos + 8, size)),
            b"data" => return Err("output: two data chunks".into()),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    let Some((off, size)) = data else {
        return Err("output: no data chunk".into());
    };
    if !fmt_ok || size % (3 * ch as u64) != 0 {
        return Err(format!("output: fmt {fmt_ok}, data size {size}"));
    }
    Ok((BufReader::with_capacity(1 << 20, f), off, size))
}

fn run() -> Result<String, String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 4 {
        return Err("usage: real_identity IN.dsf|IN.dff OUT.wav RATE 30|40|wide".into());
    }
    let mut inp = open(&a[0])?;
    let hz: u32 = a[2].parse().map_err(|_| "bad rate")?;
    let rate = sb::PcmRate::from_hz(hz).ok_or("bad rate")?;
    let cutoff = match a[3].as_str() {
        "30" => sb::Cutoff::Low,
        "40" => sb::Cutoff::Standard,
        "wide" => sb::Cutoff::Wide,
        _ => return Err("bad cutoff".into()),
    };
    let ch = inp.ch;
    let mut conv = sb::Converter::new(sb::PcmConfig::new(rate, cutoff, ch as u8))
        .map_err(|e| format!("scarletbook-pcm: {e:?}"))?;
    let spf = conv.samples_per_frame();
    let (mut w, data_off, data_size) = wav(&a[1], ch, hz)?;
    let out_samples = data_size / (3 * ch as u64);

    let frames = (inp.bytes / FRAME as u64) as usize;
    let partial = inp.bytes % FRAME as u64 != 0;
    let compared = if partial {
        frames.saturating_sub(1)
    } else {
        frames
    };
    let decim = 2_822_400 / hz as u64;
    let samples = inp.bytes * 8 - inp.pad as u64;
    if out_samples < (frames * spf) as u64 || out_samples > samples / decim + 1 {
        return Err(format!(
            "output has {out_samples} samples per channel; whole frames give {}, input/decimation {}",
            frames * spf,
            samples / decim
        ));
    }

    let fb = FRAME * ch;
    let mut ours = vec![0u8; RUN * spf * ch * 3];
    let mut theirs = vec![0u8; RUN * spf * ch * 3];
    let mut f = 0usize;
    while f < compared {
        let n = RUN.min(compared - f);
        // The run with one frame on each side (when there is one).
        let lo = f.saturating_sub(1);
        let hi = (f + n + 1).min(frames);
        let buf = inp.interleaved((lo * FRAME) as u64, (hi - lo) * FRAME)?;
        let before = (f > 0).then(|| &buf[..fb]);
        let start = (f - lo) * fb;
        let body = &buf[start..start + n * fb];
        let after = (f + n < frames).then(|| &buf[start + n * fb..start + (n + 1) * fb]);
        let len = n * spf * ch * 3;
        conv.convert(
            sb::Run::new(body).with_before(before).with_after(after),
            sb::PcmOut::S24le(&mut theirs[..len]),
            None,
        )
        .map_err(|e| format!("scarletbook-pcm: {e:?}"))?;
        w.seek(SeekFrom::Start(data_off + (f * spf * ch * 3) as u64))
            .map_err(|e| e.to_string())?;
        w.read_exact(&mut ours[..len]).map_err(|e| e.to_string())?;
        if ours[..len] != theirs[..len] {
            let i = (0..len).find(|&i| ours[i] != theirs[i]).unwrap();
            let s = i / 3;
            return Err(format!(
                "differs from scarletbook-pcm at output sample {} of channel {} (frame {})",
                f * spf + s / ch,
                s % ch,
                f + s / ch / spf
            ));
        }
        f += n;
    }

    // Silent end: the last frame of every channel idle, then the last 64 outputs are zero.
    let mut tail = "end not idle";
    if inp.bytes >= FRAME as u64 {
        let last = inp.interleaved(inp.bytes - FRAME as u64, FRAME)?;
        if last.iter().all(|&b| b == IDLE) {
            let n = 64.min(out_samples) as usize;
            let mut t = vec![0u8; n * ch * 3];
            w.seek(SeekFrom::Start(data_off + data_size - t.len() as u64))
                .map_err(|e| e.to_string())?;
            w.read_exact(&mut t).map_err(|e| e.to_string())?;
            if t.iter().any(|&b| b != 0) {
                return Err("idle end: the last 64 output samples are not all zero".into());
            }
            tail = "idle end: last 64 samples zero";
        }
    }
    Ok(format!(
        "{ch} ch, {} bytes/ch (pad {}), {hz} Hz {}: {out_samples} samples/ch, {} identical to scarletbook-pcm 0.7.0 ({compared}/{frames} frames{}); {tail}",
        inp.bytes,
        inp.pad,
        a[3],
        compared * spf,
        if partial { ", partial frame after" } else { "" }
    ))
}

fn main() -> ExitCode {
    match run() {
        Ok(s) => {
            println!("OK {s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("FAIL {e}");
            ExitCode::FAILURE
        }
    }
}
