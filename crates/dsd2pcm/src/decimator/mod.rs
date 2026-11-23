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

static PAIR_S1: OnceLock<lanes::S1Pair> = OnceLock::new();
static PAIR_S1_WIDE: OnceLock<lanes::S1Pair> = OnceLock::new();

/// S1's tables and the wide filter's, in each layout.
static LUT_S1: [OnceLock<Lut>; 2] = [OnceLock::new(), OnceLock::new()];
static LUT_S1_WIDE: [OnceLock<Lut>; 2] = [OnceLock::new(), OnceLock::new()];

/// The tables of `half` (S1 or the wide filter) in `layout`, built once.
fn s1_lut(half: &'static [i32], layout: S1Layout) -> &'static Lut {
    let cell = if half == &tables::S1_WIDE[..] {
        &LUT_S1_WIDE
    } else {
        &LUT_S1
    };
    cell[layout as usize].get_or_init(|| lut(half, layout))
}

const Q_FIR: i32 = 24;

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
const MAX_OPS: usize = 3;

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
/// assumed): S1 is a function of those bytes alone, so the output cannot change.
#[derive(Debug, Default)]
pub(crate) struct Carry {
    /// The previous pass's bytes, the first at byte `b0` of its timeline.
    bytes: Vec<u8>,
    b0: i64,
    /// Its S1 values: split (`ev[i]` over the window from byte `b0 + 2 i`, `od[i]` from
    /// `b0 + 2 i + 1`) or not (`ev[i]` from `b0 + i`).
    ev: Vec<f64>,
    od: Vec<f64>,
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
        let start = self.b0 - self.shift;
        if !self.valid || self.split != split || start > p0 {
            return 0;
        }
        let off = (p0 - start) as usize;
        let Some(prev) = self.bytes.get(off..) else {
            return 0;
        };
        // The bytes both passes share, from `p0` on.
        let n = prev.len().min(bytes.len());
        let eq = if prev[..n] == bytes[..n] {
            n
        } else {
            prev.iter().zip(bytes).take_while(|(x, y)| x == y).count()
        };
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

    /// Keeps a pass's bytes (the first at byte `p0`) and its S1 values, swapping the
    /// buffers.
    fn keep(&mut self, bytes: &[u8], p0: i64, split: bool, ev: &mut Vec<f64>, od: &mut Vec<f64>) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
        self.b0 = p0;
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
    lut: &'static Lut,
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

impl Plan {
    pub(crate) fn new(rate: PcmRate, cutoff: Cutoff) -> Result<Plan> {
        let hb = || Sym::new(&tables::HB, pow2(-Q_FIR));
        let f40 = || Sym::new(&tables::F40, pow2(-Q_FIR));
        let isa = Isa::current();
        let layout = dispatch::s1_layout(isa);
        let wide = || s1_lut(&tables::S1_WIDE, layout);
        let (s1_half, lut, mut ops) = match (cutoff, rate) {
            (Cutoff::Wide, PcmRate::Hz88200) => {
                return Err(Error::InvalidArgument(
                    "the wide filter is not available at 88.2 kHz".into(),
                ))
            }
            (Cutoff::Wide, PcmRate::Hz352800) => {
                (&tables::S1_WIDE[..], wide(), vec![Op::Fir1(f40())])
            }
            (Cutoff::Wide, PcmRate::Hz176400) => (
                &tables::S1_WIDE[..],
                wide(),
                vec![Op::Dec2(Sym::new(&tables::F80, pow2(-Q_FIR)))],
            ),
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
                (&tables::S1[..], s1_lut(&tables::S1, layout), ops)
            }
        };
        debug_assert!(ops.len() <= MAX_OPS);
        exact::prove(s1_half, &mut ops);
        let k = (rate.decimation() / 8) as i64;
        let pass = (PASS_BYTES / k as usize).div_ceil(PAD) * PAD;
        let lanes = (dispatch::lanes_level(isa)
            && ops.iter().all(|op| match op {
                Op::Fir1(s) | Op::Dec2(s) | Op::Interp2(s) => lanes::takes(s),
            }))
        .then(|| {
            dispatch::lanes_pair().then(|| {
                if cutoff == Cutoff::Wide {
                    PAIR_S1_WIDE.get_or_init(|| pair(&tables::S1_WIDE))
                } else {
                    PAIR_S1.get_or_init(|| pair(&tables::S1))
                }
            })
        });
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
        let n = LANES_MIN - 2;
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
                // that warmed up with a long pass allocates nothing later: one ordered pass
                // of the longest length that stays ordered (on idle bytes; `sc.a` is
                // overwritten below).
                sc.ordered_sized = true;
                self.pass_ordered(&[], 0, 0, LANES_MIN - 2, sc, None);
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
                _ => dispatch::s1(self.isa, self.lut, bytes, 0, 1, a),
            }
        }
        for (k, op) in self.ops.iter().enumerate() {
            let n_out = levels[k + 1].1;
            match op {
                Op::Fir1(s) => {
                    b.resize(n_out.div_ceil(PAD) * PAD, 0.0);
                    dispatch::fir1(self.isa, s, a, b);
                    if let (0, Some(c)) = (k, carry.as_deref_mut()) {
                        c.keep(bytes, p0, false, a, &mut Vec::new());
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
                            c.keep(bytes, p0, true, even, odd);
                        }
                    } else {
                        even.resize(np + 2 * e, 0.0);
                        odd.resize(np + 2 * o, 0.0);
                        if k == 0 {
                            dispatch::s1(self.isa, self.lut, bytes, de, 2, even);
                            dispatch::s1(self.isa, self.lut, bytes, dodd, 2, odd);
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

    fn s1_half(cutoff: Cutoff) -> &'static [i32] {
        if cutoff == Cutoff::Wide {
            &tables::S1_WIDE
        } else {
            &tables::S1
        }
    }

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
        for (rate, cutoff) in ALL {
            let plan = Plan::new(rate, cutoff).unwrap();
            let s1 = exact::s1_bound(s1_half(cutoff));
            assert!(s1 < i32::MAX as u128, "S1 bound {s1}");
            // The tables themselves stay within the bound, in both layouts.
            for layout in [S1Layout::Wide, S1Layout::Byte] {
                let worst: u128 = lut(s1_half(cutoff), layout)
                    .table_slices()
                    .iter()
                    .map(|t| {
                        t.iter()
                            .map(|e| u128::from(e.unsigned_abs()))
                            .max()
                            .unwrap()
                    })
                    .sum();
                assert!(worst <= s1, "{rate:?} {cutoff:?}: tables {worst} > {s1}");
            }
            let mut x = s1;
            for op in &plan.ops {
                let (sum, out) = exact::stage(op, x)
                    .unwrap_or_else(|| panic!("{rate:?} {cutoff:?}: {op:?} not exact"));
                assert!(sum < exact::LIMIT);
                assert!(sym(op).fused, "{rate:?} {cutoff:?}: {op:?} not fused");
                x = out;
            }
            // The gain multiply (at most +24 dB, below 16) keeps an integer output below
            // 2^53: one correctly rounded `f64` product, the same at every level (not exact
            // unless the gain is a power of two); the quantiser then rounds it.
            assert!(16 * x < exact::LIMIT);
        }
        // The paired S1 tables of the lanes: no half of a sum reaches 2^32, so the halves
        // never carry into each other; and the tables stay small enough for the L2 cache.
        for half in [&tables::S1[..], &tables::S1_WIDE[..]] {
            let p = pair(half);
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
            for (rate, cutoff) in ALL {
                let plan = with_simd_level(level, || Plan::new(rate, cutoff))
                    .unwrap()
                    .unwrap();
                let paired = Some(if cutoff == Cutoff::Wide {
                    PAIR_S1_WIDE.get_or_init(|| pair(&tables::S1_WIDE))
                } else {
                    PAIR_S1.get_or_init(|| pair(&tables::S1))
                });
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
                            "{level:?} {rate:?} {cutoff:?} paired {} o0 {o0} n {n}",
                            s1.is_some()
                        );
                    }
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
        for half in [&tables::S1[..], &tables::S1_WIDE[..]] {
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
