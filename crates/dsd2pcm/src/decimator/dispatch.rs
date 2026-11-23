// SPDX-License-Identifier: MIT OR Apache-2.0
//! The kernels' instruction set, chosen at run time. Ported from scarletbook-pcm 0.7.0
//! (src/dispatch.rs); Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR
//! Apache-2.0 (see PROVENANCE.md).
//!
//! The levels: the generic kernels as compiled; on aarch64, `neon`: passes in lanes
//! ([`super::lanes`]) in blocks sized for its 32 vector registers, with fused multiply-adds
//! in the stages [`super::exact`] proved exact (NEON and FMA are aarch64's baseline: no
//! `#[target_feature]`, no `unsafe`); on x86 and x86-64, the same safe code built
//! with SSE2 (`#[target_feature]`), which runs passes in lanes ([`super::lanes`]) in blocks
//! sized for SSE2, picked when the CPU has no AVX2; the safe code built with AVX2; and that
//! build with fused multiply-adds in the stages whose sums [`super::exact`] proved exact
//! (`Sym::fused`). None changes an output: the builds do the same IEEE-754 operations in the
//! same order per output (the compiler never fuses or reorders floating-point operations on
//! its own), a fused multiply-add gives the same result as a product and an addition when
//! both are exact, and the lanes do the same operations per output in the same order as the
//! ordered kernels, on another data layout. On x86-64 the fused level runs plans whose every
//! stage is proven exact in lanes.
//!
//! This is the only module of the crate with `unsafe`: calling a `#[target_feature]`
//! function is `unsafe`. Every call is made with an [`Isa`] whose level is at most the one
//! [`Isa::detect`] found with `is_x86_feature_detected!`: no other code can make an `Isa`,
//! and [`Isa::cap`] can only lower the level. The wrappers hold no other unsafe operation:
//! no intrinsics, no raw pointers.

#![allow(unsafe_code)]

use super::kernel::{self, Plain, S1Layout, S1Lut, Sym};
use super::lanes;

/// A SIMD level of the kernels. Every level gives the same output bytes.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SimdLevel {
    /// The kernels as compiled.
    Generic,
    /// aarch64 (always NEON and FMA): passes in lanes in blocks sized for its 32 vector
    /// registers, fused only in stages proven exact.
    Neon,
    /// x86 or x86-64 with SSE2 (and no AVX2): passes in lanes, in blocks sized for SSE2.
    Sse2,
    /// x86 or x86-64 with AVX2.
    Avx2,
    /// x86 or x86-64 with AVX2 and FMA (fused only in stages proven exact).
    Avx2Fma,
}

impl SimdLevel {
    /// The levels this CPU can run, lowest first.
    pub fn available() -> Vec<SimdLevel> {
        let top = Isa::detect().0;
        [
            SimdLevel::Generic,
            SimdLevel::Neon,
            SimdLevel::Sse2,
            SimdLevel::Avx2,
            SimdLevel::Avx2Fma,
        ]
        .into_iter()
        .filter(|&l| l <= top && l.on_this_target())
        .collect()
    }

    /// Whether this target has the level at all: NEON is a level on aarch64 only, the
    /// others above the generic one on x86 and x86-64 only.
    fn on_this_target(self) -> bool {
        if self == SimdLevel::Neon {
            cfg!(target_arch = "aarch64")
        } else {
            self == SimdLevel::Generic || cfg!(any(target_arch = "x86", target_arch = "x86_64"))
        }
    }

    fn parse(s: &str) -> Option<SimdLevel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "generic" | "scalar" => Some(SimdLevel::Generic),
            "neon" => Some(SimdLevel::Neon),
            "sse2" => Some(SimdLevel::Sse2),
            "avx2" => Some(SimdLevel::Avx2),
            "avx2fma" | "avx2+fma" | "fma" => Some(SimdLevel::Avx2Fma),
            _ => None,
        }
    }
}

std::thread_local! {
    static CAP: std::cell::Cell<Option<SimdLevel>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with the kernels capped at `level` for decimators and conversions set up on this
/// thread inside `f` (workers they start use the same level). Returns `None` without running
/// `f` if this CPU lacks `level`. For tests and benchmarks.
#[doc(hidden)]
pub fn with_simd_level<R>(level: SimdLevel, f: impl FnOnce() -> R) -> Option<R> {
    if level > Isa::detect().0 || !level.on_this_target() {
        return None;
    }
    let old = CAP.with(|c| c.replace(Some(level)));
    struct Restore(Option<SimdLevel>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CAP.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(old);
    Some(f())
}

/// The instruction set the kernels use. Only [`Isa::detect`] makes one; [`Isa::cap`] lowers
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Isa(SimdLevel);

impl Isa {
    /// The best level this CPU has. On 32-bit x86 every level above the generic one needs
    /// SSE2.
    fn detect() -> Isa {
        #[cfg(target_arch = "x86")]
        if !std::arch::is_x86_feature_detected!("sse2") {
            return Isa(SimdLevel::Generic);
        }
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("avx2") {
            if std::arch::is_x86_feature_detected!("fma") {
                return Isa(SimdLevel::Avx2Fma);
            }
            return Isa(SimdLevel::Avx2);
        }
        if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            // x86-64 always has SSE2; on 32-bit x86 it was checked above.
            Isa(SimdLevel::Sse2)
        } else if cfg!(target_arch = "aarch64") {
            // aarch64 always has NEON and FMA.
            Isa(SimdLevel::Neon)
        } else {
            Isa(SimdLevel::Generic)
        }
    }

    /// At most `level`: the highest level of this target at or below both.
    fn cap(self, level: SimdLevel) -> Isa {
        let l = self.0.min(level);
        Isa(if l.on_this_target() {
            l
        } else {
            SimdLevel::Generic
        })
    }

    /// The level to use now: the detected one, capped by the environment variable
    /// `DSD2PCM_SIMD` (`generic`, `neon`, `sse2`, `avx2` or `avx2fma`) and by [`with_simd_level`].
    pub(crate) fn current() -> Isa {
        static ENV: std::sync::OnceLock<Option<SimdLevel>> = std::sync::OnceLock::new();
        let mut isa = Isa::detect();
        if let Some(l) = ENV.get_or_init(|| {
            std::env::var("DSD2PCM_SIMD")
                .ok()
                .and_then(|s| SimdLevel::parse(&s))
        }) {
            isa = isa.cap(*l);
        }
        if let Some(l) = CAP.with(|c| c.get()) {
            isa = isa.cap(l);
        }
        isa
    }

    /// The level.
    #[cfg(test)]
    pub(crate) fn level(self) -> SimdLevel {
        self.0
    }
}

// SAFETY (every `unsafe` block below): an `Isa` at `Avx2` or above exists only if
// `Isa::detect` found AVX2 on this CPU, and at `Avx2Fma` only if it also found FMA; on
// 32-bit x86, an `Isa` at `Sse2` or above only if it found SSE2; on x86-64 every CPU has
// SSE2 (the target's baseline, which every x86-64 build assumes). `Isa::cap` only lowers a
// level (to `Generic` where the target lacks a level). Each block calls one
// `#[target_feature]` wrapper whose features the matched level guarantees, and nothing
// else.

/// [`kernel::s1`].
pub(crate) fn s1(
    isa: Isa,
    lut: &S1Lut,
    bytes: &[u8],
    start: usize,
    stride: usize,
    out: &mut [f64],
) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::s1(lut, bytes, start, stride, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::s1(lut, bytes, start, stride, out) },
        _ => kernel::s1(lut, bytes, start, stride, out),
    }
}

/// [`kernel::fir1`]; fused only if `s.fused`.
pub(crate) fn fir1(isa: Isa, s: &Sym, x: &[f64], out: &mut [f64]) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 and FMA (see above).
        SimdLevel::Avx2Fma if s.fused => unsafe { avx2fma::fir1(s, x, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::fir1(s, x, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::fir1(s, x, out) },
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if s.fused => neon::fir1(s, x, out),
        _ => kernel::fir1::<{ kernel::L_FIR1 }, Plain>(s, x, out),
    }
}

/// [`kernel::dec2`]; fused only if `s.fused`.
pub(crate) fn dec2(isa: Isa, s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 and FMA (see above).
        SimdLevel::Avx2Fma if s.fused => unsafe { avx2fma::dec2(s, xe, xo, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::dec2(s, xe, xo, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::dec2(s, xe, xo, out) },
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if s.fused => neon::dec2(s, xe, xo, out),
        _ => kernel::dec2::<{ kernel::L }, Plain>(s, xe, xo, out),
    }
}

/// [`kernel::interp2_odd`]; fused only if `s.fused`.
pub(crate) fn interp2_odd(isa: Isa, s: &Sym, w: &[f64], out: &mut [f64]) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 and FMA (see above).
        SimdLevel::Avx2Fma if s.fused => unsafe { avx2fma::interp2_odd(s, w, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::interp2_odd(s, w, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::interp2_odd(s, w, out) },
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon if s.fused => neon::interp2_odd(s, w, out),
        _ => kernel::interp2_odd::<{ kernel::L }, Plain>(s, w, out),
    }
}

/// Whether passes may run in lanes ([`lanes::run`]) at `isa`: x86-64 with AVX2 and FMA, and
/// the `sse2` and `neon` levels (on 32-bit x86 the AVX2 levels run the ordered kernels).
pub(super) fn lanes_level(isa: Isa) -> bool {
    (cfg!(target_arch = "x86_64") && isa.0 == SimdLevel::Avx2Fma)
        || matches!(isa.0, SimdLevel::Sse2 | SimdLevel::Neon)
}

/// Whether passes run in lanes from fewer outputs at `isa` (`super::LANES_MIN_SSE2`): at
/// the `sse2` level.
pub(super) fn short_lanes(isa: Isa) -> bool {
    isa.0 == SimdLevel::Sse2
}

/// The layout of the S1 tables at `isa`: bytes on 32-bit targets below AVX2 (small
/// in-order cores, whose level-1 cache holds the byte tables but not the wide ones), else
/// the wide chunks.
pub(super) fn s1_layout(isa: Isa) -> S1Layout {
    if cfg!(target_pointer_width = "32") && isa.0 < SimdLevel::Avx2 {
        S1Layout::Byte
    } else {
        S1Layout::Wide
    }
}

/// Whether the lanes take S1 from the paired tables ([`lanes::S1Pair`]): not on 32-bit
/// targets, where each 64-bit addition takes two instructions and the 112 KiB of tables
/// leave the level-1 cache; there S1 uses the byte tables of the ordered kernels.
pub(super) fn lanes_pair() -> bool {
    cfg!(target_pointer_width = "64")
}

/// [`lanes::run`]; fused only at the fused level (every stage must be proven exact:
/// [`lanes::takes`]).
pub(super) fn lanes<'b>(
    isa: Isa,
    p: &lanes::Pass<'_>,
    pad: usize,
    bufs: &'b mut lanes::Bufs,
) -> &'b [lanes::Lanes] {
    match isa.0 {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: the level guarantees AVX2 and FMA (see above).
        SimdLevel::Avx2Fma => unsafe { avx2fma::run_lanes(p, pad, bufs) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::run_lanes(p, pad, bufs) },
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon => neon::run_lanes(p, pad, bufs),
        _ => lanes::run::<Plain, 8, 8, false>(p, pad, bufs),
    }
}

/// [`kernel::quantise`].
pub(crate) fn quantise(isa: Isa, v: &[f64], gain: f64, q: &mut [i32]) -> u64 {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::quantise(v, gain, q) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::quantise(v, gain, q) },
        _ => kernel::quantise(v, gain, q),
    }
}

/// [`kernel::pack24`].
pub(crate) fn pack24(isa: Isa, q: &[&[i32]], out: &mut [u8]) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::pack24(q, out) },
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees SSE2 (see above).
        SimdLevel::Sse2 => unsafe { sse2::pack24(q, out) },
        _ => kernel::pack24(q, out),
    }
}

/// The kernels built with SSE2 (which 32-bit x86 targets need not assume; x86-64 always has
/// it), without fused multiply-adds, and the lanes in blocks of [`sse2::SV1`] and
/// [`sse2::SV2`] output vectors, sized for the SSE2 registers of the target. Each is the
/// safe generic kernel; callers must have checked that the CPU has SSE2.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod sse2 {
    use super::kernel::{self, Plain, S1Lut, Sym};
    use super::lanes;

    /// Output vectors per block of the lanes (2 SSE2 registers each) in Fir1 and Interp2:
    /// on 32-bit x86, 4 sums and the operands of a step in its 8 registers; on x86-64, with
    /// 16 registers, 8 sums.
    pub(super) const SV1: usize = if cfg!(target_arch = "x86") { 2 } else { 4 };
    /// Output vectors per block of the lanes in Dec2: 8 sums (faster than 4 on 32-bit x86;
    /// 16 measured slower on x86-64).
    pub(super) const SV2: usize = 4;

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn s1(
        lut: &S1Lut,
        bytes: &[u8],
        start: usize,
        stride: usize,
        out: &mut [f64],
    ) {
        kernel::s1(lut, bytes, start, stride, out)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn fir1(s: &Sym, x: &[f64], out: &mut [f64]) {
        kernel::fir1::<{ kernel::L_FIR1 }, Plain>(s, x, out)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn dec2(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
        kernel::dec2::<{ kernel::L }, Plain>(s, xe, xo, out)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn interp2_odd(s: &Sym, w: &[f64], out: &mut [f64]) {
        kernel::interp2_odd::<{ kernel::L }, Plain>(s, w, out)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn quantise(v: &[f64], gain: f64, q: &mut [i32]) -> u64 {
        kernel::quantise(v, gain, q)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn pack24(q: &[&[i32]], out: &mut [u8]) {
        kernel::pack24(q, out)
    }

    #[target_feature(enable = "sse2")]
    pub(super) unsafe fn run_lanes<'b>(
        p: &lanes::Pass<'_>,
        pad: usize,
        bufs: &'b mut lanes::Bufs,
    ) -> &'b [lanes::Lanes] {
        lanes::run::<Plain, SV1, SV2, true>(p, pad, bufs)
    }
}

/// The kernels built with AVX2, without fused multiply-adds. Each is the safe generic
/// kernel; callers must have checked that the CPU has AVX2.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx2 {
    use super::kernel::{self, Plain, S1Lut, Sym};

    /// Outputs per block: 8 vectors of AVX.
    const L: usize = 32;

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn s1(
        lut: &S1Lut,
        bytes: &[u8],
        start: usize,
        stride: usize,
        out: &mut [f64],
    ) {
        kernel::s1(lut, bytes, start, stride, out)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn fir1(s: &Sym, x: &[f64], out: &mut [f64]) {
        kernel::fir1::<L, Plain>(s, x, out)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn dec2(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
        kernel::dec2::<L, Plain>(s, xe, xo, out)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn interp2_odd(s: &Sym, w: &[f64], out: &mut [f64]) {
        kernel::interp2_odd::<L, Plain>(s, w, out)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn quantise(v: &[f64], gain: f64, q: &mut [i32]) -> u64 {
        kernel::quantise(v, gain, q)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn pack24(q: &[&[i32]], out: &mut [u8]) {
        kernel::pack24(q, out)
    }
}

/// A fused multiply-add (one rounding). The only `mul_add` of the crate is here, used only
/// by the fused builds below ([`avx2fma`], [`neon`]) and only for stages with `Sym::fused`
/// (and the lanes, which take only such stages: [`lanes::takes`]), so no other stage can
/// fuse.
#[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
struct Fused;

#[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
impl kernel::Mac for Fused {
    #[inline(always)]
    fn mac(acc: f64, h: f64, x: f64) -> f64 {
        h.mul_add(x, acc)
    }
}

/// The FIR kernels built with AVX2 and FMA, with fused multiply-adds. Used only for stages
/// with `Sym::fused`; callers must have checked that the CPU has AVX2 and FMA.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx2fma {
    use super::kernel::{self, Sym};
    #[cfg(target_arch = "x86_64")]
    use super::lanes;
    use super::Fused;

    /// Outputs per block of the ordered kernels: 8 vectors of AVX, 8 independent chains to
    /// cover the FMA latency.
    const L: usize = 32;

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn fir1(s: &Sym, x: &[f64], out: &mut [f64]) {
        kernel::fir1::<L, Fused>(s, x, out)
    }

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn dec2(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
        kernel::dec2::<L, Fused>(s, xe, xo, out)
    }

    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn interp2_odd(s: &Sym, w: &[f64], out: &mut [f64]) {
        kernel::interp2_odd::<L, Fused>(s, w, out)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,fma")]
    pub(super) unsafe fn run_lanes<'b>(
        p: &lanes::Pass<'_>,
        pad: usize,
        bufs: &'b mut lanes::Bufs,
    ) -> &'b [lanes::Lanes] {
        lanes::run::<Fused, 8, 8, false>(p, pad, bufs)
    }
}

/// The `neon` level of aarch64: the ordered FIR kernels fused (stages with `Sym::fused`
/// only) and the lanes in blocks of [`neon::SV1`] and [`neon::SV2`] output vectors (2 NEON
/// registers each), sized for its 32 vector registers. NEON and FMA are the target's
/// baseline, so these are plain safe functions.
#[cfg(target_arch = "aarch64")]
mod neon {
    use super::kernel::{self, Plain, Sym};
    use super::{lanes, Fused};

    /// TEMPORARY tuning: DSD2PCM_NEON_TUNE="sv1,sv2,whole,l,lmin".
    #[derive(Clone, Copy)]
    pub(crate) struct Tune {
        pub sv1: usize,
        pub sv2: usize,
        pub whole: bool,
        pub l: usize,
        pub lmin: usize,
    }

    pub(crate) fn tune() -> Tune {
        static T: std::sync::OnceLock<Tune> = std::sync::OnceLock::new();
        *T.get_or_init(|| {
            let v: Vec<usize> = std::env::var("DSD2PCM_NEON_TUNE")
                .unwrap_or_else(|_| "8,8,1,16,0".into())
                .split(',')
                .map(|x| x.trim().parse().unwrap())
                .collect();
            Tune {
                sv1: v[0],
                sv2: v[1],
                whole: v[2] == 1,
                l: v[3],
                lmin: v[4],
            }
        })
    }

    pub(super) fn fir1(s: &Sym, x: &[f64], out: &mut [f64]) {
        match tune().l {
            32 => kernel::fir1::<32, Fused>(s, x, out),
            0 => kernel::fir1::<{ kernel::L }, Plain>(s, x, out),
            _ => kernel::fir1::<16, Fused>(s, x, out),
        }
    }

    pub(super) fn dec2(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
        match tune().l {
            32 => kernel::dec2::<32, Fused>(s, xe, xo, out),
            0 => kernel::dec2::<{ kernel::L }, Plain>(s, xe, xo, out),
            _ => kernel::dec2::<16, Fused>(s, xe, xo, out),
        }
    }

    pub(super) fn interp2_odd(s: &Sym, w: &[f64], out: &mut [f64]) {
        match tune().l {
            32 => kernel::interp2_odd::<32, Fused>(s, w, out),
            0 => kernel::interp2_odd::<{ kernel::L }, Plain>(s, w, out),
            _ => kernel::interp2_odd::<16, Fused>(s, w, out),
        }
    }

    pub(super) fn run_lanes<'b>(
        p: &lanes::Pass<'_>,
        pad: usize,
        bufs: &'b mut lanes::Bufs,
    ) -> &'b [lanes::Lanes] {
        let t = tune();
        match (t.sv1, t.sv2, t.whole) {
            (4, 4, true) => lanes::run::<Fused, 4, 4, true>(p, pad, bufs),
            (4, 8, true) => lanes::run::<Fused, 4, 8, true>(p, pad, bufs),
            (8, 8, false) => lanes::run::<Fused, 8, 8, false>(p, pad, bufs),
            (16, 8, true) => lanes::run::<Fused, 16, 8, true>(p, pad, bufs),
            (8, 16, true) => lanes::run::<Fused, 8, 16, true>(p, pad, bufs),
            (4, 16, true) => lanes::run::<Fused, 4, 16, true>(p, pad, bufs),
            _ => lanes::run::<Fused, 8, 8, true>(p, pad, bufs),
        }
    }
}

/// TEMPORARY tuning: the lane threshold choice at the `neon` level.
pub(super) fn neon_lmin(isa: Isa) -> Option<usize> {
    #[cfg(target_arch = "aarch64")]
    if isa.0 == SimdLevel::Neon {
        return Some(neon::tune().lmin);
    }
    let _ = isa;
    None
}

// pysacd's filter (`crate::filters::pysacd`): the same safe kernels, built with AVX2. No
// level fuses: its sums are not exact, so every product is rounded before it is added.
use crate::filters::pysacd::{self as pk, Lut};

impl Isa {
    /// [`pk::s1_kernel`].
    pub(crate) fn pysacd_s1(self, lut: &Lut, bytes: &[u8], out: &mut [f32]) {
        match self.0 {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            // SAFETY: the level guarantees AVX2 (see above).
            SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { pysacd_avx2::s1(lut, bytes, out) },
            _ => pk::s1_kernel(lut, bytes, out),
        }
    }

    /// [`pk::dec2_kernel`].
    pub(crate) fn pysacd_dec2(self, h: &[f32], xe: &[f32], xo: &[f32], out: &mut [f32]) {
        match self.0 {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            // SAFETY: the level guarantees AVX2 (see above).
            SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { pysacd_avx2::dec2(h, xe, xo, out) },
            _ => pk::dec2_kernel(h, xe, xo, out),
        }
    }

    /// [`pk::quantise_kernel`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pysacd_quantise(
        self,
        chans: &[&[f32]],
        n0: u64,
        tc0: u64,
        per_frame: usize,
        scale: f32,
        out: &mut [i32],
        clips: &mut [u64],
    ) {
        match self.0 {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            // SAFETY: the level guarantees AVX2 (see above).
            SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe {
                pysacd_avx2::quantise(chans, n0, tc0, per_frame, scale, out, clips)
            },
            _ => pk::quantise_kernel(chans, n0, tc0, per_frame, scale, out, clips),
        }
    }
}

/// pysacd's kernels built with AVX2 (never FMA); callers must have checked that the CPU has
/// AVX2.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod pysacd_avx2 {
    use crate::filters::pysacd::{self as pk, Lut};

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn s1(lut: &Lut, bytes: &[u8], out: &mut [f32]) {
        pk::s1_kernel(lut, bytes, out)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn dec2(h: &[f32], xe: &[f32], xo: &[f32], out: &mut [f32]) {
        pk::dec2_kernel(h, xe, xo, out)
    }

    #[allow(clippy::too_many_arguments)]
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn quantise(
        chans: &[&[f32]],
        n0: u64,
        tc0: u64,
        per_frame: usize,
        scale: f32,
        out: &mut [i32],
        clips: &mut [u64],
    ) {
        pk::quantise_kernel(chans, n0, tc0, per_frame, scale, out, clips)
    }
}
