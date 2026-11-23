// SPDX-License-Identifier: MIT OR Apache-2.0
//! The kernels' instruction set, chosen at run time. Ported from scarletbook-pcm 0.7.0
//! (src/dispatch.rs); Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR
//! Apache-2.0 (see PROVENANCE.md).
//!
//! Three levels: the generic kernels as compiled; the same safe code built with AVX2
//! (`#[target_feature]`); and that build with fused multiply-adds in the stages whose sums
//! [`super::exact`] proved exact (`Sym::fused`). None changes an output: the generic and
//! AVX2 builds do the same IEEE-754 operations in the same order per output (the compiler
//! never fuses or reorders floating-point operations on its own), and a fused multiply-add
//! gives the same result as a product and an addition when both are exact.
//!
//! This is the only module of the crate with `unsafe`: calling a `#[target_feature]`
//! function is `unsafe`. Every call is made with an [`Isa`] whose level is at most the one
//! [`Isa::detect`] found with `is_x86_feature_detected!`: no other code can make an `Isa`,
//! and [`Isa::cap`] can only lower the level. The wrappers hold no other unsafe operation:
//! no intrinsics, no raw pointers.

#![allow(unsafe_code)]

use super::kernel::{self, Plain, S1Lut, Sym};

/// A SIMD level of the kernels. Every level gives the same output bytes.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SimdLevel {
    /// The kernels as compiled.
    Generic,
    /// x86 or x86-64 with AVX2.
    Avx2,
    /// x86 or x86-64 with AVX2 and FMA (fused only in stages proven exact).
    Avx2Fma,
}

impl SimdLevel {
    /// The levels this CPU can run, lowest first.
    pub fn available() -> Vec<SimdLevel> {
        let top = Isa::detect().0;
        [SimdLevel::Generic, SimdLevel::Avx2, SimdLevel::Avx2Fma]
            .into_iter()
            .filter(|&l| l <= top)
            .collect()
    }

    fn parse(s: &str) -> Option<SimdLevel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "generic" | "scalar" => Some(SimdLevel::Generic),
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
    if level > Isa::detect().0 {
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
    /// The best level this CPU has.
    fn detect() -> Isa {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::arch::is_x86_feature_detected!("avx2") {
            if std::arch::is_x86_feature_detected!("fma") {
                return Isa(SimdLevel::Avx2Fma);
            }
            return Isa(SimdLevel::Avx2);
        }
        Isa(SimdLevel::Generic)
    }

    /// At most `level`.
    fn cap(self, level: SimdLevel) -> Isa {
        Isa(self.0.min(level))
    }

    /// The level to use now: the detected one, capped by the environment variable
    /// `DSD2PCM_SIMD` (`generic`, `avx2` or `avx2fma`) and by [`with_simd_level`].
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
// `Isa::detect` found AVX2 on this CPU, and at `Avx2Fma` only if it also found FMA;
// `Isa::cap` only lowers a level. Each block calls one `#[target_feature]` wrapper whose
// features the matched level guarantees, and nothing else.

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
        _ => kernel::fir1::<{ kernel::L }, Plain>(s, x, out),
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
        _ => kernel::interp2_odd::<{ kernel::L }, Plain>(s, w, out),
    }
}

/// [`kernel::quantise`].
pub(crate) fn quantise(isa: Isa, v: &[f64], gain: f64, q: &mut [i32]) -> u64 {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::quantise(v, gain, q) },
        _ => kernel::quantise(v, gain, q),
    }
}

/// [`kernel::pack24`].
pub(crate) fn pack24(isa: Isa, q: &[&[i32]], out: &mut [u8]) {
    match isa.0 {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        // SAFETY: the level guarantees AVX2 (see above).
        SimdLevel::Avx2 | SimdLevel::Avx2Fma => unsafe { avx2::pack24(q, out) },
        _ => kernel::pack24(q, out),
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

/// The FIR kernels built with AVX2 and FMA, with fused multiply-adds. Used only for stages
/// with `Sym::fused`; callers must have checked that the CPU has AVX2 and FMA. The only
/// `mul_add` of the crate is here, so no other build can fuse.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx2fma {
    use super::kernel::{self, Mac, Sym};

    /// Outputs per block: 8 vectors of AVX, 8 independent chains to cover the FMA latency.
    const L: usize = 32;

    /// A fused multiply-add (one rounding).
    struct Fused;

    impl Mac for Fused {
        #[inline(always)]
        fn mac(acc: f64, h: f64, x: f64) -> f64 {
            h.mul_add(x, acc)
        }
    }

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
