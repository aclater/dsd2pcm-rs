// SPDX-License-Identifier: MIT OR Apache-2.0
//! The filter kernels, ported from scarletbook-pcm 0.7.0 (src/kernel.rs).
//!
//! Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR Apache-2.0 (see
//! PROVENANCE.md). Ported: the S1 table lookup read in machine words, and the Fir1, Dec2
//! and Interp2 kernels, which compute a block of `L` independent outputs at a time so that
//! the compiler puts the block in vector lanes. Each output has its own sum over the taps in
//! a fixed order, so the block size `L` cannot change a result.
//!
//! The kernels are generic and `#[inline(always)]`; `super::dispatch` instantiates them for
//! the CPU. They are generic over the multiply-add `M` too: this module has only [`Plain`]
//! (a product and a separate addition, as in the original). A fused multiply-add rounds once
//! instead of twice, which gives the same sum only when every product and partial sum is an
//! exact integer below 2^53; `super::exact` proves that per stage from the tables, and
//! `dispatch` fuses only stages it proved.

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

/// Outputs per block of the generic kernels: 4 vectors of SSE2, 8 of NEON.
pub(super) const L: usize = 16;

/// The largest block of any instantiation; stage buffers are padded to a multiple of it.
pub(super) const MAX_L: usize = 32;

/// A multiply-add `acc + h x`.
pub(super) trait Mac {
    /// `acc + h x`.
    fn mac(acc: f64, h: f64, x: f64) -> f64;
}

/// A product and a separate addition (two roundings).
pub(super) struct Plain;

impl Mac for Plain {
    #[inline(always)]
    fn mac(acc: f64, h: f64, x: f64) -> f64 {
        acc + h * x
    }
}

/// Index bits of the S1 tables.
pub(super) const S1_BITS: usize = 13;

/// One S1 table: a partial sum for each value of a chunk of up to [`S1_BITS`] samples.
pub(super) type S1Table = [i32; 1 << S1_BITS];

/// The S1 lookup tables of a window of `bytes` bytes. The window is read as little-endian
/// 64-bit words (the last one may be shorter), and each word is cut, from its low bit up,
/// into chunks of [`S1_BITS`] bits (the last chunk of a word may be shorter). Table `k` maps
/// the value of chunk `k` to the sum over its samples of `+tap` for a 1 bit and `-tap` for a
/// 0 bit. scarletbook-pcm uses one table per byte; wider chunks need fewer lookups, and the
/// sum is the same integer: no partial sum wraps (the sum of the absolute taps is below
/// 2^31, `super::tests`).
#[derive(Debug)]
pub(super) struct S1Lut {
    /// Bytes per window.
    pub bytes: usize,
    /// One table per chunk, in window order.
    pub tables: Vec<S1Table>,
}

impl S1Lut {
    /// The chunks of a window of `bytes` bytes: (first bit in the window, bits).
    pub fn chunks(bytes: usize) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for w in (0..bytes).step_by(8) {
            let bits = 8 * (bytes - w).min(8);
            for s in (0..bits).step_by(S1_BITS) {
                v.push((8 * w + s, (bits - s).min(S1_BITS)));
            }
        }
        v
    }

    /// The tables of a window of `bytes` bytes whose sample `i` (0 .. 8 `bytes`, in time
    /// order: bit 7 of byte 0 first) has the tap `tap(i)`.
    pub fn new(bytes: usize, tap: impl Fn(usize) -> i32) -> S1Lut {
        let tables = S1Lut::chunks(bytes)
            .into_iter()
            .map(|(first, bits)| {
                let mut t = [0i32; 1 << S1_BITS];
                for (v, e) in t.iter_mut().enumerate() {
                    *e = (0..bits)
                        .map(|b| {
                            // Bit `p` of byte `j` of the word is sample `8 j + 7 - p`.
                            let pos = first + b;
                            let h = tap(8 * (pos / 8) + 7 - pos % 8);
                            if (v >> b) & 1 == 1 {
                                h
                            } else {
                                -h
                            }
                        })
                        .sum();
                }
                t
            })
            .collect();
        S1Lut { bytes, tables }
    }
}

/// S1: `out[k]` = the S1 sum over the window `bytes[start + stride k ..]`.
#[inline(always)]
pub(super) fn s1<'a, O>(lut: &S1Lut, bytes: &[u8], start: usize, stride: usize, out: O)
where
    O: IntoIterator<Item = &'a mut f64>,
    O::IntoIter: ExactSizeIterator,
{
    // S1's 8-byte window and the wide filter's 12-byte one, with a constant table count, so
    // that the loops unroll.
    const T8: usize = s1_tables(8);
    const T12: usize = s1_tables(12);
    if let (8, Ok(t)) = (lut.bytes, <&[S1Table; T8]>::try_from(&lut.tables[..])) {
        s1_n::<8, T8, _>(t, bytes, start, stride, out)
    } else if let (12, Ok(t)) = (lut.bytes, <&[S1Table; T12]>::try_from(&lut.tables[..])) {
        s1_n::<12, T12, _>(t, bytes, start, stride, out)
    } else {
        s1_any(lut, bytes, start, stride, out)
    }
}

/// The number of S1 tables (chunks) of a window of `bytes` bytes.
const fn s1_tables(bytes: usize) -> usize {
    let mut n = 0;
    let mut w = 0;
    while w < bytes {
        let bits = 8 * if bytes - w < 8 { bytes - w } else { 8 };
        n += bits.div_ceil(S1_BITS);
        w += 8;
    }
    n
}

/// `u64` little-endian word from up to 8 bytes, zero above them.
#[inline(always)]
fn load(part: &[u8]) -> u64 {
    let mut w = [0u8; 8];
    w[..part.len()].copy_from_slice(part);
    u64::from_le_bytes(w)
}

/// [`s1`] with a window of `B` bytes and `T` tables: each window is read as 64-bit words
/// and cut into chunks, with the loops unrolled.
#[inline(always)]
fn s1_n<'a, const B: usize, const T: usize, O>(
    t: &[S1Table; T],
    bytes: &[u8],
    start: usize,
    stride: usize,
    out: O,
) where
    O: IntoIterator<Item = &'a mut f64>,
    O::IntoIter: ExactSizeIterator,
{
    let out = out.into_iter();
    let Some(last) = out.len().checked_sub(1) else {
        return;
    };
    let windows = bytes[start..start + stride * last + B]
        .windows(B)
        .step_by(stride);
    for (o, w) in out.zip(windows) {
        let mut sum = 0i32;
        let mut k = 0;
        for part in w.chunks(8) {
            let word = load(part);
            let mut s = 0;
            while s < 8 * part.len() {
                let i = ((word >> s) as usize) & ((1 << S1_BITS) - 1);
                sum = sum.wrapping_add(t[k][i]);
                k += 1;
                s += S1_BITS;
            }
        }
        *o = f64::from(sum);
    }
}

/// [`s1`] for any window.
pub(super) fn s1_any<'a>(
    lut: &S1Lut,
    bytes: &[u8],
    start: usize,
    stride: usize,
    out: impl IntoIterator<Item = &'a mut f64>,
) {
    let chunks = S1Lut::chunks(lut.bytes);
    for (k, o) in out.into_iter().enumerate() {
        let w = &bytes[start + stride * k..][..lut.bytes];
        let sum = chunks
            .iter()
            .zip(&lut.tables)
            .fold(0i32, |acc, (&(first, bits), t)| {
                let word = load(&w[first / 64 * 8..(first / 64 * 8 + 8).min(w.len())]);
                let i = ((word >> (first % 64)) as usize) & ((1 << bits) - 1);
                acc.wrapping_add(t[i])
            });
        *o = f64::from(sum);
    }
}

/// Adds `h[j] (x[hi + j + l] + x[lo - j + l])` over `j`, for the `L` outputs `l`.
#[inline(always)]
fn add_pairs<const L: usize, M: Mac>(
    acc: &mut [f64; L],
    h: &[f64],
    x: &[f64],
    hi: usize,
    lo: usize,
) {
    let Some(last) = h.len().checked_sub(1) else {
        return;
    };
    let up = x[hi..hi + last + L].windows(L);
    let down = x[lo - last..lo + L].windows(L).rev();
    for ((a, b), &hj) in up.zip(down).zip(h) {
        for l in 0..L {
            acc[l] = M::mac(acc[l], hj, a[l] + b[l]);
        }
    }
}

/// Runs `f` on each block of `L` outputs and stores the rounded, rescaled sums. `out.len()`
/// is a multiple of `L`.
#[inline(always)]
fn blocks<const L: usize>(out: &mut [f64], scale: f64, f: impl Fn(usize, &mut [f64; L])) {
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
    /// Whether a fused multiply-add, or another summation order (`super::lanes`), may be
    /// used: set by the plan only when `super::exact` proved every sum of this stage exact.
    pub fused: bool,
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
            fused: false,
        }
    }

    /// The largest tap offset.
    pub fn half_len(&self) -> usize {
        self.full.len() - 1
    }
}

/// Fir1: `out[i]` centred on `x[i + m]`.
#[inline(always)]
pub(super) fn fir1<const L: usize, M: Mac>(s: &Sym, x: &[f64], out: &mut [f64]) {
    let m = s.half_len();
    let (h0, h) = (s.full[0], &s.full[1..]);
    blocks(out, s.scale, |i, acc| {
        for (l, a) in acc.iter_mut().enumerate() {
            *a = h0 * x[i + m + l];
        }
        add_pairs::<L, M>(acc, h, x, i + m + 1, i + m - 1);
    });
}

/// Dec2: `out[i]` centred on `xe[i + e]` (`e` = the even taps after the centre), with the
/// odd taps on `xo[i + o + j]` and `xo[i + o - 1 - j]`.
#[inline(always)]
pub(super) fn dec2<const L: usize, M: Mac>(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
    let (e, o) = (s.even.len() - 1, s.odd.len());
    let (h0, he) = (s.even[0], &s.even[1..]);
    blocks(out, s.scale, |i, acc| {
        for (l, a) in acc.iter_mut().enumerate() {
            *a = h0 * xe[i + e + l];
        }
        if e > 0 {
            add_pairs::<L, M>(acc, he, xe, i + e + 1, i + e - 1);
        }
        add_pairs::<L, M>(acc, &s.odd, xo, i + o, i + o - 1);
    });
}

/// The odd outputs of Interp2: `out[i]` between `w[i + o]` and `w[i + o + 1]`. The
/// interpolation gain of 2 is in the scale.
#[inline(always)]
pub(super) fn interp2_odd<const L: usize, M: Mac>(s: &Sym, w: &[f64], out: &mut [f64]) {
    let o = s.odd.len();
    blocks(out, s.scale * 2.0, |i, acc| {
        add_pairs::<L, M>(acc, &s.odd, w, i + o + 1, i + o)
    });
}

/// Rounds `v` (on the 2^26 grid) times `gain` to 24 bits, clamped, into `q`; returns the
/// number clamped. Without branches, so that it vectorises.
#[inline(always)]
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

/// Interleaves the channels `q` (equal lengths) as 24-bit little-endian samples into `out`
/// (`3 q.len() q[0].len()` bytes). The channel count is a constant in each arm, so that the
/// loop is unrolled.
#[inline(always)]
pub(super) fn pack24(q: &[&[i32]], out: &mut [u8]) {
    #[inline(always)]
    fn by<const CH: usize>(q: &[&[i32]], out: &mut [u8]) {
        let q: [&[i32]; CH] = std::array::from_fn(|c| q[c]);
        let n = q[0].len();
        for (i, d) in out.chunks_exact_mut(3 * CH).take(n).enumerate() {
            for (c, d) in d.chunks_exact_mut(3).enumerate() {
                let [b0, b1, b2, _] = q[c][i].to_le_bytes();
                d.copy_from_slice(&[b0, b1, b2]);
            }
        }
    }
    match q.len() {
        0 => {}
        1 => by::<1>(q, out),
        2 => by::<2>(q, out),
        3 => by::<3>(q, out),
        4 => by::<4>(q, out),
        5 => by::<5>(q, out),
        6 => by::<6>(q, out),
        ch => {
            for (i, d) in out.chunks_exact_mut(3 * ch).enumerate() {
                for (c, d) in d.chunks_exact_mut(3).enumerate() {
                    d.copy_from_slice(&q[c][i].to_le_bytes()[..3]);
                }
            }
        }
    }
}
