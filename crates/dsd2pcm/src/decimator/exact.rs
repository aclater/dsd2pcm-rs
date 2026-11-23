// SPDX-License-Identifier: MIT OR Apache-2.0
//! The exactness proof of the stage sums, from the tables and the stage plan. The bounds
//! follow scarletbook-pcm 0.7.0 (`design::tests::f64_sums_are_exact`; Portions Copyright
//! (c) 2026 The scarletbook-rs authors, MIT OR Apache-2.0, see PROVENANCE.md).
//!
//! S1 outputs are sums of `+-tap` over the window, so `|x| <= sum |tap|`. A stage with input
//! bound `x` sums products `h (a + b)` with `|a + b| <= 2x`; every partial sum, in any order,
//! is at most `sum |h| x` (all taps, both sides). If that is below 2^53, every operand,
//! product and partial sum is an integer that `f64` holds exactly, so a fused multiply-add
//! (one rounding) and a product and addition (two) give the same sum, and so does any order
//! or grouping of the terms (`super::lanes` sums groups of taps into partial sums: each is
//! the sum of a subset of the terms, so within the same bound). The output, rounded
//! back to the grid by `2^-q`, is at most `(sum |h| x >> q) + 1`. Interp2 sums only the odd
//! taps (both sides) and passes its input through as the even outputs.
//!
//! The arithmetic here is in `u128`, so the proof itself cannot overflow.

use super::kernel::Sym;
use super::Op;

/// Integers below this are exact in `f64`.
pub(super) const LIMIT: u128 = 1 << 53;

/// `sum |h|` over a symmetric filter given from the centre out.
fn abs_full(half: &[f64]) -> u128 {
    half.iter()
        .enumerate()
        .map(|(i, &h)| int(h) * if i == 0 { 1 } else { 2 })
        .sum()
}

/// `|h|` of an integer tap.
fn int(h: f64) -> u128 {
    assert!(
        h.fract() == 0.0 && h.abs() < 2f64.powi(63),
        "taps are integers"
    );
    h.abs() as u128
}

/// `q` of a scale `2^-q`.
fn shift(scale: f64) -> u32 {
    let e = ((scale.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    assert!(
        scale == f64::from_bits(((1023 + e) as u64) << 52) && e < 0,
        "scale is 2^-q"
    );
    (-e) as u32
}

/// The bound on S1's outputs: `sum |tap|` over the whole window.
pub(super) fn s1_bound(s1_half: &[i32]) -> u128 {
    let half: Vec<f64> = s1_half.iter().map(|&h| f64::from(h)).collect();
    abs_full(&half)
}

/// The proof for one stage with input bound `x`: the largest partial sum and the output
/// bound, or `None` if a sum may not be exact.
pub(super) fn stage(op: &Op, x: u128) -> Option<(u128, u128)> {
    if 2 * x >= LIMIT {
        return None;
    }
    let (worst, out) = match op {
        Op::Fir1(s) | Op::Dec2(s) => {
            let worst = abs_full(&s.full) * x;
            (worst, (worst >> shift(s.scale)) + 1)
        }
        Op::Interp2(s) => {
            let odd: u128 = s.odd.iter().map(|&h| 2 * int(h)).sum();
            let worst = odd * x;
            // The gain of 2 is in the scale: 2^-(q - 1).
            (worst, ((worst >> (shift(s.scale) - 1)) + 1).max(x))
        }
    };
    (worst < LIMIT).then_some((worst, out))
}

/// Marks `fused` on each stage whose sums are exact for every input, given S1's taps; a
/// stage after one without a proof is not marked. Returns whether every stage is.
pub(super) fn prove(s1_half: &[i32], ops: &mut [Op]) -> bool {
    let mut x = Some(s1_bound(s1_half));
    for op in ops.iter_mut() {
        let proved = x.and_then(|x| stage(op, x));
        sym_mut(op).fused = proved.is_some();
        x = proved.map(|(_, out)| out);
    }
    x.is_some()
}

fn sym_mut(op: &mut Op) -> &mut Sym {
    match op {
        Op::Fir1(s) | Op::Dec2(s) | Op::Interp2(s) => s,
    }
}
