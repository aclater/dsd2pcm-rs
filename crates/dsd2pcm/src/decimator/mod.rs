// SPDX-License-Identifier: MIT OR Apache-2.0
//! DSD64 to PCM decimation, ported from scarletbook-pcm 0.7.0 (src/kernel.rs, src/lib.rs).
//!
//! Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR Apache-2.0 (see
//! PROVENANCE.md). The filter tables (`tables.rs`) are copied unchanged; the stage plan,
//! the S1 lookup tables, the rounding rule, the 24-bit quantiser and the gain function are
//! ported. What differs is the driver: scarletbook-pcm converts whole Super Audio CD frames
//! with neighbour frames as context, while this module streams any number of bytes. Both
//! define each output as a function of the infinite input with the idle pattern (0x69)
//! beyond the ends, so the samples are identical (checked against the crate in tests).
//!
//! Arithmetic (as in the original): samples between stages are integers on a grid where
//! full scale (100 % modulation) is 2^26; coefficients are integers; every product and
//! partial sum is an exact integer in `f64`, so summation order cannot change a result.
//! Each stage rounds half to even after scaling back to the 2^26 grid.

mod tables;

use crate::error::{Error, Result};

/// The idle pattern of Super Audio CD: DSD silence, and the input beyond the ends.
pub const IDLE: u8 = 0x69;

/// DSD64 sample rate.
pub const DSD64_RATE: u32 = 2_822_400;

/// PCM output rate of the ported converter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PcmRate {
    /// 88 200 Hz.
    Hz88200,
    /// 176 400 Hz.
    Hz176400,
    /// 352 800 Hz.
    Hz352800,
}

impl PcmRate {
    /// The rate in Hz.
    pub fn hz(self) -> u32 {
        match self {
            PcmRate::Hz88200 => 88_200,
            PcmRate::Hz176400 => 176_400,
            PcmRate::Hz352800 => 352_800,
        }
    }
    /// The rate of `hz`, if supported.
    pub fn from_hz(hz: u32) -> Option<PcmRate> {
        [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800]
            .into_iter()
            .find(|r| r.hz() == hz)
    }
    /// DSD64 samples per output sample (32, 16 or 8).
    pub fn decimation(self) -> u64 {
        u64::from(DSD64_RATE / self.hz())
    }
}

/// Low-pass filter choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cutoff {
    /// Passband to 40 kHz, stopband from 48 kHz.
    #[default]
    Standard,
    /// Passband to 30 kHz, stopband from 36 kHz.
    Low,
    /// Passband to 80 kHz, stopband from 88.2 kHz (176.4 kHz) or 96 kHz (352.8 kHz).
    Wide,
}

/// `10^(db / 20)` with IEEE-754 additions, multiplications and divisions only, in a fixed
/// order (ported unchanged): exactly 1 at 0 dB, monotonic, identical on every platform.
/// Valid for `|db| <= 120`.
pub fn db_to_linear(db: f64) -> f64 {
    let r = db.abs() * (std::f64::consts::LN_10 / 20.0) / 32.0;
    let (mut term, mut sum) = (1.0f64, 1.0f64);
    for k in 1..=25 {
        term = term * r / f64::from(k);
        sum += term;
    }
    for _ in 0..5 {
        sum *= sum;
    }
    if db < 0.0 {
        1.0 / sum
    } else {
        sum
    }
}

/// 1.5 x 2^52: adding and subtracting it rounds to an integer, half to even.
const MAGIC: f64 = 6_755_399_441_055_744.0;

#[inline(always)]
fn round_even(x: f64) -> f64 {
    (x + MAGIC) - MAGIC
}

/// `2^e` from its bits (exact).
fn pow2(e: i32) -> f64 {
    f64::from_bits(((1023 + e) as u64) << 52)
}

/// S1 lookup tables: entry `[j][b]` is the sum of the taps over the 8 samples of byte `j`
/// of the window when that byte is `b` (+tap for a 1 bit, -tap for a 0 bit).
fn lut(half: &[i32]) -> Vec<[i32; 256]> {
    let m = half.len() - 1;
    let bytes = (m + 1) / 4;
    let tap = |t: i64| -> i32 { half.get(t.unsigned_abs() as usize).copied().unwrap_or(0) };
    (0..bytes)
        .map(|j| {
            let rel = j as i64 - (bytes / 2) as i64;
            let mut table = [0i32; 256];
            for (b, entry) in table.iter_mut().enumerate() {
                *entry = (0..8)
                    .map(|p| {
                        let h = tap(8 * rel + p);
                        if (b >> (7 - p)) & 1 == 1 {
                            h
                        } else {
                            -h
                        }
                    })
                    .sum();
            }
            table
        })
        .collect()
}

/// A symmetric filter: taps from the centre out, as `f64`, with the zero taps dropped.
#[derive(Clone, Debug)]
struct Sym {
    h0: f64,
    /// (offset, tap) for offsets >= 1 with a non-zero tap.
    pairs: Vec<(usize, f64)>,
    half_len: usize,
    scale: f64,
}

impl Sym {
    fn new(half: &[i32], q: i32) -> Sym {
        let pairs = half
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, &h)| h != 0)
            .map(|(k, &h)| (k, f64::from(h)))
            .collect();
        Sym {
            h0: f64::from(half[0]),
            pairs,
            half_len: half.len() - 1,
            scale: pow2(-q),
        }
    }
}

const Q_FIR: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fir1,
    Dec2,
    /// Half-band interpolation by 2 by the odd taps.
    Interp2,
}

/// A sample buffer holding `x[start .. start + data.len()]`.
#[derive(Debug, Default)]
struct Buf {
    start: i64,
    data: Vec<f64>,
}

impl Buf {
    fn end(&self) -> i64 {
        self.start + self.data.len() as i64
    }
    fn at(&self, i: i64) -> f64 {
        self.data[(i - self.start) as usize]
    }
    /// Drops samples before `keep_from`.
    fn trim(&mut self, keep_from: i64) {
        let n = (keep_from - self.start).clamp(0, self.data.len() as i64) as usize;
        if n > 0 {
            self.data.drain(..n);
            self.start += n as i64;
        }
    }
}

#[derive(Debug)]
struct Stage {
    kind: Kind,
    sym: Sym,
    input: Buf,
    /// Next output index (pair index for Interp2), once known.
    next: Option<i64>,
}

impl Stage {
    fn new(kind: Kind, sym: Sym) -> Stage {
        Stage {
            kind,
            sym,
            input: Buf::default(),
            next: None,
        }
    }

    /// Appends input `x[start..]` and pushes every computable output to `out`, returning
    /// the index of the first output pushed.
    fn run(&mut self, start: i64, x: &[f64], out: &mut Buf) {
        if self.input.data.is_empty() {
            self.input.start = start;
        }
        self.input.data.extend_from_slice(x);
        let m = self.sym.half_len as i64;
        let s = &self.sym;
        let inb = &self.input;
        match self.kind {
            Kind::Fir1 => {
                let mut i = *self.next.get_or_insert(inb.start + m);
                if out.data.is_empty() {
                    out.start = i;
                }
                while i + m < inb.end() {
                    let mut acc = s.h0 * inb.at(i);
                    for &(k, h) in &s.pairs {
                        let k = k as i64;
                        acc += h * (inb.at(i + k) + inb.at(i - k));
                    }
                    out.data.push(round_even(acc * s.scale));
                    i += 1;
                }
                self.next = Some(i);
                self.input.trim(i - m);
            }
            Kind::Dec2 => {
                let mut i = *self.next.get_or_insert((inb.start + m + 1).div_euclid(2));
                if out.data.is_empty() {
                    out.start = i;
                }
                while 2 * i + m < inb.end() {
                    let c = 2 * i;
                    let mut acc = s.h0 * inb.at(c);
                    for &(k, h) in &s.pairs {
                        let k = k as i64;
                        acc += h * (inb.at(c + k) + inb.at(c - k));
                    }
                    out.data.push(round_even(acc * s.scale));
                    i += 1;
                }
                self.next = Some(i);
                self.input.trim(2 * i - m);
            }
            Kind::Interp2 => {
                // odd taps: offsets 1, 3, 5, ... -> output 2i+1 = sum odd[j] (x[i+1+j] + x[i-j])
                let o = s.pairs.iter().filter(|(k, _)| k % 2 == 1).count() as i64;
                let mut i = *self.next.get_or_insert(inb.start + o - 1);
                if out.data.is_empty() {
                    out.start = 2 * i;
                }
                let sc = s.scale * 2.0;
                while i + o < inb.end() {
                    let mut acc = 0.0f64;
                    for &(k, h) in &s.pairs {
                        if k % 2 == 1 {
                            let j = (k as i64 - 1) / 2;
                            acc += h * (inb.at(i + 1 + j) + inb.at(i - j));
                        }
                    }
                    out.data.push(inb.at(i));
                    out.data.push(round_even(acc * sc));
                    i += 1;
                }
                self.next = Some(i);
                self.input.trim(i - o + 1);
            }
        }
    }
}

/// Streaming DSD64 decimator producing scarletbook-pcm's samples on the 2^26 grid.
#[derive(Debug)]
pub struct Decimator {
    lut: Vec<[i32; 256]>,
    stages: Vec<Stage>,
    bytes: Vec<u8>,
    /// Absolute index of `bytes[0]`.
    byte_start: i64,
    /// Next S1 output index.
    s1_next: i64,
    /// Next final output index to deliver (outputs before 0 are dropped).
    out_next: i64,
    bufs: Vec<Buf>,
    rate: PcmRate,
}

/// Idle bytes placed before the input: more than the context of every plan.
const LEAD: i64 = 4096;

impl Decimator {
    /// A decimator for `rate` and `cutoff` (Wide is not available at 88.2 kHz).
    pub fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Decimator> {
        let hb = || Sym::new(&tables::HB, Q_FIR);
        let f40 = || Sym::new(&tables::F40, Q_FIR);
        let (lut_t, ops): (&[i32], Vec<(Kind, Sym)>) = match (cutoff, rate) {
            (Cutoff::Wide, PcmRate::Hz88200) => {
                return Err(Error::InvalidArgument(
                    "the wide filter is not available at 88.2 kHz".into(),
                ))
            }
            (Cutoff::Wide, PcmRate::Hz352800) => (&tables::S1_WIDE, vec![(Kind::Fir1, f40())]),
            (Cutoff::Wide, PcmRate::Hz176400) => (
                &tables::S1_WIDE,
                vec![(Kind::Dec2, Sym::new(&tables::F80, Q_FIR))],
            ),
            (_, rate) => {
                let f = if cutoff == Cutoff::Low {
                    Sym::new(&tables::F30, Q_FIR)
                } else {
                    f40()
                };
                let ops = match rate {
                    PcmRate::Hz88200 => vec![(Kind::Dec2, hb()), (Kind::Dec2, f)],
                    PcmRate::Hz176400 => vec![(Kind::Dec2, hb()), (Kind::Fir1, f)],
                    PcmRate::Hz352800 => {
                        vec![(Kind::Dec2, hb()), (Kind::Fir1, f), (Kind::Interp2, hb())]
                    }
                };
                (&tables::S1, ops)
            }
        };
        let lut = lut(lut_t);
        let b2 = (lut.len() / 2) as i64;
        let n = ops.len();
        let mut d = Decimator {
            lut,
            stages: ops.into_iter().map(|(k, s)| Stage::new(k, s)).collect(),
            bytes: vec![IDLE; LEAD as usize],
            byte_start: -LEAD,
            s1_next: -LEAD + b2,
            out_next: 0,
            bufs: (0..=n).map(|_| Buf::default()).collect(),
            rate,
        };
        d.bytes.reserve(1 << 16);
        Ok(d)
    }

    /// Output rate.
    pub fn rate(&self) -> PcmRate {
        self.rate
    }

    /// Feeds MSB-first DSD64 bytes (pass [`IDLE`] bytes after the end of the input) and
    /// appends the new output samples, on the 2^26 grid, to `out`.
    pub fn push(&mut self, input: &[u8], out: &mut Vec<f64>) {
        self.bytes.extend_from_slice(input);
        let b = self.lut.len();
        let b2 = (b / 2) as i64;
        // S1 into bufs[0]
        let mut s1 = std::mem::take(&mut self.bufs[0]);
        s1.data.clear();
        s1.start = self.s1_next;
        let byte_end = self.byte_start + self.bytes.len() as i64;
        let mut m = self.s1_next;
        while m - b2 + b as i64 <= byte_end {
            let off = (m - b2 - self.byte_start) as usize;
            let w = &self.bytes[off..off + b];
            let mut sum = 0i32;
            for (t, &x) in self.lut.iter().zip(w) {
                sum = sum.wrapping_add(t[usize::from(x)]);
            }
            s1.data.push(f64::from(sum));
            m += 1;
        }
        self.s1_next = m;
        let drop = (m - b2 - self.byte_start).max(0) as usize;
        self.bytes.drain(..drop.min(self.bytes.len()));
        self.byte_start += drop as i64;
        // stages
        let mut cur = s1;
        for (k, st) in self.stages.iter_mut().enumerate() {
            let mut nb = std::mem::take(&mut self.bufs[k + 1]);
            nb.data.clear();
            if !cur.data.is_empty() {
                st.run(cur.start, &cur.data, &mut nb);
            }
            self.bufs[k] = cur;
            cur = nb;
        }
        // deliver outputs with index >= 0
        if !cur.data.is_empty() {
            let first = cur.start;
            let skip = (self.out_next - first).max(0) as usize;
            if skip < cur.data.len() {
                out.extend_from_slice(&cur.data[skip..]);
                self.out_next = first + cur.data.len() as i64;
            }
        }
        let n = self.stages.len();
        self.bufs[n] = cur;
    }
}

/// Number of PCM samples per channel for `dsd_samples` DSD64 samples: one per started
/// group of `decimation` input samples.
pub fn output_len(dsd_samples: u64, rate: PcmRate) -> u64 {
    dsd_samples.div_ceil(rate.decimation())
}

/// Rounds samples on the 2^26 grid times `gain` to 24 bits (half to even), clamped; writes
/// them as `i32` and returns how many were clamped. Ported unchanged.
pub fn quantise(v: &[f64], gain: f64, q: &mut Vec<i32>) -> u64 {
    const MAX: f64 = 8_388_607.0;
    const MIN: f64 = -8_388_608.0;
    let mut clipped = 0u64;
    for &v in v {
        let y = round_even((v * gain) * 0.125);
        let c = y.clamp(MIN, MAX);
        clipped += u64::from(c != y);
        q.push(c as i32);
    }
    clipped
}

/// The scale from the 2^26 grid to full scale 1.0.
pub const GRID_TO_UNIT: f64 = 1.0 / 67_108_864.0;
