// SPDX-License-Identifier: MIT OR Apache-2.0
//! A pass of outputs computed as 4 lanes, for plans whose every stage `super::exact` proved
//! exact (`Sym::fused`). The stages and their integers are those of the ordered kernels
//! (`super::kernel`); the terms of each output are added in another order and grouping,
//! which gives the same sum only because every product and partial sum is an exact integer
//! below 2^53.
//!
//! The pass's outputs are split into 4 runs of consecutive outputs, and each run goes
//! through S1 and every stage on its own, as `Plan::pass_run` would: lane `w` of every
//! vector belongs to run `w`. Each stage is then an FIR over vectors, all loads are aligned,
//! decimation and interpolation are index arithmetic on whole vectors, and nothing is
//! copied between stages. Within a group of `SV` output vectors, one loaded input vector
//! serves every (output, tap) pair with the same sum of indices: a group of `U` symmetric
//! tap pairs loads `U + SV - 1` vectors per side instead of `2 U SV`. Each group's products
//! go into a partial sum that is added to the output's sum, so that groups do not wait for
//! each other's multiply-adds.
//!
//! Used on x86-64 with AVX2 and FMA (`super::dispatch`); elsewhere the ordered kernels run.

#![cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]

use super::kernel::{round_even, s1, Mac, S1Lut, Sym};
use super::Op;

/// Lanes per vector: runs per pass.
pub(super) const W: usize = 4;

/// One vector: lane `w` belongs to run `w`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(align(32))]
pub(super) struct Lanes(pub(super) [f64; W]);

/// Output vectors per group.
const SV: usize = 4;

/// Tap pairs per group.
const U: usize = 4;

/// Input vectors per side of a group.
const N: usize = U + SV - 1;

/// The largest tap offset the lane kernels take.
pub(super) const MAX_M: usize = 128;

/// Whether the lane kernels take stage `s`.
pub(super) fn takes(s: &Sym) -> bool {
    s.fused && (1..=MAX_M).contains(&s.half_len())
}

/// The lane buffers of one worker, reused across passes.
#[derive(Debug, Default)]
pub(super) struct Bufs {
    a: Vec<Lanes>,
    b: Vec<Lanes>,
}

/// One pass: what `Plan::pass_run` computes for each run.
pub(super) struct Pass<'a> {
    pub lut: &'a S1Lut,
    /// The same S1 as paired tables.
    pub pair: &'a S1Pair,
    pub ops: &'a [Op],
    /// The bytes of run `w` start at `bytes[starts[w]]`.
    pub bytes: &'a [u8],
    pub starts: [usize; W],
    /// The length of every level of a run, as `Plan::levels` gives it (padded).
    pub lens: &'a [usize],
}

/// Index bits of the paired S1 tables.
pub(super) const P_BITS: usize = 11;

/// One paired S1 table: two partial sums per chunk value, biased to be non-negative, in the
/// low and high 32 bits.
type PTable = [u64; 1 << P_BITS];

/// S1 tables for two outputs at once: the windows of outputs `k` and `k + 1` (one byte
/// later) are read as one window of `bytes + 1` bytes, cut into chunks as `S1Lut` cuts its
/// window, and each table entry holds the chunk's contribution to both outputs. Each
/// contribution is stored plus a bias (the table's largest absolute contribution), so that
/// the 32-bit halves are never negative; the sum of a window's entries keeps them apart (no
/// half reaches 2^32: [`S1Pair::worst`], checked by `sums_are_exact`), and subtracting the
/// biases gives the same integers as `S1Lut`. One set of lookups thus gives two outputs.
#[derive(Debug)]
pub(super) struct S1Pair {
    /// Bytes per window of one output.
    pub bytes: usize,
    /// One table per chunk of the joint window.
    tables: Vec<PTable>,
    /// The sum of the biases of each half.
    bias: [i64; 2],
}

impl S1Pair {
    /// The tables for windows of `bytes` bytes whose sample `i` (0 .. 8 `bytes`, in time
    /// order: bit 7 of byte 0 first) has the tap `tap(i)`.
    pub fn new(bytes: usize, tap: impl Fn(usize) -> i32) -> S1Pair {
        let joint = bytes + 1;
        let n = 8 * bytes;
        // The taps of sample `i` of the joint window in the two outputs.
        let h = |i: usize| -> [i64; 2] {
            [
                if i < n { i64::from(tap(i)) } else { 0 },
                if (8..n + 8).contains(&i) {
                    i64::from(tap(i - 8))
                } else {
                    0
                },
            ]
        };
        let mut chunks = Vec::new();
        for w in (0..joint).step_by(8) {
            let bits = 8 * (joint - w).min(8);
            for s in (0..bits).step_by(P_BITS) {
                chunks.push((8 * w + s, (bits - s).min(P_BITS)));
            }
        }
        let mut bias = [0i64; 2];
        let tables = chunks
            .into_iter()
            .map(|(first, bits)| {
                let sums: Vec<[i64; 2]> = (0..1usize << P_BITS)
                    .map(|v| {
                        let mut s = [0i64; 2];
                        for b in 0..bits {
                            // Bit `p` of byte `j` of the word is sample `8 j + 7 - p`.
                            let pos = first + b;
                            let t = h(8 * (pos / 8) + 7 - pos % 8);
                            let sign = if (v >> b) & 1 == 1 { 1 } else { -1 };
                            s[0] += sign * t[0];
                            s[1] += sign * t[1];
                        }
                        s
                    })
                    .collect();
                let b: [i64; 2] =
                    std::array::from_fn(|k| sums.iter().map(|s| s[k].abs()).max().unwrap());
                bias[0] += b[0];
                bias[1] += b[1];
                let mut t = [0u64; 1 << P_BITS];
                for (e, s) in t.iter_mut().zip(&sums) {
                    let lo = u64::try_from(s[0] + b[0]).unwrap();
                    let hi = u64::try_from(s[1] + b[1]).unwrap();
                    *e = lo | hi << 32;
                }
                t
            })
            .collect();
        S1Pair {
            bytes,
            tables,
            bias,
        }
    }

    /// The largest value a half of a window's sum can reach: twice its bias.
    #[cfg(test)]
    pub fn worst(&self) -> [i64; 2] {
        [2 * self.bias[0], 2 * self.bias[1]]
    }

    /// Bytes of tables.
    #[cfg(test)]
    pub fn size(&self) -> usize {
        std::mem::size_of_val(&self.tables[..])
    }
}

/// `out[k].0[w]` for `k` in pairs: S1 over `bytes[k ..]` (`B` = window bytes + 1, `T`
/// tables).
#[inline(always)]
fn s1_pairs_n<const B: usize, const T: usize>(
    t: &[PTable; T],
    bias: [i64; 2],
    bytes: &[u8],
    out: &mut [Lanes],
    w: usize,
) {
    for (k, o) in out.chunks_exact_mut(2).enumerate() {
        let win: &[u8; B] = bytes[2 * k..2 * k + B].try_into().unwrap();
        let mut sum = 0u64;
        let mut i = 0;
        for part in win.chunks(8) {
            let mut word = [0u8; 8];
            word[..part.len()].copy_from_slice(part);
            let word = u64::from_le_bytes(word);
            let mut s = 0;
            while s < 8 * part.len() {
                sum = sum.wrapping_add(t[i][((word >> s) as usize) & ((1 << P_BITS) - 1)]);
                i += 1;
                s += P_BITS;
            }
        }
        o[0].0[w] = (i64::from(sum as u32) - bias[0]) as f64;
        o[1].0[w] = (i64::from((sum >> 32) as u32) - bias[1]) as f64;
    }
}

/// The number of tables of a joint window of `joint` bytes.
const fn pair_tables(joint: usize) -> usize {
    let mut n = 0;
    let mut w = 0;
    while w < joint {
        let bits = 8 * if joint - w < 8 { joint - w } else { 8 };
        n += bits.div_ceil(P_BITS);
        w += 8;
    }
    n
}

/// `out[k].0[w]` = the S1 sum over the window `bytes[starts[w] + k ..]` (`out.len()` even),
/// two outputs per set of lookups.
#[inline(always)]
fn s1_lanes(p: &Pass<'_>, out: &mut [Lanes]) {
    const T9: usize = pair_tables(9);
    const T13: usize = pair_tables(13);
    let pr = p.pair;
    for w in 0..W {
        let bytes = &p.bytes[p.starts[w]..];
        if let (8, Ok(t)) = (pr.bytes, <&[PTable; T9]>::try_from(&pr.tables[..])) {
            s1_pairs_n::<9, T9>(t, pr.bias, bytes, out, w);
        } else if let (12, Ok(t)) = (pr.bytes, <&[PTable; T13]>::try_from(&pr.tables[..])) {
            s1_pairs_n::<13, T13>(t, pr.bias, bytes, out, w);
        } else {
            s1(p.lut, bytes, 0, 1, out.iter_mut().map(|v| &mut v.0[w]));
        }
    }
}

/// `acc[g] += h[j] (X[hi + g + j] + X[lo + g - j])` over `j`, with `X[k] = x[S k]`.
#[inline(always)]
fn pairs<M: Mac, const S: usize>(
    acc: &mut [[f64; W]; SV],
    h: &[f64],
    x: &[Lanes],
    hi: usize,
    lo: usize,
) {
    /// `G` tap pairs from `j`: `up[i]` = `X[hi + j + i]`, `dn[i]` = `X[lo - j - (G - 1) + i]`.
    #[inline(always)]
    fn group<M: Mac, const S: usize, const G: usize, const NG: usize>(
        acc: &mut [[f64; W]; SV],
        h: &[f64],
        up: &[Lanes],
        dn: &[Lanes],
    ) {
        let h: &[f64; G] = h[..G].try_into().unwrap();
        let up: [Lanes; NG] = std::array::from_fn(|i| up[S * i]);
        let dn: [Lanes; NG] = std::array::from_fn(|i| dn[S * i]);
        for (g, acc) in acc.iter_mut().enumerate() {
            let mut p = [0.0; W];
            for u in 0..G {
                let (a, b) = (up[u + g].0, dn[g + G - 1 - u].0);
                for l in 0..W {
                    p[l] = if u == 0 {
                        h[0] * (a[l] + b[l])
                    } else {
                        M::mac(p[l], h[u], a[l] + b[l])
                    };
                }
            }
            for l in 0..W {
                acc[l] += p[l];
            }
        }
    }
    let t = h.len();
    let mut j = 0;
    while j + U <= t {
        let d = lo + 1 - j - U;
        group::<M, S, U, N>(
            acc,
            &h[j..],
            &x[S * (hi + j)..][..S * (N - 1) + 1],
            &x[S * d..][..S * (N - 1) + 1],
        );
        j += U;
    }
    while j < t {
        group::<M, S, 1, SV>(
            acc,
            &h[j..],
            &x[S * (hi + j)..][..S * (SV - 1) + 1],
            &x[S * (lo - j)..][..S * (SV - 1) + 1],
        );
        j += 1;
    }
}

/// `acc[g] = c X[c + g]` with `X[k] = x[S k]`.
#[inline(always)]
fn centre<const S: usize>(c0: f64, x: &[Lanes], c: usize) -> [[f64; W]; SV] {
    std::array::from_fn(|g| {
        let x = x[S * (c + g)].0;
        std::array::from_fn(|l| c0 * x[l])
    })
}

/// `out[i + g]` = `acc[g]` rescaled and rounded.
#[inline(always)]
fn store(acc: &[[f64; W]; SV], scale: f64, out: &mut [Lanes]) {
    for (o, a) in out[..SV].iter_mut().zip(acc) {
        o.0 = std::array::from_fn(|l| round_even(a[l] * scale));
    }
}

/// Fir1 over vectors: `out[i]` centred on `x[i + m]` (as `kernel::fir1`).
#[inline(always)]
fn fir1<M: Mac>(s: &Sym, x: &[Lanes], out: &mut [Lanes]) {
    let m = s.half_len();
    let h = &s.full[1..];
    for (bi, o) in out.chunks_exact_mut(SV).enumerate() {
        let i = bi * SV;
        let mut acc = centre::<1>(s.full[0], x, i + m);
        pairs::<M, 1>(&mut acc, h, x, i + m + 1, i + m - 1);
        store(&acc, s.scale, o);
    }
}

/// Dec2 over vectors (as `kernel::dec2`), with `xe[k]` = `xe0[S k]`, `xo[k]` = `xo0[S k]`.
#[inline(always)]
fn dec2<M: Mac, const S: usize>(s: &Sym, xe: &[Lanes], xo: &[Lanes], out: &mut [Lanes]) {
    let (e, o) = (s.even.len() - 1, s.odd.len());
    let he = &s.even[1..];
    for (bi, d) in out.chunks_exact_mut(SV).enumerate() {
        let i = bi * SV;
        let mut acc = centre::<S>(s.even[0], xe, i + e);
        if e > 0 {
            pairs::<M, S>(&mut acc, he, xe, i + e + 1, i + e - 1);
        }
        pairs::<M, S>(&mut acc, &s.odd, xo, i + o, i + o - 1);
        store(&acc, s.scale, d);
    }
}

/// Interp2 over vectors: `out[2 i]` = `w[i + o]`, `out[2 i + 1]` between `w[i + o]` and
/// `w[i + o + 1]` (as `kernel::interp2_odd` and `Plan::pass_run`).
#[inline(always)]
fn interp2<M: Mac>(s: &Sym, w: &[Lanes], out: &mut [Lanes]) {
    let o = s.odd.len();
    for (bi, d) in out.chunks_exact_mut(2 * SV).enumerate() {
        let i = bi * SV;
        let mut acc = [[0.0; W]; SV];
        pairs::<M, 1>(&mut acc, &s.odd, w, i + o + 1, i + o);
        let mut odd = [Lanes::default(); SV];
        store(&acc, s.scale * 2.0, &mut odd);
        for (g, d) in d.chunks_exact_mut(2).enumerate() {
            d[0] = w[i + o + g];
            d[1] = odd[g];
        }
    }
}

/// `n` rounded up to whole groups and blocks: `pad` is `Plan`'s block (a multiple of `SV`).
fn up(n: usize, pad: usize) -> usize {
    n.div_ceil(pad) * pad
}

/// Runs the pass; the outputs of run `w` end up in lane `w` of `bufs.a[..lens[ops.len()]]`,
/// which is returned. `pad` is `Plan`'s block size.
#[inline(always)]
pub(super) fn run<'b, M: Mac>(p: &Pass<'_>, pad: usize, bufs: &'b mut Bufs) -> &'b [Lanes] {
    debug_assert!(pad % (2 * SV) == 0);
    let Bufs { a, b } = bufs;
    // S1 at every byte: the first stage takes its even and odd inputs from it as later
    // stages do.
    a.resize(up(p.lens[0], pad), Lanes::default());
    s1_lanes(p, a);
    for (k, op) in p.ops.iter().enumerate() {
        let n_out = p.lens[k + 1];
        match op {
            Op::Fir1(s) => {
                b.resize(up(n_out, pad), Lanes::default());
                fir1::<M>(s, a, b);
            }
            Op::Dec2(s) => {
                let m = s.half_len();
                let (e, o) = (s.even.len() - 1, s.odd.len());
                let (de, dodd) = (m - 2 * e, m + 1 - 2 * o);
                b.resize(up(n_out, pad), Lanes::default());
                dec2::<M, 2>(s, &a[de..], &a[dodd..], b);
            }
            Op::Interp2(s) => {
                let half = up(n_out / 2, pad);
                b.resize(2 * half, Lanes::default());
                interp2::<M>(s, a, b);
            }
        }
        std::mem::swap(a, b);
    }
    &a[..p.lens[p.ops.len()]]
}
