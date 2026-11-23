// SPDX-License-Identifier: MIT OR Apache-2.0
//! Staggered versions of the Fir1, Dec2 and Interp2 kernels, for stages whose sums
//! `super::exact` proved exact (`Sym::fused`). They add the terms of each output in another
//! order and grouping than [`super::kernel`], which gives the same sum only because every
//! product and partial sum is an exact integer below 2^53.
//!
//! A block of `4 S` outputs is computed as 4 lanes of `S` consecutive outputs each: lane `w`
//! holds outputs `w S .. (w + 1) S`. The input is copied once per block into vectors
//! `y[k] = (x[k], x[k + S], x[k + 2 S], x[k + 3 S])` (`stagger`), so that the input of output
//! `v` of every lane at tap offset `j` is the one aligned vector `y[v + j]`. Within a group of
//! `SV` output vectors, a vector `y[k]` serves every pair (output `v`, tap `j`) with
//! `v + j = k`: a group of `ST_U` taps loads `ST_U + SV - 1` vectors per side instead of
//! `2 ST_U SV`. Each group's `ST_U` products are summed into a partial sum that is then added
//! to the output's sum, so that consecutive groups do not wait for each other.
//!
//! Used on x86-64 with AVX2 and FMA only (`super::dispatch`); elsewhere the ordered kernels
//! run, which avoids spilling on targets with fewer vector registers.

#![cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]

use super::kernel::{dec2, fir1, interp2_odd, round_even, Mac, Sym};

/// Lanes of the staggered kernels.
const SW: usize = 4;

/// One vector of staggered input or output: lane `w` belongs to lane block `w`.
#[derive(Clone, Copy, Debug)]
#[repr(align(32))]
struct Lanes([f64; SW]);

/// Output vectors per group.
const SV: usize = 4;

/// Taps per group.
const ST_U: usize = 4;

/// Vectors per side of a group: `ST_U + SV - 1`.
const ST_N: usize = ST_U + SV - 1;

/// Outputs per lane of a full block.
const SS: usize = 256;

/// The largest tap offset the staggered kernels take; stages with longer filters use the
/// ordered kernels.
const ST_MAX: usize = 128;

/// Input vectors per staggered block.
const ST_Y: usize = SS + 2 * ST_MAX;

/// Outputs per lane of the block from output `i` of `n`: a multiple of `SV`, 0 when fewer
/// than `SW SV` outputs are left.
#[inline(always)]
fn st_block(n: usize, i: usize) -> usize {
    ((n - i) / SW).min(SS) / SV * SV
}

/// `y[k].0[w] = x[a + w s + k]`.
#[inline(always)]
fn stagger(x: &[f64], a: usize, s: usize, y: &mut [Lanes]) {
    let n = y.len();
    let (x0, x1, x2, x3) = (
        &x[a..][..n],
        &x[a + s..][..n],
        &x[a + 2 * s..][..n],
        &x[a + 3 * s..][..n],
    );
    for ((((d, &a), &b), &c), &e) in y.iter_mut().zip(x0).zip(x1).zip(x2).zip(x3) {
        d.0 = [a, b, c, e];
    }
}

/// `acc[g] += h[j] (y[hi + g + j] + y[lo + g - j])` over `j`, in groups of `ST_U` taps.
#[inline(always)]
fn st_pairs<M: Mac>(acc: &mut [[f64; SW]; SV], h: &[f64], y: &[Lanes], hi: usize, lo: usize) {
    /// One group of `U` taps: `up[i]` = `y[hi + j + i]`, `dn[i]` = `y[lo - j - (U - 1) + i]`.
    #[inline(always)]
    fn group<M: Mac, const U: usize, const N: usize>(
        acc: &mut [[f64; SW]; SV],
        h: &[f64; U],
        up: &[Lanes; N],
        dn: &[Lanes; N],
    ) {
        for (g, acc) in acc.iter_mut().enumerate() {
            let mut p = [0.0; SW];
            for u in 0..U {
                let (a, b) = (up[u + g].0, dn[g + U - 1 - u].0);
                for w in 0..SW {
                    p[w] = if u == 0 {
                        h[0] * (a[w] + b[w])
                    } else {
                        M::mac(p[w], h[u], a[w] + b[w])
                    };
                }
            }
            for w in 0..SW {
                acc[w] += p[w];
            }
        }
    }
    let t = h.len();
    let tu = t / ST_U * ST_U;
    let mut j = 0;
    while j < tu {
        let d = lo + 1 - j - ST_U;
        group::<M, ST_U, ST_N>(
            acc,
            h[j..j + ST_U].try_into().unwrap(),
            y[hi + j..hi + j + ST_N].try_into().unwrap(),
            y[d..d + ST_N].try_into().unwrap(),
        );
        j += ST_U;
    }
    while j < t {
        group::<M, 1, SV>(
            acc,
            h[j..j + 1].try_into().unwrap(),
            y[hi + j..hi + j + SV].try_into().unwrap(),
            y[lo - j..lo - j + SV].try_into().unwrap(),
        );
        j += 1;
    }
}

/// Stores a group (outputs `v0 .. v0 + SV` of every lane, lanes `s` outputs apart) rescaled
/// and rounded.
#[inline(always)]
fn st_store(acc: &[[f64; SW]; SV], scale: f64, out: &mut [f64], v0: usize, s: usize) {
    for w in 0..SW {
        let o = &mut out[w * s + v0..][..SV];
        for (g, o) in o.iter_mut().enumerate() {
            *o = round_even(acc[g][w] * scale);
        }
    }
}

/// One input of a staggered stage: output `i` takes `c0 x[i + c]` (if `c0` is not 0) and
/// `h[j] (x[i + hi + j] + x[i + lo - j])`; the window of output `i` is `x[i .. i + span]`.
struct StIn<'a> {
    x: &'a [f64],
    c0: f64,
    c: usize,
    h: &'a [f64],
    hi: usize,
    lo: usize,
    span: usize,
}

/// A staggered stage over up to two inputs; returns the number of outputs done (all but
/// fewer than `SW SV` at the end; the caller finishes them).
#[inline(always)]
fn st_stage<M: Mac, const K: usize>(ins: [StIn<'_>; K], scale: f64, out: &mut [f64]) -> usize {
    let mut ys = [[Lanes([0.0; SW]); ST_Y]; K];
    let mut i = 0;
    loop {
        let s = st_block(out.len(), i);
        if s == 0 {
            return i;
        }
        for (y, inp) in ys.iter_mut().zip(&ins) {
            stagger(inp.x, i, s, &mut y[..s + inp.span]);
        }
        for v0 in (0..s).step_by(SV) {
            let mut acc = [[0.0; SW]; SV];
            let (y, inp) = (&ys[0], &ins[0]);
            if inp.c0 != 0.0 {
                for (g, a) in acc.iter_mut().enumerate() {
                    let x = y[v0 + g + inp.c].0;
                    for w in 0..SW {
                        a[w] = inp.c0 * x[w];
                    }
                }
            }
            for (y, inp) in ys.iter().zip(&ins) {
                st_pairs::<M>(&mut acc, inp.h, y, v0 + inp.hi, v0 + inp.lo);
            }
            st_store(&acc, scale, &mut out[i..], v0, s);
        }
        i += SW * s;
    }
}

/// The shortest filter (largest tap offset) the staggered kernels take: for the half-band
/// (offset 15, 8 odd taps) copying the input costs more than the loads it saves.
const ST_MIN: usize = 32;

/// Whether the staggered kernels take `s`: proven exact, and long enough to gain but every
/// tap offset within reach.
#[inline(always)]
pub(super) fn st_ok(s: &Sym) -> bool {
    s.fused && (ST_MIN..=ST_MAX).contains(&s.half_len())
}

/// [`fir1`] with staggered lanes, for a stage with [`st_ok`]; the result is the same.
#[inline(always)]
pub(super) fn fir1_st<M: Mac>(s: &Sym, x: &[f64], out: &mut [f64]) {
    let m = s.half_len();
    let inp = StIn {
        x,
        c0: s.full[0],
        c: m,
        h: &s.full[1..],
        hi: m + 1,
        lo: m - 1,
        span: 2 * m,
    };
    let d = st_stage::<M, 1>([inp], s.scale, out);
    fir1::<1, M>(s, &x[d..], &mut out[d..]);
}

/// [`dec2`] with staggered lanes, for a stage with [`st_ok`]; the result is the same.
#[inline(always)]
pub(super) fn dec2_st<M: Mac>(s: &Sym, xe: &[f64], xo: &[f64], out: &mut [f64]) {
    let (e, o) = (s.even.len() - 1, s.odd.len());
    let even = StIn {
        x: xe,
        c0: s.even[0],
        c: e,
        h: &s.even[1..],
        hi: e + 1,
        lo: e.saturating_sub(1),
        span: 2 * e,
    };
    let odd = StIn {
        x: xo,
        c0: 0.0,
        c: 0,
        h: &s.odd,
        hi: o,
        lo: o - 1,
        span: 2 * o,
    };
    let d = st_stage::<M, 2>([even, odd], s.scale, out);
    dec2::<1, M>(s, &xe[d..], &xo[d..], &mut out[d..]);
}

/// [`interp2_odd`] with staggered lanes, for a stage with [`st_ok`]; the result is the
/// same.
#[inline(always)]
pub(super) fn interp2_odd_st<M: Mac>(s: &Sym, w: &[f64], out: &mut [f64]) {
    let o = s.odd.len();
    let inp = StIn {
        x: w,
        c0: 0.0,
        c: 0,
        h: &s.odd,
        hi: o + 1,
        lo: o,
        span: 2 * o,
    };
    let d = st_stage::<M, 1>([inp], s.scale * 2.0, out);
    interp2_odd::<1, M>(s, &w[d..], &mut out[d..]);
}

#[cfg(test)]
mod tests {
    use super::super::kernel::Plain;
    use super::super::{exact, tables, Cutoff, Op, PcmRate, Plan};
    use super::*;

    /// Every proven stage of every plan that the staggered kernels take gives the ordered
    /// kernels' outputs, for inputs at the stage's input bound (signs random, all equal, or
    /// alternating) and any output count, the tail included.
    #[test]
    fn staggered_matches_ordered() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut taken = 0;
        for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
            for cutoff in [Cutoff::Low, Cutoff::Standard, Cutoff::Wide] {
                let Ok(plan) = Plan::new(rate, cutoff) else {
                    continue;
                };
                let s1 = if cutoff == Cutoff::Wide {
                    &tables::S1_WIDE[..]
                } else {
                    &tables::S1[..]
                };
                let mut bound = exact::s1_bound(s1);
                for op in &plan.ops {
                    let s = match op {
                        Op::Fir1(s) | Op::Dec2(s) | Op::Interp2(s) => s,
                    };
                    let next = exact::stage(op, bound).unwrap().1;
                    if !st_ok(s) {
                        bound = next;
                        continue;
                    }
                    taken += 1;
                    let b = bound as f64;
                    for pattern in 0..3 {
                        let mut input = |n: usize| -> Vec<f64> {
                            (0..n)
                                .map(|i| match pattern {
                                    0 => b,
                                    1 => [b, -b][i % 2],
                                    _ => (rnd() % (2 * bound as u64 + 1)) as f64 - b,
                                })
                                .collect()
                        };
                        for n in (0..70).chain([1029, 1040, 4100]) {
                            let mut want = vec![0.0; n];
                            let mut got = vec![0.0; n];
                            let x = input(n + 2 * s.half_len() + 2);
                            match op {
                                Op::Fir1(s) => {
                                    fir1::<1, Plain>(s, &x, &mut want);
                                    fir1_st::<Plain>(s, &x, &mut got);
                                }
                                Op::Dec2(s) => {
                                    let xo = input(x.len());
                                    dec2::<1, Plain>(s, &x, &xo, &mut want);
                                    dec2_st::<Plain>(s, &x, &xo, &mut got);
                                }
                                Op::Interp2(s) => {
                                    interp2_odd::<1, Plain>(s, &x, &mut want);
                                    interp2_odd_st::<Plain>(s, &x, &mut got);
                                }
                            }
                            assert!(got == want, "{rate:?} {cutoff:?} {op:?} n {n}");
                        }
                    }
                    bound = next;
                }
            }
        }
        assert!(taken >= 6, "staggered stages: {taken}");
    }
}
