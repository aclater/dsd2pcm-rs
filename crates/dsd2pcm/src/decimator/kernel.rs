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

/// Outputs per block of the generic Fir1 kernel: [`L`], but 8 on 32-bit x86, whose SSE2 has
/// 8 vector registers instead of 16, so that the sums and a step's operands stay in
/// registers (as in scarletbook-pcm). Dec2 and Interp2 keep [`L`].
pub(super) const L_FIR1: usize = if cfg!(target_arch = "x86") { 8 } else { L };

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

/// The layout of the S1 tables: how a window is read and cut into chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum S1Layout {
    /// 64-bit words cut into 13-bit chunks: 5 lookups for S1's 8-byte window.
    Wide,
    /// 32-bit words cut into bytes, as in scarletbook-pcm: 8 lookups, but 8 KiB of tables
    /// for S1's window, which stay in the level-1 cache of small in-order cores (which stall
    /// on every table read from the level-2 cache), and no 64-bit shifts (a register pair
    /// and two instructions on 32-bit targets). Used on 32-bit targets at the levels below
    /// AVX2.
    Byte,
}

impl S1Layout {
    /// Bytes per word.
    const fn word(self) -> usize {
        match self {
            S1Layout::Wide => 8,
            S1Layout::Byte => 4,
        }
    }

    /// Index bits per chunk.
    const fn bits(self) -> usize {
        match self {
            S1Layout::Wide => WIDE_BITS,
            S1Layout::Byte => 8,
        }
    }
}

/// Index bits of the [`S1Layout::Wide`] tables.
const WIDE_BITS: usize = 13;

/// The S1 tables of one layout: a partial sum for each value of a chunk.
#[derive(Debug)]
pub(super) enum S1Tables {
    Wide(Vec<[i32; 1 << WIDE_BITS]>),
    Byte(Vec<[i32; 256]>),
}

/// The S1 lookup tables of a window of `bytes` bytes. The window is read as little-endian
/// words (the last one may be shorter), and each word is cut, from its low bit up, into
/// chunks (the last chunk of a word may be shorter), as the layout says. Table `k` maps the
/// value of chunk `k` to the sum over its samples of `+tap` for a 1 bit and `-tap` for a 0
/// bit. scarletbook-pcm uses one table per byte; wider chunks need fewer lookups, and the
/// sum is the same integer: no partial sum wraps (the sum of the absolute taps is below
/// 2^31, `super::tests`).
#[derive(Debug)]
pub(super) struct S1Lut {
    /// Bytes per window.
    pub bytes: usize,
    /// The layout.
    pub layout: S1Layout,
    /// One table per chunk, in window order.
    pub tables: S1Tables,
}

impl S1Lut {
    /// The chunks of a window of `bytes` bytes: (first bit in the window, bits).
    pub fn chunks(bytes: usize, layout: S1Layout) -> Vec<(usize, usize)> {
        let (word, cut) = (layout.word(), layout.bits());
        let mut v = Vec::new();
        for w in (0..bytes).step_by(word) {
            let bits = 8 * (bytes - w).min(word);
            for s in (0..bits).step_by(cut) {
                v.push((8 * w + s, (bits - s).min(cut)));
            }
        }
        v
    }

    /// The tables of a window of `bytes` bytes whose sample `i` (0 .. 8 `bytes`, in time
    /// order: bit 7 of byte 0 first) has the tap `tap(i)`.
    pub fn new(bytes: usize, layout: S1Layout, tap: impl Fn(usize) -> i32) -> S1Lut {
        fn build<const N: usize>(
            chunks: Vec<(usize, usize)>,
            tap: impl Fn(usize) -> i32,
        ) -> Vec<[i32; N]> {
            chunks
                .into_iter()
                .map(|(first, bits)| {
                    let mut t = [0i32; N];
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
                .collect()
        }
        let chunks = S1Lut::chunks(bytes, layout);
        let tables = match layout {
            S1Layout::Wide => S1Tables::Wide(build(chunks, tap)),
            S1Layout::Byte => S1Tables::Byte(build(chunks, tap)),
        };
        S1Lut {
            bytes,
            layout,
            tables,
        }
    }

    /// The tables, each as a slice.
    #[cfg(test)]
    pub fn table_slices(&self) -> Vec<&[i32]> {
        match &self.tables {
            S1Tables::Wide(t) => t.iter().map(|t| &t[..]).collect(),
            S1Tables::Byte(t) => t.iter().map(|t| &t[..]).collect(),
        }
    }
}

/// S1: `out[k]` = the S1 sum over the window `bytes[start + stride k ..]`.
#[inline(always)]
pub(super) fn s1<'a, O>(lut: &S1Lut, bytes: &[u8], start: usize, stride: usize, out: O)
where
    O: IntoIterator<Item = &'a mut f64>,
    O::IntoIter: ExactSizeIterator,
{
    // S1's 8-byte window and the wide filter's 12-byte one, and the precise filter's 12 and
    // 16 bytes, with a constant table count, so that the loops unroll.
    const W8: usize = s1_tables(8, S1Layout::Wide);
    const W12: usize = s1_tables(12, S1Layout::Wide);
    const W16: usize = s1_tables(16, S1Layout::Wide);
    const B8: usize = s1_tables(8, S1Layout::Byte);
    const B12: usize = s1_tables(12, S1Layout::Byte);
    const B16: usize = s1_tables(16, S1Layout::Byte);
    match (&lut.tables, lut.bytes) {
        (S1Tables::Wide(t), 8) if t.len() == W8 => {
            s1_n::<u64, WIDE_BITS, { 1 << WIDE_BITS }, 8, W8, _>(
                t[..].try_into().unwrap(),
                bytes,
                start,
                stride,
                out,
            )
        }
        (S1Tables::Wide(t), 12) if t.len() == W12 => {
            s1_n::<u64, WIDE_BITS, { 1 << WIDE_BITS }, 12, W12, _>(
                t[..].try_into().unwrap(),
                bytes,
                start,
                stride,
                out,
            )
        }
        (S1Tables::Wide(t), 16) if t.len() == W16 => {
            s1_n::<u64, WIDE_BITS, { 1 << WIDE_BITS }, 16, W16, _>(
                t[..].try_into().unwrap(),
                bytes,
                start,
                stride,
                out,
            )
        }
        (S1Tables::Byte(t), 8) if t.len() == B8 => {
            s1_n::<u32, 8, 256, 8, B8, _>(t[..].try_into().unwrap(), bytes, start, stride, out)
        }
        (S1Tables::Byte(t), 12) if t.len() == B12 => {
            s1_n::<u32, 8, 256, 12, B12, _>(t[..].try_into().unwrap(), bytes, start, stride, out)
        }
        (S1Tables::Byte(t), 16) if t.len() == B16 => {
            s1_n::<u32, 8, 256, 16, B16, _>(t[..].try_into().unwrap(), bytes, start, stride, out)
        }
        _ => s1_any(lut, bytes, start, stride, out),
    }
}

/// The number of S1 tables (chunks) of a window of `bytes` bytes.
const fn s1_tables(bytes: usize, layout: S1Layout) -> usize {
    let (word, cut) = (layout.word(), layout.bits());
    let mut n = 0;
    let mut w = 0;
    while w < bytes {
        let bits = 8 * if bytes - w < word { bytes - w } else { word };
        n += bits.div_ceil(cut);
        w += word;
    }
    n
}

/// A machine word that S1 reads a window in.
trait S1Word: Copy {
    /// Its size in bytes.
    const BYTES: usize;
    /// `part` (at most `BYTES` bytes) as a little-endian word, zero above it.
    fn load(part: &[u8]) -> Self;
    /// The word shifted right by `s` bits, as an index.
    fn index(self, s: usize) -> usize;
}

macro_rules! s1_word {
    ($t:ty) => {
        impl S1Word for $t {
            const BYTES: usize = std::mem::size_of::<$t>();
            #[inline(always)]
            fn load(part: &[u8]) -> Self {
                let mut w = [0u8; std::mem::size_of::<$t>()];
                w[..part.len()].copy_from_slice(part);
                <$t>::from_le_bytes(w)
            }
            #[inline(always)]
            fn index(self, s: usize) -> usize {
                (self >> s) as usize
            }
        }
    };
}
s1_word!(u32);
s1_word!(u64);

/// [`s1`] with a window of `B` bytes and `T` tables of `BITS`-bit chunks (`N` = 2^`BITS`
/// entries) of words `W`: each
/// window is read as words and cut into chunks, with the loops unrolled.
#[inline(always)]
fn s1_n<'a, W: S1Word, const BITS: usize, const N: usize, const B: usize, const T: usize, O>(
    t: &[[i32; N]; T],
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
        for part in w.chunks(W::BYTES) {
            let word = W::load(part);
            let mut s = 0;
            while s < 8 * part.len() {
                let i = word.index(s) & (N - 1);
                sum = sum.wrapping_add(t[k][i]);
                k += 1;
                s += BITS;
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
    let chunks = S1Lut::chunks(lut.bytes, lut.layout);
    let word = lut.layout.word();
    let entry = |k: usize, i: usize| match &lut.tables {
        S1Tables::Wide(t) => t[k][i],
        S1Tables::Byte(t) => t[k][i],
    };
    for (n, o) in out.into_iter().enumerate() {
        let w = &bytes[start + stride * n..][..lut.bytes];
        let sum = chunks
            .iter()
            .enumerate()
            .fold(0i32, |acc, (k, &(first, bits))| {
                let at = first / (8 * word) * word;
                let v = u64::load(&w[at..(at + word).min(w.len())]);
                let i = (v.index(first % (8 * word))) & ((1 << bits) - 1);
                acc.wrapping_add(entry(k, i))
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
    /// Whether a fused multiply-add may be used: set by the plan only when
    /// `super::exact` proved every sum of this stage exact.
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
