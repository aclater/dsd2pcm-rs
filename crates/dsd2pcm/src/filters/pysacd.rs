// SPDX-License-Identifier: MIT OR Apache-2.0
//! pysacd's DSD64 to PCM filter ([`super::Filter::Pysacd`]), bit-exact with its reference
//! build.
//!
//! Ported from pysacd (commit 94b2c53), `csrc/dsp.c` and `csrc/dsp.h`, MIT OR Apache-2.0
//! (relicensed by their copyright holder; Copyright (c) 2026 the pysacd authors), and the
//! 24-bit quantiser of `process_batch()` in `csrc/sacdconv.c`. Stage 1 evaluates its filter
//! with lookup tables indexed by whole DSD bytes, a technique credited to Sebastian
//! Gesemann's dsd2pcm (BSD-3-Clause, 2009). See PROVENANCE.md.
//!
//! Three linear-phase Kaiser-windowed FIR stages, in `f32`:
//! 1. 2.8224 MHz to 352.8 kHz (/8), 96 taps, as 12 byte tables summed in order;
//! 2. 352.8 kHz to 176.4 kHz (/2), 95 taps (pass to 60 kHz, stop from 88.2 kHz);
//! 3. 176.4 kHz to 88.2 kHz (/2), 161 taps (pass to 40 kHz, stop from 48 kHz), 88.2 kHz only.
//!
//! The taps are the bits `dsd2pcm_init` computes (`pysacd_taps.rs`, generated from the C
//! code by `tests/pysacd-ref/gen.sh`). Each output is the sum of its products in tap order,
//! each product rounded to `f32` and then added (no fused multiply-add), exactly as the C
//! loop compiled with `-ffp-contract=off`. The SIMD builds compute several outputs at once,
//! each in that order, so every SIMD level gives the same bits.
//!
//! Each area is one stream: the C filter started at the area's start (stage 1 history of
//! DSD silence, 0x69; zeros in the later stages' histories) and fed every frame without a
//! reset, then DSD silence after the end. The first `delay` outputs (the group delay) are
//! dropped. So every output is a function of its absolute position and the input around
//! it, and any split of the work gives the same bytes.
//!
//! Quantiser: `scale = 8388607 x 10^(gain/20)` in `f32`; TPDF dither
//! `(x() - x()) x 2^-32` from xorshift32, two draws per sample, channel by channel within
//! each sample; added after scaling, rounded half to even and clamped to 24 bits. The
//! generator is seeded at the start of every Super Audio CD frame (1/75 s) from its index
//! `tc` in the area: `0x9E3779B9 ^ (tc x 2654435761)` (wrapping, 32 bits).

use super::pysacd_taps as taps;
use crate::convert::{run_pieces, PcmOptions, Report};
use crate::decimator::{self, Cutoff, Isa, PcmRate, IDLE};
use crate::error::{Error, Result};
use crate::frames::{self, ConvertStats, PcmOut, Run, FRAME_BYTES_PER_CHANNEL};
use crate::io::DsdRead;
use crate::wav::WavWriter;
use std::io::{Seek, Write};
use std::sync::Mutex;

/// Stage 1: bytes per output (12 tables of 8 taps).
const S1_BYTES: usize = 12;
/// One table per byte of the stage 1 window, oldest first.
pub(crate) type Lut = [[f32; 256]; S1_BYTES];

/// The filter for one output rate.
pub(crate) struct Pysacd {
    /// Three stages (88.2 kHz) or two (176.4 kHz).
    three: bool,
    lut: Box<Lut>,
    h2: [f32; 95],
    h3: [f32; 161],
    /// Group delay in output samples: outputs before it are dropped.
    delay: i64,
    isa: Isa,
}

impl std::fmt::Debug for Pysacd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pysacd")
            .field("three", &self.three)
            .field("delay", &self.delay)
            .finish_non_exhaustive()
    }
}

impl Pysacd {
    /// The filter for `rate` (88.2 or 176.4 kHz). The response is fixed: `cutoff` must be
    /// [`Cutoff::Standard`] (the default).
    pub(crate) fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Pysacd> {
        let three = match rate {
            PcmRate::Hz88200 => true,
            PcmRate::Hz176400 => false,
            PcmRate::Hz352800 => {
                return Err(Error::InvalidArgument(
                    "the pysacd filter has 88.2 and 176.4 kHz only".into(),
                ))
            }
        };
        if cutoff != Cutoff::Standard {
            return Err(Error::InvalidArgument(
                "the pysacd filter has a fixed response: leave the cutoff at the default".into(),
            ));
        }
        let h1 = taps::H1.map(f64::from_bits);
        let mut lut = Box::new([[0f32; 256]; S1_BYTES]);
        for (j, t) in lut.iter_mut().enumerate() {
            for (v, e) in t.iter_mut().enumerate() {
                // As dsd2pcm_init: in double, bits MSB first, 1 -> +1, 0 -> -1.
                let mut acc = 0f64;
                for i in 0..8 {
                    let s = if (v >> (7 - i)) & 1 == 1 { 1.0 } else { -1.0 };
                    acc += h1[j * 8 + i] * s;
                }
                *e = acc as f32;
            }
        }
        Ok(Pysacd {
            three,
            lut,
            h2: taps::H2.map(f32::from_bits),
            h3: taps::H3.map(f32::from_bits),
            delay: taps::DELAY[usize::from(!three)] as i64,
            isa: Isa::current(),
        })
    }

    /// Output samples per channel per frame: 1176 or 2352.
    pub(crate) fn per_frame(&self) -> usize {
        if self.three {
            1176
        } else {
            2352
        }
    }

    /// Input bytes per output sample.
    fn k(&self) -> i64 {
        if self.three {
            4
        } else {
            2
        }
    }

    /// The first byte that output `n` (or a later one) needs.
    pub(crate) fn first_byte(&self, n: i64) -> i64 {
        let o = n + self.delay;
        if self.three {
            4 * o - 422
        } else {
            2 * o - 104
        }
    }

    /// The last byte that output `n` needs.
    pub(crate) fn last_byte(&self, n: i64) -> i64 {
        let k = self.k();
        k * (n + self.delay) + k - 1
    }

    /// The end (exclusive) of the outputs whose bytes all lie before byte `end`.
    fn ready_end(&self, end: i64) -> i64 {
        let k = self.k();
        (end - k).div_euclid(k) - self.delay + 1
    }

    /// Outputs `n0 .. n0 + out.len()` of one channel, before quantising. `src` holds bytes
    /// `src0 ..`; all other bytes are DSD silence. Bytes and outputs count from the same
    /// origin; `area_start` is the area's first byte, where the stream starts (`None`: so far
    /// back that no output reaches it).
    pub(crate) fn render(
        &self,
        src: &[u8],
        src0: i64,
        area_start: Option<i64>,
        n0: i64,
        out: &mut [f32],
        sc: &mut Scratch,
    ) {
        let n = out.len();
        if n == 0 {
            return;
        }
        let lo = self.first_byte(n0);
        let hi = self.last_byte(n0 + n as i64 - 1) + 1;
        let len = (hi - lo) as usize;
        // The window, with silence outside `src`.
        sc.bytes.clear();
        sc.bytes.resize(len, IDLE);
        let a = lo.max(src0);
        let b = hi.min(src0 + src.len() as i64);
        if a < b {
            sc.bytes[(a - lo) as usize..(b - lo) as usize]
                .copy_from_slice(&src[(a - src0) as usize..(b - src0) as usize]);
        }
        // Stage 1: one output per byte, from the 11 bytes before it and itself.
        let t1lo = lo + S1_BYTES as i64 - 1;
        let m1 = len - (S1_BYTES - 1);
        sc.t1.resize(m1, 0.0);
        self.isa.pysacd_s1(&self.lut, &sc.bytes, &mut sc.t1);
        // Before the area's start, the later stages' histories hold zeros.
        if let Some(a) = area_start {
            let z = (a - t1lo).clamp(0, m1 as i64) as usize;
            sc.t1[..z].fill(0.0);
        }
        if self.three {
            let m2 = 2 * n + 159;
            let t2lo = 2 * (n0 + self.delay) - 159;
            let mut t2 = std::mem::take(&mut sc.t2);
            t2.resize(m2, 0.0);
            dec2(self.isa, &self.h2, &sc.t1, &mut sc.xe, &mut sc.xo, &mut t2);
            if let Some(a) = area_start {
                let z = (a.div_euclid(2) - t2lo).clamp(0, m2 as i64) as usize;
                t2[..z].fill(0.0);
            }
            dec2(self.isa, &self.h3, &t2, &mut sc.xe, &mut sc.xo, out);
            sc.t2 = t2;
        } else {
            dec2(self.isa, &self.h2, &sc.t1, &mut sc.xe, &mut sc.xo, out);
        }
    }
}

/// `out[o] = sum over k of h[k] x[2o + k]`, in tap order, through even and odd halves.
fn dec2(isa: Isa, h: &[f32], x: &[f32], xe: &mut Vec<f32>, xo: &mut Vec<f32>, out: &mut [f32]) {
    debug_assert_eq!(x.len(), 2 * out.len() + h.len() - 2);
    xe.clear();
    xo.clear();
    xe.extend(x.iter().step_by(2));
    xo.extend(x.iter().skip(1).step_by(2));
    isa.pysacd_dec2(h, xe, xo, out);
}

/// Buffers for [`Pysacd::render`] and the quantiser; reused.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    bytes: Vec<u8>,
    t1: Vec<f32>,
    t2: Vec<f32>,
    xe: Vec<f32>,
    xo: Vec<f32>,
    /// Outputs per channel, back to back.
    f: Vec<f32>,
    /// Quantised samples, interleaved.
    q: Vec<i32>,
    /// Clamped samples per frame and channel.
    clips: Vec<u64>,
}

/// Outputs per block of the kernels: 4 AVX vectors, or 8 SSE vectors.
pub(crate) const L: usize = 32;

/// Stage 1: `out[i] = sum over j of lut[j][bytes[i + j]]`, in table order.
#[inline(always)]
pub(crate) fn s1_kernel(lut: &Lut, bytes: &[u8], out: &mut [f32]) {
    let n = out.len();
    debug_assert!(bytes.len() >= n + S1_BYTES - 1);
    let mut o = 0;
    while o + L <= n {
        let mut acc = [0f32; L];
        for (j, t) in lut.iter().enumerate() {
            let b = &bytes[o + j..][..L];
            for i in 0..L {
                acc[i] += t[usize::from(b[i])];
            }
        }
        out[o..o + L].copy_from_slice(&acc);
        o += L;
    }
    for (i, d) in out.iter_mut().enumerate().skip(o) {
        let mut acc = 0f32;
        for (j, t) in lut.iter().enumerate() {
            acc += t[usize::from(bytes[i + j])];
        }
        *d = acc;
    }
}

/// A decimate-by-2 stage: `out[o] = sum over k of h[k] x[2o + k]` with `xe[i] = x[2i]` and
/// `xo[i] = x[2i + 1]`, each product rounded and then added, in tap order (`h` odd length).
#[inline(always)]
pub(crate) fn dec2_kernel(h: &[f32], xe: &[f32], xo: &[f32], out: &mut [f32]) {
    let n = out.len();
    let pairs = h.len() / 2;
    debug_assert!(h.len() % 2 == 1 && xe.len() > n + pairs - 1 && xo.len() >= n + pairs - 1);
    let last = h[2 * pairs];
    let mut o = 0;
    while o + L <= n {
        let mut acc = [0f32; L];
        for p in 0..pairs {
            let (he, ho) = (h[2 * p], h[2 * p + 1]);
            let e = &xe[o + p..][..L];
            for i in 0..L {
                acc[i] += he * e[i];
            }
            let od = &xo[o + p..][..L];
            for i in 0..L {
                acc[i] += ho * od[i];
            }
        }
        let e = &xe[o + pairs..][..L];
        for i in 0..L {
            acc[i] += last * e[i];
        }
        out[o..o + L].copy_from_slice(&acc);
        o += L;
    }
    for (i, d) in out.iter_mut().enumerate().skip(o) {
        let mut acc = 0f32;
        for p in 0..pairs {
            acc += h[2 * p] * xe[i + p];
            acc += h[2 * p + 1] * xo[i + p];
        }
        acc += last * xe[i + pairs];
        *d = acc;
    }
}

fn xorshift(s: &mut u32) -> u32 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    x
}

/// The dither generator's state at the start of frame `tc`.
fn seed(tc: u64) -> u32 {
    0x9E37_79B9 ^ (tc as u32).wrapping_mul(2_654_435_761)
}

/// Quantises the outputs `chans` (one slice per channel, equal lengths), the first of which
/// is output `n0` of frame `tc0 + n0 / per_frame`, into `out` (interleaved), and counts the
/// clamped samples into `clips[(frame - n0 / per_frame) x channels + c]`.
#[inline(always)]
pub(crate) fn quantise_kernel(
    chans: &[&[f32]],
    n0: u64,
    tc0: u64,
    per_frame: usize,
    scale: f32,
    out: &mut [i32],
    clips: &mut [u64],
) {
    const LSB: f32 = 1.0 / 4_294_967_296.0;
    let ch = chans.len();
    let pf = per_frame as u64;
    let mut frame = n0 / pf;
    let mut pos = (n0 % pf) as usize;
    let mut rng = seed(tc0.wrapping_add(frame));
    for _ in 0..2 * ch * pos {
        xorshift(&mut rng);
    }
    let mut fi = 0;
    for (i, d) in out.chunks_exact_mut(ch).enumerate() {
        if pos == per_frame {
            pos = 0;
            frame += 1;
            fi += ch;
            rng = seed(tc0.wrapping_add(frame));
        }
        for (c, d) in d.iter_mut().enumerate() {
            let a = xorshift(&mut rng) as f32;
            let b = xorshift(&mut rng) as f32;
            let dith = (a - b) * LSB;
            let v = (chans[c][i] * scale + dith).round_ties_even();
            *d = if v > 8_388_607.0 {
                clips[fi + c] += 1;
                8_388_607
            } else if v < -8_388_608.0 {
                clips[fi + c] += 1;
                -8_388_608
            } else {
                v as i32
            };
        }
        pos += 1;
    }
}

/// `8388607 x 10^(gain/20)` as `f32`, as pysacd computes it (the factor in `f64` rounded
/// once to `f32`).
pub(crate) fn scale(gain_db: f64) -> f32 {
    8_388_607.0f32 * decimator::db_to_linear(gain_db) as f32
}

impl Pysacd {
    /// Renders outputs `n0 .. n0 + n` of every channel (`chans[c]` holds bytes `src0 ..`)
    /// and quantises them into `sc.q` (interleaved) with the frames' clip counts in
    /// `sc.clips`; outputs count from the area's frame `tc0`.
    #[allow(clippy::too_many_arguments)]
    fn block<'a>(
        &self,
        chans: impl ExactSizeIterator<Item = &'a [u8]>,
        src0: i64,
        area_start: Option<i64>,
        n0: i64,
        n: usize,
        tc0: u64,
        scale: f32,
        sc: &mut Scratch,
    ) {
        let ch = chans.len();
        let mut f = std::mem::take(&mut sc.f);
        f.resize(n * ch, 0.0);
        for (src, fc) in chans.zip(f.chunks_exact_mut(n)) {
            self.render(src, src0, area_start, n0, fc, sc);
        }
        let pf = self.per_frame() as u64;
        let frames = ((n0 as u64 + n as u64).div_ceil(pf) - n0 as u64 / pf) as usize;
        sc.clips.clear();
        sc.clips.resize(frames * ch, 0);
        sc.q.resize(n * ch, 0);
        let per: [&[f32]; 6] =
            std::array::from_fn(|c| if c < ch { &f[c * n..][..n] } else { &[][..] });
        self.isa.pysacd_quantise(
            &per[..ch],
            n0 as u64,
            tc0,
            self.per_frame(),
            scale,
            &mut sc.q,
            &mut sc.clips,
        );
        sc.f = f;
    }
}

/// [`crate::convert::dsd64_to_pcm`] with this filter; the options are checked.
pub(crate) fn dsd64_to_pcm<W: Write + Seek>(
    mut r: Box<dyn DsdRead + Send>,
    w: &mut WavWriter<W>,
    opts: &PcmOptions,
) -> Result<Report> {
    let eng = Pysacd::new(opts.rate, opts.cutoff)?;
    let scale = scale(opts.gain_db);
    let info = r.info();
    let ch = usize::from(info.channels);
    if !(1..=6).contains(&ch) {
        return Err(Error::Unsupported(format!("{ch} channels")));
    }
    let n_out = decimator::output_len(info.samples, opts.rate);
    let workers = if opts.threads {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        1
    };
    let mut scratch: Vec<Scratch> = (0..workers).map(|_| Scratch::default()).collect();
    let mut clipped = vec![0u64; ch];
    let mut bufs = vec![Vec::new(); ch];
    let mut byte_start = 0i64;
    let mut done = 0u64;
    let mut eof = false;
    let mut bytes = Vec::new();
    // Pieces of work: whole frames of output, all channels.
    let piece = PIECE_FRAMES * eng.per_frame() as u64;
    while done < n_out {
        let got = if eof {
            0
        } else {
            r.read_bytes(CHUNK, &mut bufs)?
        };
        let end = byte_start + bufs[0].len() as i64;
        if got == 0 && !eof && (end as u64).saturating_mul(8) < info.samples {
            return Err(Error::Format(format!(
                "input ended after {end} of {} bytes per channel",
                info.samples.div_ceil(8)
            )));
        }
        eof |= got == 0;
        let ready = if eof {
            n_out
        } else {
            eng.ready_end(end).max(0) as u64
        };
        let stop = ready.min(n_out);
        if stop <= done {
            continue;
        }
        let keep = (stop - done) as usize;
        bytes.resize(keep * ch * 3, 0);
        let mut items = Vec::new();
        let mut rest = &mut bytes[..];
        let mut a = done;
        while a < stop {
            let b = ((a / piece + 1) * piece).min(stop);
            let (part, tail) = rest.split_at_mut((b - a) as usize * ch * 3);
            items.push((a, (b - a) as usize, Mutex::new(part)));
            rest = tail;
            a = b;
        }
        let results = run_pieces(&items, &mut scratch, |(a, n, part), sc| {
            eng.block(
                bufs.iter().map(|b| &b[..]),
                byte_start,
                Some(0),
                *a as i64,
                *n,
                0,
                scale,
                sc,
            );
            let mut part = part.lock().unwrap_or_else(|e| e.into_inner());
            frames::pack_s24le(&sc.q, &mut part);
            let mut clip = [0u64; 6];
            for (i, k) in sc.clips.iter().enumerate() {
                clip[i % ch] += k;
            }
            clip
        });
        for clip in results {
            for (t, k) in clipped.iter_mut().zip(clip) {
                *t += k;
            }
        }
        drop(items);
        w.write_raw(&bytes)?;
        done = stop;
        let first = eng.first_byte(done as i64);
        let drop = ((first - byte_start).max(0) as usize).min(bufs[0].len());
        for b in &mut bufs {
            b.drain(..drop);
        }
        byte_start += drop as i64;
    }
    Ok(Report {
        in_rate: info.sample_rate,
        out_rate: opts.rate.hz(),
        seconds: info.samples as f64 / f64::from(info.sample_rate),
        out_samples: n_out,
        clipped,
    })
}

/// Input bytes per channel read at a time.
const CHUNK: usize = 1 << 20;
/// Frames of output per piece of work.
const PIECE_FRAMES: u64 = 2;
/// Frames per block of the frames API.
const BLOCK_FRAMES: usize = 4;

/// The state of a [`crate::frames::Converter`] with this filter.
#[derive(Debug)]
pub(crate) struct FrameState {
    eng: Pysacd,
    scale: f32,
    sc: Scratch,
    bytes: Vec<u8>,
}

impl FrameState {
    pub(crate) fn new(rate: PcmRate, cutoff: Cutoff, gain_db: f64) -> Result<FrameState> {
        Ok(FrameState {
            eng: Pysacd::new(rate, cutoff)?,
            scale: scale(gain_db),
            sc: Scratch::default(),
            bytes: Vec::new(),
        })
    }

    /// Bytes per channel of each neighbour frame that the output depends on.
    pub(crate) fn context_bytes(&self) -> usize {
        let before = -self.eng.first_byte(0);
        let spf = self.eng.per_frame() as i64;
        let after = self.eng.last_byte(spf - 1) + 1 - FRAME_BYTES_PER_CHANNEL as i64;
        before.max(after).max(0) as usize
    }

    /// [`crate::frames::Converter::convert`] for a checked run of `frames` frames of `ch`
    /// channels and an output of the right length.
    pub(crate) fn convert(
        &mut self,
        run: &Run<'_>,
        frames: usize,
        ch: usize,
        mut out: PcmOut<'_>,
        mut frame_clipped: Option<&mut [u64]>,
    ) -> ConvertStats {
        let mut stats = ConvertStats::default();
        let spf = self.eng.per_frame();
        // The run starts the area if nothing comes before it.
        let area_start = run.before.is_none().then_some(0);
        let FrameState {
            eng,
            scale,
            sc,
            bytes,
        } = self;
        let mut f0 = 0;
        while f0 < frames {
            let nf = BLOCK_FRAMES.min(frames - f0);
            let n = nf * spf;
            let n0 = (f0 * spf) as i64;
            let lo = eng.first_byte(n0);
            let span = (eng.last_byte(n0 + n as i64 - 1) + 1 - lo) as usize;
            bytes.resize(span * ch, 0);
            frames::fill(run, frames, ch, lo, span, bytes);
            eng.block(
                bytes.chunks_exact(span),
                lo,
                area_start,
                n0,
                n,
                run.first_frame,
                *scale,
                sc,
            );
            for (i, k) in sc.clips.iter().enumerate() {
                stats.clipped[i % ch] += k;
                if let Some(fc) = &mut frame_clipped {
                    fc[f0 + i / ch] += k;
                }
            }
            let at = f0 * spf * ch;
            match &mut out {
                PcmOut::S24le(o) => frames::pack_s24le(&sc.q, &mut o[3 * at..][..3 * n * ch]),
                PcmOut::I32(o) => o[at..][..n * ch].copy_from_slice(&sc.q),
            }
            f0 += nf;
        }
        stats
    }
}

#[cfg(test)]
#[path = "../../tests/pysacd-ref/inputs.rs"]
mod test_inputs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decimator::{with_simd_level, SimdLevel};

    /// The outputs before quantising equal the C reference's bit for bit, at every SIMD
    /// level of this CPU.
    #[test]
    fn floats_match_the_reference_at_every_simd_level() {
        use sha2::{Digest, Sha256};
        for level in SimdLevel::available() {
            with_simd_level(level, || {
                for kind in ["silence", "ff", "music", "random"] {
                    for ch in [2usize, 6] {
                        let area = test_inputs::gen(kind, ch, 5);
                        for rate in [PcmRate::Hz88200, PcmRate::Hz176400] {
                            let p = Pysacd::new(rate, Cutoff::Standard).unwrap();
                            let n = 5 * p.per_frame();
                            let mut sc = Scratch::default();
                            let f: Vec<Vec<f32>> = (0..ch)
                                .map(|c| {
                                    let src: Vec<u8> =
                                        area.iter().skip(c).step_by(ch).copied().collect();
                                    let mut f = vec![0f32; n];
                                    p.render(&src, 0, Some(0), 0, &mut f, &mut sc);
                                    f
                                })
                                .collect();
                            let mut h = Sha256::new();
                            for i in 0..n {
                                for fc in &f {
                                    h.update(fc[i].to_bits().to_le_bytes());
                                }
                            }
                            let name = format!("f32_{kind}_{ch}ch_{}", rate.hz());
                            let got = format!("{:x}", h.finalize());
                            assert_eq!(got, test_inputs::reference(&name), "{name} {level:?}");
                        }
                    }
                }
            })
            .unwrap();
        }
    }

    #[test]
    fn tables_match_the_c_code() {
        let p = Pysacd::new(PcmRate::Hz88200, Cutoff::Standard).unwrap();
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for t in p.lut.iter() {
            for v in t {
                for b in v.to_bits().to_le_bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        assert_eq!(h, taps::LUT_FNV);
    }

    #[test]
    fn delay_matches_the_formula() {
        // dsd2pcm_init: stage 1 centred (96 - 1) / 2 - 7 bits before the end of its byte;
        // each /2 stage adds (taps - 1) / 2 - 1 inputs and halves.
        let d1 = ((96.0 - 1.0) / 2.0 - 7.0) / 8.0;
        let d2: f64 = (d1 + (95.0 - 1.0) / 2.0 - 1.0) / 2.0;
        let d3: f64 = (d2 + (161.0 - 1.0) / 2.0 - 1.0) / 2.0;
        assert_eq!(taps::DELAY, [d3.round() as usize, d2.round() as usize]);
    }

    #[test]
    fn scale_matches_pow() {
        // The factor rounded to f32 equals that of the C library's pow on this machine, at
        // every 0.01 dB step.
        for i in -12_000..=2_400 {
            let db = f64::from(i) / 100.0;
            let c = 8_388_607.0f32 * 10f64.powf(db / 20.0) as f32;
            assert_eq!(scale(db).to_bits(), c.to_bits(), "{db} dB");
        }
    }

    #[test]
    fn rejects_other_rates_and_cutoffs() {
        assert!(Pysacd::new(PcmRate::Hz352800, Cutoff::Standard).is_err());
        assert!(Pysacd::new(PcmRate::Hz176400, Cutoff::Wide).is_err());
        assert!(Pysacd::new(PcmRate::Hz88200, Cutoff::Low).is_err());
    }

    /// Render matches a direct transcription of the C stream on random input, at any
    /// start and length.
    #[test]
    fn render_matches_the_stream() {
        for rate in [PcmRate::Hz88200, PcmRate::Hz176400] {
            let p = Pysacd::new(rate, Cutoff::Standard).unwrap();
            let mut s = 0x1234_5678u32;
            let src: Vec<u8> = (0..3 * 4704).map(|_| xorshift(&mut s) as u8).collect();
            let want = stream(&p, &src);
            let mut sc = Scratch::default();
            for (n0, n) in [
                (0usize, 1usize),
                (0, 77),
                (5, 300),
                (1000, 2000),
                (3000, 528),
            ] {
                let mut got = vec![0f32; n];
                p.render(&src, 0, Some(0), n0 as i64, &mut got, &mut sc);
                let w: Vec<u32> = want[n0..n0 + n].iter().map(|v| v.to_bits()).collect();
                let g: Vec<u32> = got.iter().map(|v| v.to_bits()).collect();
                assert_eq!(g, w, "{rate:?} {n0} {n}");
            }
        }
    }

    /// The C stream, transcribed directly: outputs after the delay.
    fn stream(p: &Pysacd, src: &[u8]) -> Vec<f32> {
        let byte = |i: i64| -> u8 {
            if i < 0 || i >= src.len() as i64 {
                IDLE
            } else {
                src[i as usize]
            }
        };
        let total = src.len() + 4704;
        let t1: Vec<f32> = (0..total as i64)
            .map(|t| {
                let mut acc = 0f32;
                for j in 0..12 {
                    acc += p.lut[j][usize::from(byte(t + j as i64 - 11))];
                }
                acc
            })
            .collect();
        let fir = |h: &[f32], x: &[f32]| -> Vec<f32> {
            (0..x.len() / 2)
                .map(|o| {
                    let mut acc = 0f32;
                    for (k, &hk) in h.iter().enumerate() {
                        let i = 2 * o as i64 + 1 + k as i64 - (h.len() as i64 - 1);
                        let v = if i < 0 { 0.0 } else { x[i as usize] };
                        acc += hk * v;
                    }
                    acc
                })
                .collect()
        };
        let t2 = fir(&p.h2, &t1);
        let out = if p.three { fir(&p.h3, &t2) } else { t2 };
        out[p.delay as usize..].to_vec()
    }
}
