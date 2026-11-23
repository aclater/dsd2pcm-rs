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

mod kernel;
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

use kernel::Sym;
use std::sync::OnceLock;

/// `2^e` from its bits (exact).
fn pow2(e: i32) -> f64 {
    f64::from_bits(((1023 + e) as u64) << 52)
}

/// S1 lookup tables: entry `[j][b]` is the sum of the taps over the 8 samples of byte `j`
/// of the window when that byte is `b` (+tap for a 1 bit, -tap for a 0 bit).
type Lut = Vec<[i32; 256]>;

fn lut(half: &[i32]) -> Lut {
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

static LUT_S1: OnceLock<Lut> = OnceLock::new();
static LUT_S1_WIDE: OnceLock<Lut> = OnceLock::new();

const Q_FIR: i32 = 24;

/// Outputs are computed in whole blocks of this many (the kernels' block).
const PAD: usize = kernel::L;

/// S1 bytes per pass: the scratch buffers stay in the L2 cache.
const PASS_BYTES: usize = 16 << 10;

/// One stage after S1.
#[derive(Clone, Debug)]
enum Op {
    Fir1(Sym),
    Dec2(Sym),
    /// Half-band interpolation by 2, by the half-band's odd taps.
    Interp2(Sym),
}

impl Op {
    /// The input range `(start, len)` that outputs `start .. start + len` need; with `pad`,
    /// including the kernels' extra outputs (whole blocks).
    fn need(&self, (a, n): (i64, usize), pad: bool) -> (i64, usize) {
        let padn = |n: usize| if pad { n.div_ceil(PAD) * PAD } else { n };
        match self {
            Op::Fir1(s) => {
                let m = s.half_len();
                (a - m as i64, padn(n) + 2 * m)
            }
            Op::Dec2(s) => {
                let m = s.half_len();
                (2 * a - m as i64, 2 * padn(n) + 2 * m - 1)
            }
            Op::Interp2(s) => {
                let o = s.odd.len();
                (a / 2 - o as i64, padn(n / 2) + 2 * o)
            }
        }
    }
}

/// Scratch buffers of one worker, reused across calls.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    bytes: Vec<u8>,
    a: Vec<f64>,
    b: Vec<f64>,
    even: Vec<f64>,
    odd: Vec<f64>,
}

/// The stages of one rate and cutoff. Output `o` is centred on DSD byte `o k` (`k` = 4, 2
/// or 1 bytes per output), i.e. on DSD sample `d o`; input bytes outside those given are
/// the idle pattern.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    lut: &'static Lut,
    ops: Vec<Op>,
    /// Bytes per output.
    k: i64,
    /// Outputs per pass (even, a multiple of `PAD`).
    pass: usize,
    /// The bytes that outputs `0` and `1` need: `win.0 .. win.0 + win.1`.
    win: (i64, usize),
}

impl Plan {
    pub(crate) fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Plan> {
        let hb = || Sym::new(&tables::HB, pow2(-Q_FIR));
        let f40 = || Sym::new(&tables::F40, pow2(-Q_FIR));
        let wide = || LUT_S1_WIDE.get_or_init(|| lut(&tables::S1_WIDE));
        let (lut, ops) = match (cutoff, rate) {
            (Cutoff::Wide, PcmRate::Hz88200) => {
                return Err(Error::InvalidArgument(
                    "the wide filter is not available at 88.2 kHz".into(),
                ))
            }
            (Cutoff::Wide, PcmRate::Hz352800) => (wide(), vec![Op::Fir1(f40())]),
            (Cutoff::Wide, PcmRate::Hz176400) => {
                (wide(), vec![Op::Dec2(Sym::new(&tables::F80, pow2(-Q_FIR)))])
            }
            (_, rate) => {
                let f = if cutoff == Cutoff::Low {
                    Sym::new(&tables::F30, pow2(-Q_FIR))
                } else {
                    f40()
                };
                let ops = match rate {
                    PcmRate::Hz88200 => vec![Op::Dec2(hb()), Op::Dec2(f)],
                    PcmRate::Hz176400 => vec![Op::Dec2(hb()), Op::Fir1(f)],
                    PcmRate::Hz352800 => {
                        vec![Op::Dec2(hb()), Op::Fir1(f), Op::Interp2(hb())]
                    }
                };
                (LUT_S1.get_or_init(|| lut(&tables::S1)), ops)
            }
        };
        let k = (rate.decimation() / 8) as i64;
        let pass = (PASS_BYTES / k as usize).div_ceil(PAD) * PAD;
        let mut plan = Plan {
            lut,
            ops,
            k,
            pass,
            win: (0, 0),
        };
        plan.win = plan.byte_range((0, 2), false);
        Ok(plan)
    }

    /// The ranges of every level for outputs `out`: `[0]` is S1's output, the last the
    /// output itself.
    fn levels(&self, out: (i64, usize), pad: bool) -> Vec<(i64, usize)> {
        let k = self.ops.len();
        let mut levels = vec![(0, 0); k + 1];
        levels[k] = out;
        for (i, op) in self.ops.iter().enumerate().rev() {
            levels[i] = op.need(levels[i + 1], pad);
        }
        levels
    }

    /// The bytes that outputs `out` need.
    fn byte_range(&self, out: (i64, usize), pad: bool) -> (i64, usize) {
        let (s, n) = self.levels(out, pad)[0];
        let n = if pad { n.div_ceil(PAD) * PAD } else { n };
        let b = self.lut.len();
        (s - (b / 2) as i64, n + b - 1)
    }

    /// The first byte that output `o` (even) or later outputs need.
    pub(crate) fn first_byte(&self, o: i64) -> i64 {
        o * self.k + self.win.0
    }

    /// The number of outputs from `o` (even) on, even, whose bytes all lie before byte
    /// `end`.
    pub(crate) fn ready(&self, o: i64, end: i64) -> usize {
        let lim = end - (self.win.0 + self.win.1 as i64);
        if lim < o * self.k {
            return 0;
        }
        let last = lim.div_euclid(self.k);
        let last = last - last.rem_euclid(2);
        (last - o + 2).max(0) as usize
    }

    /// Outputs `o0 .. o0 + n` (`o0` and `n` even) on the 2^26 grid, appended to `out`.
    /// `src` holds bytes `src0 ..`; all other bytes are the idle pattern.
    pub(crate) fn run(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        n: usize,
        sc: &mut Scratch,
        out: &mut Vec<f64>,
    ) {
        debug_assert!(o0 % 2 == 0 && n % 2 == 0);
        let mut done = 0;
        while done < n {
            let m = self.pass.min(n - done);
            self.pass_run(src, src0, o0 + done as i64, m, sc);
            out.extend_from_slice(&sc.a[..m]);
            done += m;
        }
    }

    /// One pass: outputs `o0 .. o0 + n` into `sc.a[..n]`.
    fn pass_run(&self, src: &[u8], src0: i64, o0: i64, n: usize, sc: &mut Scratch) {
        let levels = self.levels((o0, n), true);
        let (p0, plen) = self.byte_range((o0, n), true);
        sc.bytes.clear();
        sc.bytes.resize(plen, IDLE);
        let lo = src0.max(p0);
        let hi = (src0 + src.len() as i64).min(p0 + plen as i64);
        if lo < hi {
            sc.bytes[(lo - p0) as usize..(hi - p0) as usize]
                .copy_from_slice(&src[(lo - src0) as usize..(hi - src0) as usize]);
        }
        if !matches!(self.ops.first(), Some(Op::Dec2(_))) {
            let n1 = levels[0].1.div_ceil(PAD) * PAD;
            sc.a.resize(n1, 0.0);
            kernel::s1(self.lut, &sc.bytes, 0, 1, &mut sc.a);
        }
        for (k, op) in self.ops.iter().enumerate() {
            let n_out = levels[k + 1].1;
            match op {
                Op::Fir1(s) => {
                    sc.b.resize(n_out.div_ceil(PAD) * PAD, 0.0);
                    kernel::fir1(s, &sc.a, &mut sc.b);
                }
                Op::Dec2(s) => {
                    let m = s.half_len();
                    let (e, o) = (s.even.len() - 1, s.odd.len());
                    let np = n_out.div_ceil(PAD) * PAD;
                    let (de, dodd) = (m - 2 * e, m + 1 - 2 * o);
                    if k == 0 {
                        sc.even.resize(np + 2 * e, 0.0);
                        sc.odd.resize(np + 2 * o, 0.0);
                        kernel::s1(self.lut, &sc.bytes, de, 2, &mut sc.even);
                        kernel::s1(self.lut, &sc.bytes, dodd, 2, &mut sc.odd);
                    } else {
                        sc.even.clear();
                        sc.odd.clear();
                        sc.even
                            .extend(sc.a.iter().skip(de).step_by(2).take(np + 2 * e));
                        sc.odd
                            .extend(sc.a.iter().skip(dodd).step_by(2).take(np + 2 * o));
                    }
                    sc.b.resize(np, 0.0);
                    kernel::dec2(s, &sc.even, &sc.odd, &mut sc.b);
                }
                Op::Interp2(s) => {
                    let o = s.odd.len();
                    let half = (n_out / 2).div_ceil(PAD) * PAD;
                    sc.even.resize(half, 0.0);
                    kernel::interp2_odd(s, &sc.a, &mut sc.even);
                    sc.b.clear();
                    for i in 0..n_out / 2 {
                        sc.b.push(sc.a[o + i]);
                        sc.b.push(sc.even[i]);
                    }
                }
            }
            std::mem::swap(&mut sc.a, &mut sc.b);
        }
    }
}

/// Streaming DSD64 decimator producing scarletbook-pcm's samples on the 2^26 grid.
#[derive(Debug)]
pub struct Decimator {
    plan: Plan,
    sc: Scratch,
    bytes: Vec<u8>,
    /// Absolute index of `bytes[0]`.
    byte_start: i64,
    /// Next output index.
    out_next: i64,
    rate: PcmRate,
}

impl Decimator {
    /// A decimator for `rate` and `cutoff` (Wide is not available at 88.2 kHz).
    pub fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Decimator> {
        Ok(Decimator {
            plan: Plan::new(rate, cutoff)?,
            sc: Scratch::default(),
            bytes: Vec::with_capacity(1 << 16),
            byte_start: 0,
            out_next: 0,
            rate,
        })
    }

    /// Output rate.
    pub fn rate(&self) -> PcmRate {
        self.rate
    }

    /// Feeds MSB-first DSD64 bytes (pass [`IDLE`] bytes after the end of the input) and
    /// appends the new output samples, on the 2^26 grid, to `out`. Output `n` is centred on
    /// DSD sample `d n` (`d` = 32, 16 or 8); the input is taken to be idle before its start.
    pub fn push(&mut self, input: &[u8], out: &mut Vec<f64>) {
        self.bytes.extend_from_slice(input);
        let end = self.byte_start + self.bytes.len() as i64;
        let n = self.plan.ready(self.out_next, end);
        if n > 0 {
            self.plan.run(
                &self.bytes,
                self.byte_start,
                self.out_next,
                n,
                &mut self.sc,
                out,
            );
            self.out_next += n as i64;
        }
        let keep = self.plan.first_byte(self.out_next);
        let drop = (keep - self.byte_start).clamp(0, self.bytes.len() as i64) as usize;
        self.bytes.drain(..drop);
        self.byte_start += drop as i64;
    }
}

/// Number of PCM samples per channel for `dsd_samples` DSD64 samples: one per started
/// group of `decimation` input samples.
pub fn output_len(dsd_samples: u64, rate: PcmRate) -> u64 {
    dsd_samples.div_ceil(rate.decimation())
}

/// Rounds samples on the 2^26 grid times `gain` to 24 bits (half to even), clamped; appends
/// them as `i32` and returns how many were clamped.
pub fn quantise(v: &[f64], gain: f64, q: &mut Vec<i32>) -> u64 {
    let at = q.len();
    q.resize(at + v.len(), 0);
    kernel::quantise(v, gain, &mut q[at..])
}

/// The scale from the 2^26 grid to full scale 1.0.
pub const GRID_TO_UNIT: f64 = 1.0 / 67_108_864.0;
