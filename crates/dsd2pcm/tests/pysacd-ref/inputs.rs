// SPDX-License-Identifier: MIT OR Apache-2.0
//! The test inputs of `pysacd_ref gen` (pysacd_ref.c), byte for byte, and the reference
//! hashes in sha256.txt.
#![allow(dead_code)]

/// Bytes per channel per frame.
pub const FRAME: usize = 4704;

/// `frames` frames of `ch` channels, interleaved byte by byte, of kind `silence`, `ff`,
/// `music` (two integer sines through a 1-bit modulator) or `random`.
pub fn gen(kind: &str, ch: usize, frames: usize) -> Vec<u8> {
    let n = frames * FRAME;
    let mut buf = vec![0u8; n * ch];
    for c in 0..ch {
        let (mut x1, mut y1, mut x2, mut y2) = (
            1i64 << 22,
            (c as i64) << 20,
            1i64 << 22,
            -((c as i64) << 20),
        );
        let (k1, k2) = (2_390_000i64, 16_730_000i64);
        let (mut integ, mut prev) = (0i64, 1i64 << 24);
        let mut r = 0x9E37_79B9_7F4A_7C15u64 ^ (c as u64 + 1);
        for i in 0..n {
            let byte = match kind {
                "silence" => 0x69,
                "ff" => 0xff,
                "random" => {
                    r ^= r << 13;
                    r ^= r >> 7;
                    r ^= r << 17;
                    (r >> 32) as u8
                }
                "music" => {
                    let mut byte = 0u8;
                    for _ in 0..8 {
                        x1 -= (k1 * y1) >> 30;
                        y1 += (k1 * x1) >> 30;
                        x2 -= (k2 * y2) >> 30;
                        y2 += (k2 * x2) >> 30;
                        integ += x1 + x2 - prev;
                        let bit = u8::from(integ >= 0);
                        prev = if bit == 1 { 1 << 24 } else { -(1 << 24) };
                        byte = byte << 1 | bit;
                    }
                    byte
                }
                _ => panic!("unknown kind {kind}"),
            };
            buf[i * ch + c] = byte;
        }
    }
    buf
}

/// The reference hash named `name` in sha256.txt.
pub fn reference(name: &str) -> &'static str {
    include_str!("sha256.txt")
        .lines()
        .find_map(|l| {
            let (h, n) = l.split_once("  ")?;
            (n == name).then_some(h)
        })
        .unwrap_or_else(|| panic!("no reference {name}"))
}
