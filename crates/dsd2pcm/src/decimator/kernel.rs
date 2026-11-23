// SPDX-License-Identifier: MIT OR Apache-2.0
//! The filter kernels, ported from scarletbook-pcm 0.7.0 (src/kernel.rs).
//!
//! Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR Apache-2.0 (see
//! PROVENANCE.md). Ported: the S1 table lookup read in machine words, and the Fir1, Dec2
//! and Interp2 kernels, which compute a block of `L` independent outputs at a time so that
//! the compiler puts the block in vector lanes. Every product and partial sum is an exact
//! integer below 2^53 (`super::tests::sums_are_exact`), so the order of the additions and
//! the vector width cannot change a result.

/// 1.5 x 2^52: adding and subtracting it rounds to an integer, half to even.
const MAGIC: f64 = 6_755_399_441_055_744.0;

/// `x` rounded to an integer, half to even; exact for `|x| < 2^51`. Vectorises.
#[inline(always)]
pub(super) fn round_even(x: f64) -> f64 {
    (x + MAGIC) - MAGIC
}

/// The integer-valued `x`, `-2^31 <= x < 2^31`, as `i32`. Unlike `x as i32`, which has to
/// handle NaN and saturate, it vectorises.
#[inline(always)]
pub(super) fn to_i32(x: f64) -> i32 {
    (x + MAGIC).to_bits() as i32
}

/// Outputs per block: 4 vectors of SSE2, 2 of AVX, 8 of NEON.
pub(super) const L: usize = 16;

/// S1: `out[k]` = the sum of `lut[j][bytes[start + stride k + j]]`.
pub(super) fn s1(lut: &[[i32; 256]], bytes: &[u8], start: usize, stride: usize, out: &mut [f64]) {
    if let Ok(lut) = <&[[i32; 256]; 8]>::try_from(lut) {
        s1_n::<8>(lut, bytes, start, stride, out);
    } else if let Ok(lut) = <&[[i32; 256]; 12]>::try_from(lut) {
        s1_n::<12>(lut, bytes, start, stride, out);
    } else {
        for (k, o) in out.iter_mut().enumerate() {
            let i = start + stride * k;
            let sum = lut
                .iter()
                .zip(&bytes[i..i + lut.len()])
                .fold(0i32, |acc, (t, &x)| acc.wrapping_add(t[usize::from(x)]));
            *o = f64::from(sum);
        }
    }
}

/// `u64` little-endian word from up to 8 bytes, zero above them.
#[inline(always)]
fn load(part: &[u8]) -> u64 {
    let mut w = [0u8; 8];
    w[..part.len()].copy_from_slice(part);
    u64::from_le_bytes(w)
}

/// [`s1`] with `B` tables, each window read as 64-bit words and taken apart byte by byte.
/// The word changes only how the bytes are fetched, not which entries are added. No partial
/// sum wraps: the sum of the absolute taps is below 2^31.
#[inline(always)]
fn s1_n<const B: usize>(
    lut: &[[i32; 256]; B],
    bytes: &[u8],
    start: usize,
    stride: usize,
    out: &mut [f64],
) {
    let Some(last) = out.len().checked_sub(1) else {
        return;
    };
    let windows = bytes[start..start + stride * last + B]
        .windows(B)
        .step_by(stride);
    for (o, w) in out.iter_mut().zip(windows) {
        let mut sum = 0i32;
        for (tables, part) in lut.chunks(8).zip(w.chunks(8)) {
            let mut word = load(part);
            for table in tables {
                sum = sum.wrapping_add(table[(word & 0xff) as usize]);
                word >>= 8;
            }
        }
        *o = f64::from(sum);
    }
}

/// Adds `h[j] (x[hi + j + l] + x[lo - j + l])` over `j`, for the `L` outputs `l`.
#[inline(always)]
fn add_pairs(acc: &mut [f64; L], h: &[f64], x: &[f64], hi: usize, lo: usize) {
    let Some(last) = h.len().checked_sub(1) else {
        return;
    };
    let up = x[hi..hi + last + L].windows(L);
    let down = x[lo - last..lo + L].windows(L).rev();
    for ((a, b), &hj) in up.zip(down).zip(h) {
        for l in 0..L {
            acc[l] += hj * (a[l] + b[l]);
        }
    }
}

/// Runs `f` on each block of `L` outputs and stores the rounded, rescaled sums. `out.len()`
/// is a multiple of `L`.
#[inline(always)]
fn blocks(out: &mut [f64], scale: f64, f: impl Fn(usize, &mut [f64; L])) {
    for (bi, o) in out.chunks_exact_mut(L).enumerate() {
        let mut acc = [0.0f64; L];
        f(bi * L, &mut acc);
        for (d, a) in o.iter_mut().zip(acc) {
            *d = round_even(a * scale);
        }
    }
}

/// A symmetric filter: taps from the centre out, split into even and odd offsets.
#[derive(Clone, Debug)]
pub(super) struct Sym {
    /// Taps from the centre out.
    pub full: Vec<f64>,
    /// Taps at even offsets 0, 2, 4, ... (trailing zeros removed).
    pub even: Vec<f64>,
    /// Taps at odd offsets 1, 3, 5, ...
    pub odd: Vec<f64>,
    /// 2^-q: from the product scale back to the sample grid.
    pub scale: f64,
}

impl Sym {
    pub fn new(half: &[i32], scale: f64) -> Sym {
        let full: Vec<f64> = half.iter().map(|&h| f64::from(h)).collect();
        let mut even: Vec<f64> = full.iter().step_by(2).copied().collect();
        while even.len() > 1 && even.last() == Some(&0.0) {
            even.pop();
        }
        let odd = full.iter().skip(1).step_by(2).copied().collect();
        Sym {
            full,
            even,
            odd,
            scale,
        }
    }

    /// The largest tap offset.
    pub fn half_len(&self) -> usize {
        self.full.len() - 1
    }
}

/// Fir1: `out[i]` centred on `x[i + m]`.
pub(super) fn fir1(s: &Sym, x: &[f64], out: &mut [f64]) {
    let m = s.half_len();
    let (h0, h) = (s.full[0], &s.full[1..]);
    blocks(out, s.scale, |i, acc| {
        for (l, a) in acc.iter_mut().enumerate() {
            *a = h0 * x[i + m + l];
        }
        add_pairs(acc, h, x, i + m + 1, i + m - 1);
    });
}

/// Dec2: `out[i]` centred on `xe[i + e]` (`e` = the even taps after the centre), with the
/// odd taps on `xo[i + o + j]` and `xo[i + o - 1 - j]`.
pub(super) fn dec2(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
    let (e, o) = (s.even.len() - 1, s.odd.len());
    let (h0, he) = (s.even[0], &s.even[1..]);
    blocks(out, s.scale, |i, acc| {
        for (l, a) in acc.iter_mut().enumerate() {
            *a = h0 * xe[i + e + l];
        }
        if e > 0 {
            add_pairs(acc, he, xe, i + e + 1, i + e - 1);
        }
        add_pairs(acc, &s.odd, xo, i + o, i + o - 1);
    });
}

/// The odd outputs of Interp2: `out[i]` between `w[i + o]` and `w[i + o + 1]`. The
/// interpolation gain of 2 is in the scale.
pub(super) fn interp2_odd(s: &Sym, w: &[f64], out: &mut [f64]) {
    let o = s.odd.len();
    blocks(out, s.scale * 2.0, |i, acc| {
        add_pairs(acc, &s.odd, w, i + o + 1, i + o)
    });
}

/// Rounds `v` (on the 2^26 grid) times `gain` to 24 bits, clamped, into `q`; returns the
/// number clamped. Without branches, so that it vectorises.
pub(super) fn quantise(v: &[f64], gain: f64, q: &mut [i32]) -> u64 {
    const MAX: f64 = 8_388_607.0;
    const MIN: f64 = -8_388_608.0;
    let mut clipped = 0u64;
    for (d, &v) in q.iter_mut().zip(v) {
        let y = round_even((v * gain) * 0.125);
        let c = y.clamp(MIN, MAX);
        clipped = clipped.wrapping_add(u64::from(c != y));
        *d = to_i32(c);
    }
    clipped
}
