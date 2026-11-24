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
//! Each stage rounds half to even after scaling back to the 2^26 grid (`exact` proves the
//! bounds from the tables).

mod dispatch;
mod exact;
mod kernel;
mod lanes;
mod tables;
mod tables_precise;

pub(crate) use dispatch::Isa;
#[doc(hidden)]
pub use dispatch::{with_simd_level, SimdLevel};

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

/// The filter taps: scarletbook-pcm 0.7.0's, or the `precise` filter's
/// (`crate::Filter::Precise`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Design {
    /// scarletbook-pcm 0.7.0's tables (`tables`).
    #[default]
    Default,
    /// The precise filter's tables (`tables_precise`): S1 with stopbands of 150 dB where
    /// the input folds into the passband, and every later stage as a cascade of two filters
    /// of about 80 dB each.
    Precise,
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

/// The S1 tables of a filter given from the centre out. The window covers the bytes
/// `-B/2 .. B/2` around the output's byte (`B` = 4 times the half length, in bytes), and the
/// output is centred on the first sample of its byte.
fn lut(half: &[i32], layout: S1Layout) -> Lut {
    let bytes = half.len() / 4;
    let centre = 8 * (bytes / 2);
    Lut::new(bytes, layout, |i| {
        half.get(i.abs_diff(centre)).copied().unwrap_or(0)
    })
}

type Lut = kernel::S1Lut;
use kernel::S1Layout;

/// [`lut`] as paired tables (`lanes`).
fn pair(half: &[i32]) -> lanes::S1Pair {
    let bytes = half.len() / 4;
    let centre = 8 * (bytes / 2);
    lanes::S1Pair::new(bytes, |i| {
        half.get(i.abs_diff(centre)).copied().unwrap_or(0)
    })
}

/// The first stages: S1 of the 30 and 40 kHz filters and of the wide one, of each
/// [`Design`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S1Kind {
    Default,
    DefaultWide,
    Precise,
    PreciseWide,
}

impl S1Kind {
    #[cfg(test)]
    const ALL: [S1Kind; 4] = [
        S1Kind::Default,
        S1Kind::DefaultWide,
        S1Kind::Precise,
        S1Kind::PreciseWide,
    ];

    fn of(design: Design, cutoff: Cutoff) -> S1Kind {
        match (design, cutoff == Cutoff::Wide) {
            (Design::Default, false) => S1Kind::Default,
            (Design::Default, true) => S1Kind::DefaultWide,
            (Design::Precise, false) => S1Kind::Precise,
            (Design::Precise, true) => S1Kind::PreciseWide,
        }
    }

    /// The taps, from the centre out.
    fn half(self) -> &'static [i32] {
        match self {
            S1Kind::Default => &tables::S1,
            S1Kind::DefaultWide => &tables::S1_WIDE,
            S1Kind::Precise => &tables_precise::S1,
            S1Kind::PreciseWide => &tables_precise::S1_WIDE,
        }
    }

    /// Its paired tables (`lanes`), built once.
    fn pair(self) -> &'static lanes::S1Pair {
        static PAIRS: [OnceLock<lanes::S1Pair>; 4] = [const { OnceLock::new() }; 4];
        PAIRS[self as usize].get_or_init(|| pair(self.half()))
    }

    /// Its tables in `layout`, built once.
    fn lut(self, layout: S1Layout) -> &'static Lut {
        static LUTS: [[OnceLock<Lut>; 2]; 4] = [const { [const { OnceLock::new() }; 2] }; 4];
        LUTS[self as usize][layout as usize].get_or_init(|| lut(self.half(), layout))
    }
}

/// S1's tables in a layout, built on first use: the level with paired tables never needs
/// them, so a converter there does not build them.
#[derive(Clone, Copy, Debug)]
struct S1Src {
    kind: S1Kind,
    layout: S1Layout,
    /// Bytes per window.
    bytes: usize,
}

impl S1Src {
    fn new(kind: S1Kind, layout: S1Layout) -> S1Src {
        S1Src {
            kind,
            layout,
            bytes: kind.half().len() / 4,
        }
    }

    /// The tables.
    fn get(self) -> &'static Lut {
        self.kind.lut(self.layout)
    }
}

const Q_FIR: i32 = 24;

/// The grid of the samples between stages and of the output: 100 % modulation is 2^26.
const GRID: i32 = 26;

/// Outputs are computed in whole blocks of this many (a multiple of every kernel's block).
const PAD: usize = kernel::MAX_L;

/// S1 bytes per pass: the scratch buffers stay in the L2 cache.
const PASS_BYTES: usize = 32 << 10;

/// The fewest outputs a pass runs in lanes: each run needs its own filter margins, which
/// for short passes (one Super Audio CD frame: 1176 outputs at 88.2 kHz, 2352 at 176.4
/// kHz) cost more than the lanes gain. Shorter passes use the ordered kernels, with the
/// paired S1 tables and the channel's [`Carry`] where the plan has them.
const LANES_MIN: usize = 128 * PAD;

/// [`LANES_MIN`] at the `sse2` level, where the ordered kernels pay more against the lanes
/// (unaligned 128-bit loads; on 32-bit x86 also 8 vector registers and no paired tables or
/// carry): one frame at 88.2 kHz runs faster in lanes there; the wide filter, whose 80 kHz
/// stage has the longest margins, from 2048 outputs (one frame at 176.4 kHz).
const LANES_MIN_SSE2: [usize; 2] = [32 * PAD, 64 * PAD];

/// Most stages after S1 in any plan.
const MAX_OPS: usize = 6;

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

/// The S1 values of one channel's previous ordered pass with paired tables, for the next
/// pass to reuse where it continues it (frame runs converted a frame at a time). A value is
/// reused only if every byte of its window is the same in both passes (compared, not
/// assumed): S1 is a function of those bytes alone, so the output cannot change. Only the
/// bytes a following pass can share are kept, so a pass that is not followed by its
/// neighbour (pooled converters) costs a copy of a few hundred bytes.
#[derive(Debug, Default)]
pub(crate) struct Carry {
    /// The previous pass's bytes from where the next adjacent pass starts to its end; the
    /// first at byte `b0` of its timeline.
    bytes: Vec<u8>,
    b0: i64,
    /// Its S1 values: split (`ev[i]` over the window from byte `s0 + 2 i`, `od[i]` from
    /// `s0 + 2 i + 1`) or not (`ev[i]` from `s0 + i`).
    ev: Vec<f64>,
    od: Vec<f64>,
    s0: i64,
    split: bool,
    valid: bool,
    reserved: bool,
    /// Byte `x` of the next pass is byte `x + shift` of the previous pass's timeline; set
    /// by the caller before each call.
    pub(crate) shift: i64,
}

impl Carry {
    /// Copies the reusable S1 values of a pass over `bytes` (the first at byte `p0`, windows
    /// of `win` bytes) into the starts of `ev` (and `od` if `split`) and returns how many
    /// (pairs if split; an even number if not).
    fn reuse(
        &self,
        bytes: &[u8],
        p0: i64,
        win: usize,
        split: bool,
        ev: &mut [f64],
        od: &mut [f64],
    ) -> usize {
        // The previous pass's kept bytes and S1 values, on this pass's timeline.
        let (b0, s0) = (self.b0 - self.shift, self.s0 - self.shift);
        if !self.valid || self.split != split || b0 > p0 || s0 > p0 {
            return 0;
        }
        let Some(prev) = self.bytes.get((p0 - b0) as usize..) else {
            return 0;
        };
        // The bytes both passes share, from `p0` on: a quick look at the first one first,
        // which fails at once for a pass that does not continue the previous one.
        let n = prev.len().min(bytes.len());
        if n < win || prev[0] != bytes[0] {
            return 0;
        }
        let eq = if prev[..n] == bytes[..n] {
            n
        } else {
            prev.iter().zip(bytes).take_while(|(x, y)| x == y).count()
        };
        let off = (p0 - s0) as usize;
        if split {
            // Pair `i` covers windows from bytes `2 i` and `2 i + 1`.
            if off % 2 != 0 || eq < win + 1 {
                return 0;
            }
            let u = ((eq - win - 1) / 2 + 1)
                .min(self.ev.len().saturating_sub(off / 2))
                .min(ev.len());
            ev[..u].copy_from_slice(&self.ev[off / 2..][..u]);
            od[..u].copy_from_slice(&self.od[off / 2..][..u]);
            u
        } else {
            if eq < win {
                return 0;
            }
            let u = (eq - win + 1)
                .min(self.ev.len().saturating_sub(off))
                .min(ev.len())
                / 2
                * 2;
            ev[..u].copy_from_slice(&self.ev[off..][..u]);
            u
        }
    }

    /// Keeps a pass's S1 values (windows from byte `p0` on, swapping the buffers) and its
    /// bytes from `next` on (`bytes` starts at `p0`; `next` is where the next adjacent pass
    /// starts: the S1 values it can reuse lie past it).
    fn keep(
        &mut self,
        bytes: &[u8],
        p0: i64,
        next: usize,
        split: bool,
        ev: &mut Vec<f64>,
        od: &mut Vec<f64>,
    ) {
        let Some(tail) = bytes.get(next..).filter(|t| !t.is_empty()) else {
            self.valid = false;
            return;
        };
        self.bytes.clear();
        self.bytes.extend_from_slice(tail);
        self.b0 = p0 + next as i64;
        self.s0 = p0;
        self.split = split;
        std::mem::swap(&mut self.ev, ev);
        if split {
            std::mem::swap(&mut self.od, od);
        }
        self.valid = true;
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
    lanes: lanes::Bufs,
    /// Whether the ordered buffers have been sized for every pass below the lane threshold.
    ordered_sized: bool,
}

/// The stages of one rate and cutoff. Output `o` is centred on DSD byte `o k` (`k` = 4, 2
/// or 1 bytes per output), i.e. on DSD sample `d o`; input bytes outside those given are
/// the idle pattern.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    lut: S1Src,
    isa: Isa,
    ops: Vec<Op>,
    /// Bytes per output.
    k: i64,
    /// Outputs per pass (even, a multiple of `PAD`).
    pass: usize,
    /// The bytes that outputs `0` and `1` need: `win.0 .. win.0 + win.1`.
    win: (i64, usize),
    /// Whether passes run in lanes (`lanes`): every stage proven exact and taken, on a
    /// level that runs them; with S1's paired tables if the level uses them (and then
    /// also in the ordered passes, with the channel's [`Carry`]).
    lanes: Option<Option<&'static lanes::S1Pair>>,
    /// The fewest outputs a pass runs in lanes.
    lanes_min: usize,
}

/// scarletbook-pcm 0.7.0's stages after S1 for `rate` and `cutoff`.
fn default_ops(rate: PcmRate, cutoff: Cutoff) -> Vec<Op> {
    let sym = |t: &[i32]| Sym::new(t, pow2(-Q_FIR));
    let hb = || sym(&tables::HB);
    match (cutoff, rate) {
        (Cutoff::Wide, PcmRate::Hz352800) => vec![Op::Fir1(sym(&tables::F40))],
        (Cutoff::Wide, _) => vec![Op::Dec2(sym(&tables::F80))],
        (_, rate) => {
            let f = sym(if cutoff == Cutoff::Low {
                &tables::F30
            } else {
                &tables::F40
            });
            match rate {
                PcmRate::Hz88200 => vec![Op::Dec2(hb()), Op::Dec2(f)],
                PcmRate::Hz176400 => vec![Op::Dec2(hb()), Op::Fir1(f)],
                PcmRate::Hz352800 => vec![Op::Dec2(hb()), Op::Fir1(f), Op::Interp2(hb())],
            }
        }
    }
}

/// The precise filter's stages after S1 for `rate` and `cutoff` (see `tables_precise`):
/// the prefilter `P` takes S1's finer grid back to 2^26, and every sharp filter runs twice,
/// so that each stopband is the sum of two in dB.
fn precise_ops(rate: PcmRate, cutoff: Cutoff) -> Vec<Op> {
    use tables_precise as t;
    // A stage with taps `h` (summing to 2^q) on samples on the grid 2^`grid`, rounded to
    // the 2^26 grid.
    let sym = |h: &[i32], q: i32, grid: i32| Sym::new(h, pow2(-(q + grid - GRID)));
    let hb = || sym(&t::HB, t::Q_HB, GRID);
    match (cutoff, rate) {
        (Cutoff::Wide, PcmRate::Hz352800) => {
            let s1 = t::Q_S1_WIDE;
            vec![
                Op::Fir1(sym(&t::F40, t::Q_F40, s1)),
                Op::Fir1(sym(&t::F40, t::Q_F40, GRID)),
            ]
        }
        (Cutoff::Wide, _) => {
            let s1 = t::Q_S1_WIDE;
            vec![
                Op::Fir1(sym(&t::F80, t::Q_F80, s1)),
                Op::Dec2(sym(&t::F80, t::Q_F80, GRID)),
            ]
        }
        (_, rate) => {
            let f = || {
                if cutoff == Cutoff::Low {
                    sym(&t::F30, t::Q_F30, GRID)
                } else {
                    sym(&t::F40, t::Q_F40, GRID)
                }
            };
            let mut ops = vec![
                Op::Fir1(sym(&t::P, t::Q_P, t::Q_S1)),
                Op::Dec2(hb()),
                Op::Fir1(f()),
            ];
            match rate {
                PcmRate::Hz88200 => ops.push(Op::Dec2(f())),
                PcmRate::Hz176400 => ops.push(Op::Fir1(f())),
                PcmRate::Hz352800 => ops.extend([
                    Op::Fir1(f()),
                    Op::Interp2(hb()),
                    Op::Fir1(sym(&t::P, t::Q_P, GRID)),
                ]),
            }
            ops
        }
    }
}

impl Plan {
    pub(crate) fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Plan> {
        Plan::with_design(rate, cutoff, Design::Default)
    }

    pub(crate) fn with_design(rate: PcmRate, cutoff: Cutoff, design: Design) -> Result<Plan> {
        if (cutoff, rate) == (Cutoff::Wide, PcmRate::Hz88200) {
            return Err(Error::InvalidArgument(
                "the wide filter is not available at 88.2 kHz".into(),
            ));
        }
        let isa = Isa::current();
        let kind = S1Kind::of(design, cutoff);
        let lut = S1Src::new(kind, dispatch::s1_layout(isa));
        let mut ops = match design {
            Design::Default => default_ops(rate, cutoff),
            Design::Precise => precise_ops(rate, cutoff),
        };
        debug_assert!(ops.len() <= MAX_OPS);
        // An interpolation's outputs start at an even index (`Op::need`): a stage after it
        // has an even half length.
        debug_assert!(ops.windows(2).all(|w| match w {
            [Op::Interp2(_), Op::Fir1(s)] => s.half_len() % 2 == 0,
            [Op::Interp2(_), _] => false,
            _ => true,
        }));
        exact::prove(kind.half(), &mut ops);
        let k = (rate.decimation() / 8) as i64;
        let pass = (PASS_BYTES / k as usize).div_ceil(PAD) * PAD;
        let lanes = (dispatch::lanes_level(isa)
            && ops.iter().all(|op| match op {
                Op::Fir1(s) | Op::Dec2(s) | Op::Interp2(s) => lanes::takes(s),
            }))
        .then(|| dispatch::lanes_pair().then(|| kind.pair()));
        let mut plan = Plan {
            lut,
            isa,
            ops,
            k,
            pass,
            win: (0, 0),
            lanes,
            lanes_min: if dispatch::short_lanes(isa) {
                LANES_MIN_SSE2[usize::from(cutoff == Cutoff::Wide)]
            } else {
                LANES_MIN
            },
        };
        plan.win = plan.byte_range((0, 2), false);
        Ok(plan)
    }

    /// The ranges of every level for outputs `out`: `[0]` is S1's output, `[ops.len()]` the
    /// output itself (a fixed array: no allocation per pass).
    fn levels(&self, out: (i64, usize), pad: bool) -> [(i64, usize); MAX_OPS + 1] {
        let k = self.ops.len();
        let mut levels = [(0, 0); MAX_OPS + 1];
        levels[k] = out;
        for (i, op) in self.ops.iter().enumerate().rev() {
            levels[i] = op.need(levels[i + 1], pad);
        }
        levels
    }

    /// The kernels' instruction set.
    pub(crate) fn isa(&self) -> Isa {
        self.isa
    }

    /// The bytes that outputs `out` need.
    fn byte_range(&self, out: (i64, usize), pad: bool) -> (i64, usize) {
        let (s, n) = self.levels(out, pad)[0];
        let n = if pad { n.div_ceil(PAD) * PAD } else { n };
        let b = self.lut.bytes;
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

    /// Outputs `o0 .. o0 + q.len()` (`o0` even), times `gain` and rounded to 24 bits, into
    /// `q`; returns the number clamped. `src` holds bytes `src0 ..`; all other bytes are the
    /// idle pattern.
    pub(crate) fn run_quantised(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        gain: f64,
        sc: &mut Scratch,
        q: &mut [i32],
    ) -> u64 {
        let mut clipped = 0;
        self.run_quantised_segs(src, src0, o0, gain, sc, q, usize::MAX, |_, n| clipped += n);
        clipped
    }

    /// As [`run_quantised`](Self::run_quantised), but counts the clamped samples per segment
    /// of `seg` outputs of `q`: `clip(i, n)` adds `n` to segment `i` (called at least once
    /// per pass and segment, possibly with 0).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_quantised_segs(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        gain: f64,
        sc: &mut Scratch,
        q: &mut [i32],
        seg: usize,
        clip: impl FnMut(usize, u64),
    ) {
        self.run_quantised_carry(src, src0, o0, gain, sc, q, seg, clip, None)
    }

    /// As [`run_quantised_segs`](Self::run_quantised_segs), reusing S1 values of the
    /// channel's previous pass (`carry`) where it can.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_quantised_carry(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        gain: f64,
        sc: &mut Scratch,
        q: &mut [i32],
        seg: usize,
        mut clip: impl FnMut(usize, u64),
        mut carry: Option<&mut Carry>,
    ) {
        debug_assert!(o0 % 2 == 0 && seg > 0);
        for (i, q) in q.chunks_mut(self.pass).enumerate() {
            let m = q.len().next_multiple_of(2);
            let at = i * self.pass;
            self.pass_carry(src, src0, o0 + at as i64, m, sc, carry.as_deref_mut());
            if let Some(c) = carry.as_deref_mut() {
                // Later passes continue on the same timeline.
                c.shift = 0;
            }
            let mut done = 0;
            while done < q.len() {
                let s = (at + done) / seg;
                let end = ((s + 1).saturating_mul(seg) - at).min(q.len());
                let n = dispatch::quantise(self.isa, &sc.a[done..end], gain, &mut q[done..end]);
                clip(s, n);
                done = end;
            }
        }
    }

    /// The bytes that [`run_quantised`](Self::run_quantised) reads for outputs
    /// `o0 .. o0 + n` (`o0` even), padding included: `(first, len)`. Bytes outside the
    /// unpadded window of each output only reach outputs that are discarded.
    pub(crate) fn span(&self, o0: i64, n: usize) -> (i64, usize) {
        let mut lo = i64::MAX;
        let mut hi = i64::MIN;
        let mut done = 0;
        while done < n {
            let m = self.pass.min(n - done);
            let (s, len) = self.byte_range((o0 + done as i64, m.next_multiple_of(2)), true);
            lo = lo.min(s);
            hi = hi.max(s + len as i64);
            done += m;
        }
        if lo > hi {
            return (o0 * self.k, 0);
        }
        (lo, (hi - lo) as usize)
    }

    /// Bytes per output (4, 2 or 1).
    pub(crate) fn bytes_per_output(&self) -> i64 {
        self.k
    }

    /// The bytes that outputs `0` and `1` need, unpadded: `(first, len)`.
    pub(crate) fn window(&self) -> (i64, usize) {
        self.win
    }

    /// One pass: outputs `o0 .. o0 + n` into `sc.a[..n]`.
    fn pass_run(&self, src: &[u8], src0: i64, o0: i64, n: usize, sc: &mut Scratch) {
        self.pass_carry(src, src0, o0, n, sc, None)
    }

    /// Sizes `c` for every pass that can use it, so that later passes allocate nothing.
    pub(crate) fn reserve(&self, c: &mut Carry) {
        if !matches!(self.lanes, Some(Some(_))) || c.reserved {
            return;
        }
        c.reserved = true;
        let n = self.lanes_min - 2;
        let (_, plen) = self.byte_range((0, n), true);
        let s1 = self.levels((0, n), true)[0].1.div_ceil(PAD) * PAD;
        c.bytes.reserve(plen);
        c.ev.reserve(s1);
        c.od.reserve(s1);
    }

    /// [`pass_run`](Self::pass_run) with the channel's [`Carry`].
    fn pass_carry(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        n: usize,
        sc: &mut Scratch,
        carry: Option<&mut Carry>,
    ) {
        if let Some(pair) = self.lanes.filter(|_| n >= self.lanes_min) {
            if let Some(c) = carry {
                c.valid = false;
            }
            if !sc.ordered_sized {
                // Size the ordered buffers too, for every shorter pass, so that a caller
                // that warmed up with a long pass allocates nothing later. S1's level is the
                // longest of an ordered pass, so it bounds every buffer of one.
                sc.ordered_sized = true;
                let n = self.lanes_min - 2;
                let longest = self.levels((0, n), true)[0].1.div_ceil(PAD) * PAD;
                for v in [&mut sc.a, &mut sc.b, &mut sc.even, &mut sc.odd] {
                    v.reserve(longest.saturating_sub(v.len()));
                }
                let plen = self.byte_range((0, n), true).1;
                sc.bytes.reserve(plen.saturating_sub(sc.bytes.len()));
            }
            return self.pass_lanes(pair, src, src0, o0, n, sc);
        }
        self.pass_ordered(src, src0, o0, n, sc, carry);
    }

    /// [`pass_run`](Self::pass_run) with the ordered kernels.
    fn pass_ordered(
        &self,
        src: &[u8],
        src0: i64,
        o0: i64,
        n: usize,
        sc: &mut Scratch,
        mut carry: Option<&mut Carry>,
    ) {
        // The carry holds S1 values of the paired tables only.
        if !matches!(self.lanes, Some(Some(_))) {
            if let Some(c) = carry.take() {
                c.valid = false;
            }
        }
        let Scratch {
            bytes: buf,
            a,
            b,
            even,
            odd,
            ..
        } = sc;
        let levels = self.levels((o0, n), true);
        let (p0, plen) = self.byte_range((o0, n), true);
        // Where the next adjacent pass (outputs from `o0 + n`) starts in `bytes`.
        let next = (self.byte_range((o0 + n as i64, n), true).0 - p0).max(0) as usize;
        let bytes = fetch(src, src0, p0, plen, buf);
        // The buffers the kernels write are resized, not cleared: they write every element.
        if !matches!(self.ops.first(), Some(Op::Dec2(_))) {
            let n1 = levels[0].1.div_ceil(PAD) * PAD;
            a.resize(n1, 0.0);
            match self.lanes {
                Some(Some(pair)) => {
                    let from = carry
                        .as_deref()
                        .map_or(0, |c| c.reuse(bytes, p0, self.lut.bytes, false, a, &mut []));
                    pair.run(self.lut, &bytes[from..], &mut lanes::Flat(&mut a[from..]));
                }
                _ => dispatch::s1(self.isa, self.lut.get(), bytes, 0, 1, a),
            }
        }
        for (k, op) in self.ops.iter().enumerate() {
            let n_out = levels[k + 1].1;
            match op {
                Op::Fir1(s) => {
                    b.resize(n_out.div_ceil(PAD) * PAD, 0.0);
                    dispatch::fir1(self.isa, s, a, b);
                    if let (0, Some(c)) = (k, carry.as_deref_mut()) {
                        c.keep(bytes, p0, next, false, a, &mut Vec::new());
                    }
                }
                Op::Dec2(s) => {
                    let m = s.half_len();
                    let (e, o) = (s.even.len() - 1, s.odd.len());
                    let np = n_out.div_ceil(PAD) * PAD;
                    let (de, dodd) = (m - 2 * e, m + 1 - 2 * o);
                    b.resize(np, 0.0);
                    if let (0, Some(Some(pair))) = (k, self.lanes) {
                        // S1 at even and odd bytes in one set of lookups per pair, as two
                        // streams that the stage reads from its offsets.
                        let half = levels[0].1.div_ceil(PAD) * PAD / 2;
                        even.resize(half, 0.0);
                        odd.resize(half, 0.0);
                        let from = carry
                            .as_deref()
                            .map_or(0, |c| c.reuse(bytes, p0, self.lut.bytes, true, even, odd));
                        pair.run(
                            self.lut,
                            &bytes[2 * from..],
                            &mut lanes::Split(&mut even[from..], &mut odd[from..]),
                        );
                        let (xe, xo) = lanes::split(even, odd, de, dodd);
                        dispatch::dec2(self.isa, s, xe, xo, b);
                        if let Some(c) = carry.as_deref_mut() {
                            c.keep(bytes, p0, next, true, even, odd);
                        }
                    } else {
                        even.resize(np + 2 * e, 0.0);
                        odd.resize(np + 2 * o, 0.0);
                        if k == 0 {
                            dispatch::s1(self.isa, self.lut.get(), bytes, de, 2, even);
                            dispatch::s1(self.isa, self.lut.get(), bytes, dodd, 2, odd);
                        } else {
                            every_other(&a[de..], even);
                            every_other(&a[dodd..], odd);
                        }
                        dispatch::dec2(self.isa, s, even, odd, b);
                    }
                }
                Op::Interp2(s) => {
                    let o = s.odd.len();
                    let half = (n_out / 2).div_ceil(PAD) * PAD;
                    even.resize(half, 0.0);
                    dispatch::interp2_odd(self.isa, s, a, even);
                    b.resize(n_out, 0.0);
                    for ((d, &x), &y) in b[..n_out]
                        .chunks_exact_mut(2)
                        .zip(&a[o..o + n_out / 2])
                        .zip(&*even)
                    {
                        d[0] = x;
                        d[1] = y;
                    }
                }
            }
            std::mem::swap(a, b);
        }
    }

    /// [`pass_run`](Self::pass_run) in lanes: `lanes::W` runs of `q` outputs (`n / W`
    /// rounded up to whole blocks), the last ones moved back to end with the padded pass
    /// (runs may overlap; they give the same outputs where they do), each computed as
    /// `pass_run` would.
    fn pass_lanes(
        &self,
        pair: Option<&lanes::S1Pair>,
        src: &[u8],
        src0: i64,
        o0: i64,
        n: usize,
        sc: &mut Scratch,
    ) {
        let n_pad = n.div_ceil(PAD) * PAD;
        let q = n.div_ceil(lanes::W).div_ceil(PAD) * PAD;
        let runs: [usize; lanes::W] = std::array::from_fn(|w| (w * q).min(n_pad - q));
        let levels = self.levels((o0, q), true);
        let mut len = [0usize; MAX_OPS + 1];
        for (l, v) in len.iter_mut().zip(&levels) {
            *l = v.1;
        }
        // The bytes of each run; every run's level lengths are the same.
        let ranges = runs.map(|r| self.byte_range((o0 + r as i64, q), true));
        let b0 = ranges[0].0;
        let starts = ranges.map(|(b, _)| (b - b0) as usize);
        let plen = starts[lanes::W - 1] + ranges[lanes::W - 1].1;
        let bytes = fetch(src, src0, b0, plen, &mut sc.bytes);
        let pass = lanes::Pass {
            lut: self.lut,
            pair,
            ops: &self.ops,
            bytes,
            starts,
            lens: &len[..=self.ops.len()],
        };
        let out = dispatch::lanes(self.isa, &pass, PAD, &mut sc.lanes);
        sc.a.resize(n, 0.0);
        for (w, &r) in runs.iter().enumerate() {
            let end = (r + q).min(n);
            for (d, v) in sc.a[r..end].iter_mut().zip(out) {
                *d = v.0[w];
            }
        }
    }
}

/// The bytes `p0 .. p0 + plen`: straight from `src` (bytes `src0 ..`) when it holds them
/// all, else copied into `buf`, with the idle pattern beyond `src`.
fn fetch<'a>(src: &'a [u8], src0: i64, p0: i64, plen: usize, buf: &'a mut Vec<u8>) -> &'a [u8] {
    if src0 <= p0 && p0 + plen as i64 <= src0 + src.len() as i64 {
        &src[(p0 - src0) as usize..][..plen]
    } else {
        buf.clear();
        buf.resize(plen, IDLE);
        let lo = src0.max(p0);
        let hi = (src0 + src.len() as i64).min(p0 + plen as i64);
        if lo < hi {
            buf[(lo - p0) as usize..(hi - p0) as usize]
                .copy_from_slice(&src[(lo - src0) as usize..(hi - src0) as usize]);
        }
        buf
    }
}

/// `dst[i] = src[2 i]`.
fn every_other(src: &[f64], dst: &mut [f64]) {
    let Some(last) = dst.len().checked_sub(1) else {
        return;
    };
    let src = &src[..=2 * last];
    for (i, d) in dst.iter_mut().enumerate() {
        *d = src[2 * i];
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
    dispatch::quantise(Isa::current(), v, gain, &mut q[at..])
}

/// Interleaves the channels `q` (equal lengths) as 24-bit little-endian samples into `out`
/// (3 bytes per sample).
pub(crate) fn pack24(isa: Isa, q: &[&[i32]], out: &mut [u8]) {
    dispatch::pack24(isa, q, out)
}

/// The scale from the 2^26 grid to full scale 1.0.
pub const GRID_TO_UNIT: f64 = 1.0 / 67_108_864.0;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(PcmRate, Cutoff); 8] = [
        (PcmRate::Hz88200, Cutoff::Low),
        (PcmRate::Hz88200, Cutoff::Standard),
        (PcmRate::Hz176400, Cutoff::Low),
        (PcmRate::Hz176400, Cutoff::Standard),
        (PcmRate::Hz176400, Cutoff::Wide),
        (PcmRate::Hz352800, Cutoff::Low),
        (PcmRate::Hz352800, Cutoff::Standard),
        (PcmRate::Hz352800, Cutoff::Wide),
    ];

    const DESIGNS: [Design; 2] = [Design::Default, Design::Precise];

    fn sym(op: &Op) -> &Sym {
        match op {
            Op::Fir1(s) | Op::Dec2(s) | Op::Interp2(s) => s,
        }
    }

    /// The exactness proof holds for every stage of every plan, derived from the tables the
    /// plans use: S1 sums stay below 2^31 (they are summed in `i32`), and every product and
    /// partial sum of every later stage is an integer below 2^53. So summation order and
    /// block size cannot change a result, and `fused` (FMA) is on for every stage. If a
    /// table or the stage plan changes, `exact::prove` re-derives the bounds; a stage it
    /// cannot prove runs unfused, and this test fails so that the change is noticed.
    #[test]
    fn sums_are_exact() {
        for (design, (rate, cutoff)) in DESIGNS.iter().flat_map(|&d| ALL.map(|p| (d, p))) {
            let plan = Plan::with_design(rate, cutoff, design).unwrap();
            let half = S1Kind::of(design, cutoff).half();
            let s1 = exact::s1_bound(half);
            assert!(s1 < i32::MAX as u128, "S1 bound {s1}");
            // The tables themselves stay within the bound, in both layouts.
            for layout in [S1Layout::Wide, S1Layout::Byte] {
                let worst: u128 = lut(half, layout)
                    .table_slices()
                    .iter()
                    .map(|t| {
                        t.iter()
                            .map(|e| u128::from(e.unsigned_abs()))
                            .max()
                            .unwrap()
                    })
                    .sum();
                assert!(
                    worst <= s1,
                    "{design:?} {rate:?} {cutoff:?}: tables {worst} > {s1}"
                );
            }
            let mut x = s1;
            for op in &plan.ops {
                let (sum, out) = exact::stage(op, x)
                    .unwrap_or_else(|| panic!("{design:?} {rate:?} {cutoff:?}: {op:?} not exact"));
                assert!(sum < exact::LIMIT);
                assert!(
                    sym(op).fused,
                    "{design:?} {rate:?} {cutoff:?}: {op:?} not fused"
                );
                x = out;
            }
            // The gain multiply (at most +24 dB, below 16) keeps an integer output below
            // 2^53: one correctly rounded `f64` product, the same at every level (not exact
            // unless the gain is a power of two); the quantiser then rounds it.
            assert!(16 * x < exact::LIMIT);
        }
        // The paired S1 tables of the lanes: no half of a sum reaches 2^32, so the halves
        // never carry into each other; and the tables stay small enough for the L2 cache.
        for kind in S1Kind::ALL {
            let p = pair(kind.half());
            for w in p.worst() {
                assert!((0..1 << 32).contains(&w), "pair half bound {w}");
            }
            assert!(p.size() <= 160 << 10, "pair tables {} bytes", p.size());
        }
    }

    /// Passes in lanes give the ordered passes' outputs, for every plan, at every level,
    /// for pass lengths with and without whole blocks per run, on random bytes, idle and
    /// full-scale runs.
    #[test]
    fn lanes_match_ordered() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let bytes: Vec<u8> = (0..40_000)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                match (i / 3000) % 4 {
                    1 => 0xFF,
                    2 => IDLE,
                    3 => 0x00,
                    _ => seed as u8,
                }
            })
            .collect();
        for level in SimdLevel::available() {
            for (design, (rate, cutoff)) in DESIGNS.iter().flat_map(|&d| ALL.map(|p| (d, p))) {
                let plan = with_simd_level(level, || Plan::with_design(rate, cutoff, design))
                    .unwrap()
                    .unwrap();
                let paired = Some(S1Kind::of(design, cutoff).pair());
                let mut ordered = plan.clone();
                ordered.lanes = None;
                // With and without the paired S1 tables.
                for s1 in [paired, None] {
                    let mut lanes = plan.clone();
                    lanes.lanes = Some(s1);
                    for (o0, n) in [
                        (0, LANES_MIN),
                        (-200, 1176 * 6),
                        (64, 2050),
                        (10, 4096),
                        (-200, 1176),
                        (8, 2),
                    ] {
                        let n = n.min(plan.pass);
                        let (mut a, mut b) = (Scratch::default(), Scratch::default());
                        ordered.pass_run(&bytes, 300, o0, n, &mut a);
                        lanes.pass_run(&bytes, 300, o0, n, &mut b);
                        assert!(
                            a.a[..n] == b.a[..n],
                            "{level:?} {design:?} {rate:?} {cutoff:?} paired {} o0 {o0} n {n}",
                            s1.is_some()
                        );
                    }
                }
            }
        }
    }

    /// `x * 2^-q` rounded half to even, in integers.
    fn round_shift(x: i128, q: u32) -> i128 {
        let (d, r) = (x >> q, x & ((1 << q) - 1));
        let half = 1i128 << (q - 1);
        d + i128::from(r > half || (r == half && d % 2 != 0))
    }

    /// Level `l` of `plan` (0: S1 at every byte) at indices `a .. a + n`, computed from the
    /// definitions in `i128`, one sample at a time: S1 over the window around each byte;
    /// Fir1 `y[i] = sum h[j] x[i + j]`; Dec2 `y[i] = sum h[j] x[2 i + j]`; Interp2
    /// `y[2 i] = x[i]`, `y[2 i + 1] = 2 sum h[2 k + 1] (x[i - k] + x[i + 1 + k])`; each
    /// rescaled and rounded half to even. Bytes outside `src` (byte 0 first) are idle.
    fn reference(plan: &Plan, half: &[i32], src: &[u8], l: usize, a: i64, n: usize) -> Vec<i128> {
        if l == 0 {
            let byte = |b: i64| usize::try_from(b).ok().and_then(|b| src.get(b)).copied();
            let m = (half.len() - 1) as i64;
            return (a..a + n as i64)
                .map(|b| {
                    // Sample `8 b` is the centre; taps at offsets -m ..= m.
                    (-m..=m)
                        .map(|t| {
                            let i = 8 * b + t;
                            let bit = (byte(i.div_euclid(8)).unwrap_or(IDLE)
                                >> (7 - i.rem_euclid(8)))
                                & 1;
                            let h = i128::from(half[t.unsigned_abs() as usize]);
                            if bit == 1 {
                                h
                            } else {
                                -h
                            }
                        })
                        .sum()
                })
                .collect();
        }
        let s = sym(&plan.ops[l - 1]);
        // From the bits, not `log2`: Rust does not specify `log2`'s precision, and Miri
        // perturbs it (log2 of 2^-52 gave -51.99999999999998, truncated to 51).
        let q = exact::shift(s.scale);
        let h = |j: i64| s.full[j.unsigned_abs() as usize] as i128;
        let m = s.half_len() as i64;
        match &plan.ops[l - 1] {
            Op::Fir1(_) => {
                let x = reference(plan, half, src, l - 1, a - m, n + 2 * m as usize);
                (0..n)
                    .map(|i| {
                        round_shift(
                            (-m..=m)
                                .map(|j| h(j) * x[(i as i64 + m + j) as usize])
                                .sum(),
                            q,
                        )
                    })
                    .collect()
            }
            Op::Dec2(_) => {
                let x = reference(plan, half, src, l - 1, 2 * a - m, 2 * n + 2 * m as usize);
                (0..n)
                    .map(|i| {
                        round_shift(
                            (-m..=m)
                                .map(|j| h(j) * x[(2 * i as i64 + m + j) as usize])
                                .sum(),
                            q,
                        )
                    })
                    .collect()
            }
            Op::Interp2(_) => {
                let (a0, o) = (a.div_euclid(2), s.odd.len() as i64);
                let x = reference(plan, half, src, l - 1, a0 - o, n / 2 + 2 * o as usize + 2);
                let xi = |i: i64| x[(i - a0 + o) as usize];
                (a..a + n as i64)
                    .map(|t| {
                        let i = t.div_euclid(2);
                        if t % 2 == 0 {
                            xi(i)
                        } else {
                            let sum: i128 = (0..o)
                                .map(|k| h(2 * k + 1) * (xi(i - k) + xi(i + 1 + k)))
                                .sum();
                            round_shift(2 * sum, q)
                        }
                    })
                    .collect()
            }
        }
    }

    /// Every plan of both designs gives the samples of its definition ([`reference`]), at
    /// every level: the stage alignment, centring, decimation and interpolation phases and
    /// rounding, checked against a direct computation in integers.
    #[test]
    fn plans_match_reference() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let src: Vec<u8> = (0..1200)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                if (300..400).contains(&i) {
                    0xFF
                } else {
                    seed as u8
                }
            })
            .collect();
        for level in SimdLevel::available() {
            for design in DESIGNS {
                for (rate, cutoff) in ALL {
                    let plan = with_simd_level(level, || Plan::with_design(rate, cutoff, design))
                        .unwrap()
                        .unwrap();
                    let half = S1Kind::of(design, cutoff).half();
                    // From before the input to past its end.
                    let k = plan.bytes_per_output();
                    let (o0, n) = ((-200 / k) & !1, ((1600 / k) as usize) & !1);
                    let mut sc = Scratch::default();
                    plan.pass_run(&src, 0, o0, n, &mut sc);
                    let want = reference(&plan, half, &src, plan.ops.len(), o0, n);
                    let got: Vec<i128> = sc.a[..n].iter().map(|&v| v as i128).collect();
                    assert!(got == want, "{level:?} {design:?} {rate:?} {cutoff:?}");
                }
            }
        }
    }

    /// The precise plans written out from `docs/design.md`, independently of
    /// `precise_ops`: S1's taps, and per stage its kind, taps (by table) and the power of two
    /// it scales by (taps scaled by 2^22, F80 by 2^21; a stage on S1's 2^29 grid scales by 3
    /// more to return to 2^26).
    fn precise_literal(rate: PcmRate, cutoff: Cutoff) -> (&'static [i32], Vec<Op>) {
        use tables_precise as t;
        let s = |h: &[i32], e: i32| Sym::new(h, pow2(-e));
        let f = || {
            if cutoff == Cutoff::Low {
                &t::F30[..]
            } else {
                &t::F40[..]
            }
        };
        match (cutoff, rate) {
            (Cutoff::Wide, PcmRate::Hz176400) => (
                &t::S1_WIDE,
                vec![Op::Fir1(s(&t::F80, 24)), Op::Dec2(s(&t::F80, 21))],
            ),
            (Cutoff::Wide, _) => (
                &t::S1_WIDE,
                vec![Op::Fir1(s(&t::F40, 25)), Op::Fir1(s(&t::F40, 22))],
            ),
            (_, PcmRate::Hz88200) => (
                &t::S1,
                vec![
                    Op::Fir1(s(&t::P, 25)),
                    Op::Dec2(s(&t::HB, 22)),
                    Op::Fir1(s(f(), 22)),
                    Op::Dec2(s(f(), 22)),
                ],
            ),
            (_, PcmRate::Hz176400) => (
                &t::S1,
                vec![
                    Op::Fir1(s(&t::P, 25)),
                    Op::Dec2(s(&t::HB, 22)),
                    Op::Fir1(s(f(), 22)),
                    Op::Fir1(s(f(), 22)),
                ],
            ),
            (_, PcmRate::Hz352800) => (
                &t::S1,
                vec![
                    Op::Fir1(s(&t::P, 25)),
                    Op::Dec2(s(&t::HB, 22)),
                    Op::Fir1(s(f(), 22)),
                    Op::Fir1(s(f(), 22)),
                    Op::Interp2(s(&t::HB, 22)),
                    Op::Fir1(s(&t::P, 22)),
                ],
            ),
        }
    }

    /// The precise plans are the ones `docs/design.md` describes ([`precise_literal`]):
    /// the same S1, stage kinds, taps and scales; and a plan built from the literal stages
    /// gives, through the definitions ([`reference`]), the samples of the real plan, in
    /// ordered passes and in lanes (a pass of `LANES_MIN` outputs and more) at every level.
    #[test]
    fn precise_plans_match_the_design() {
        let mut seed = 0xD1B5_4A32_D192_ED03u64;
        let src: Vec<u8> = (0..20_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        for (rate, cutoff) in ALL {
            let (half, ops) = precise_literal(rate, cutoff);
            let plan = Plan::with_design(rate, cutoff, Design::Precise).unwrap();
            assert!(S1Kind::of(Design::Precise, cutoff).half() == half);
            assert_eq!(plan.ops.len(), ops.len(), "{rate:?} {cutoff:?}");
            for (a, b) in plan.ops.iter().zip(&ops) {
                assert_eq!(
                    std::mem::discriminant(a),
                    std::mem::discriminant(b),
                    "{rate:?} {cutoff:?}"
                );
                let (a, b) = (sym(a), sym(b));
                assert!(
                    a.full == b.full && a.scale == b.scale,
                    "{rate:?} {cutoff:?}"
                );
            }
            let mut literal = plan.clone();
            literal.ops = ops;
            let k = plan.bytes_per_output();
            for level in SimdLevel::available() {
                let plan =
                    with_simd_level(level, || Plan::with_design(rate, cutoff, Design::Precise))
                        .unwrap()
                        .unwrap();
                for (o0, n, lanes) in [((-64 / k) & !1, 1000usize, false), (0, LANES_MIN, true)] {
                    let mut p = plan.clone();
                    if lanes {
                        // The lanes path, with and without the paired tables where the
                        // level has none.
                        p.lanes = Some(p.lanes.flatten());
                    } else {
                        p.lanes = None;
                    }
                    let n = n.min(((src.len() as i64 / k) as usize) & !1);
                    assert!(!lanes || n >= p.lanes_min, "the lanes path is taken");
                    let mut sc = Scratch::default();
                    p.pass_run(&src, 0, o0, n, &mut sc);
                    let want = reference(&literal, half, &src, literal.ops.len(), o0, n);
                    let got: Vec<i128> = sc.a[..n].iter().map(|&v| v as i128).collect();
                    assert!(got == want, "{level:?} {rate:?} {cutoff:?} lanes {lanes}");
                }
            }
        }
    }

    /// A stage whose sums could exceed 2^53 is not fused, nor is any stage after it.
    #[test]
    fn unproven_stages_are_not_fused() {
        let big = [1 << 30; 8];
        let mut ops = vec![
            Op::Dec2(Sym::new(&tables::HB, pow2(-Q_FIR))),
            Op::Fir1(Sym::new(&big, pow2(-Q_FIR))),
            Op::Fir1(Sym::new(&tables::F40, pow2(-Q_FIR))),
        ];
        assert!(!exact::prove(&tables::S1, &mut ops));
        let fused: Vec<bool> = ops.iter().map(|op| sym(op).fused).collect();
        assert_eq!(fused, [true, false, false]);
    }

    /// scarletbook-pcm's S1 tables: one per byte of the window.
    fn byte_tables(half: &[i32]) -> Vec<[i32; 256]> {
        let bytes = half.len() / 4;
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

    /// The S1 kernel with wide-index tables gives the sums of scarletbook-pcm's byte tables,
    /// for both windows, strides 1 and 2, every start in a word, at every level and on the
    /// any-window path.
    #[test]
    fn s1_chunks_match_bytes() {
        let mut seed = 0x5DEE_CE66_D1A4_F00Du64;
        let bytes: Vec<u8> = (0..4096)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                // Runs of all ones and all zeros as well: the largest sums.
                match (i / 512) % 4 {
                    1 => 0xFF,
                    3 => 0x00,
                    _ => seed as u8,
                }
            })
            .collect();
        for half in S1Kind::ALL.map(S1Kind::half) {
            let reference = byte_tables(half);
            let b = reference.len();
            for stride in [1, 2] {
                for start in 0..9 {
                    let n = (bytes.len() - start - b) / stride;
                    let want: Vec<f64> = (0..n)
                        .map(|k| {
                            let w = &bytes[start + stride * k..][..b];
                            let sum: i32 = reference
                                .iter()
                                .zip(w)
                                .map(|(t, &x)| t[usize::from(x)])
                                .sum();
                            f64::from(sum)
                        })
                        .collect();
                    for layout in [S1Layout::Wide, S1Layout::Byte] {
                        let chunked = lut(half, layout);
                        for level in SimdLevel::available() {
                            let mut got = vec![0.0; n];
                            let isa = with_simd_level(level, Isa::current).unwrap();
                            dispatch::s1(isa, &chunked, &bytes, start, stride, &mut got);
                            assert!(
                                got == want,
                                "{level:?} {layout:?} {b} bytes, stride {stride}, start {start}"
                            );
                        }
                        let mut got = vec![0.0; n];
                        kernel::s1_any(&chunked, &bytes, start, stride, &mut got);
                        assert!(
                            got == want,
                            "any window: {layout:?} {b} bytes, stride {stride}"
                        );
                    }
                }
            }
        }
    }

    /// The detected level is the best this CPU has, and `with_simd_level` caps it.
    #[test]
    fn levels_follow_the_cpu() {
        // DSD2PCM_SIMD caps every level on purpose; this check only holds without it.
        if std::env::var_os("DSD2PCM_SIMD").is_some() {
            return;
        }
        let levels = SimdLevel::available();
        assert_eq!(levels[0], SimdLevel::Generic);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            let avx2 = std::arch::is_x86_feature_detected!("avx2");
            let fma = avx2 && std::arch::is_x86_feature_detected!("fma");
            assert_eq!(levels.contains(&SimdLevel::Avx2), avx2);
            assert_eq!(levels.contains(&SimdLevel::Avx2Fma), fma);
            let sse2 = std::arch::is_x86_feature_detected!("sse2");
            assert_eq!(levels.contains(&SimdLevel::Sse2), sse2);
        }
        for &level in &levels {
            let plan = with_simd_level(level, || Plan::new(PcmRate::Hz88200, Cutoff::Standard))
                .unwrap()
                .unwrap();
            assert_eq!(plan.isa().level(), level);
        }
        if std::env::var_os("DSD2PCM_SIMD").is_none() {
            assert_eq!(Isa::current().level(), *levels.last().unwrap());
        }
    }
}
