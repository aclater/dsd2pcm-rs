// SPDX-License-Identifier: MIT OR Apache-2.0
//! `Filter::Pysacd` gives the bytes of pysacd's reference (tests/pysacd-ref: pysacd's
//! dsp.c built with `-O2 -ffp-contract=off`, fed each area as one stream) at every SIMD
//! level, through every entry point and however the work is split.

#[path = "pysacd-ref/inputs.rs"]
mod inputs;

use dsd2pcm::convert::{check_options, dsd64_to_pcm};
use dsd2pcm::decimator::{with_simd_level, SimdLevel};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::frames::{Converter, Gain, PcmConfig, PcmOut, Run};
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, Filter, PcmOptions, PcmRate};
use inputs::{gen, reference, FRAME};
use std::io::Cursor;

const FRAMES: usize = 5;
const RATES: [PcmRate; 2] = [PcmRate::Hz88200, PcmRate::Hz176400];

fn sha(q: &[i32]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for v in q {
        h.update(v.to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

fn name(kind: &str, ch: usize, rate: PcmRate, gain: i32, tc0: u64) -> String {
    format!("i32_{kind}_{ch}ch_{}_{gain}dB_tc{tc0}", rate.hz())
}

fn unpack(b: &[u8]) -> Vec<i32> {
    b.chunks_exact(3)
        .map(|s| i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8)
        .collect()
}

fn conv(ch: usize, rate: PcmRate, gain: f64) -> Converter {
    let cfg = PcmConfig::new(rate, Cutoff::Standard, ch as u8)
        .with_gain(Gain::from_db(gain).unwrap())
        .with_filter(Filter::Pysacd);
    Converter::new(cfg).unwrap()
}

/// The whole area as one run, interleaved `i32`.
fn whole_run(area: &[u8], ch: usize, rate: PcmRate, gain: f64, tc0: u64) -> (Vec<i32>, u64) {
    let mut c = conv(ch, rate, gain);
    let mut out = vec![0i32; area.len() / FRAME / ch * c.samples_per_frame() * ch];
    let st = c
        .convert(Run::new(area).at_frame(tc0), PcmOut::I32(&mut out), None)
        .unwrap();
    (out, st.clipped_total())
}

/// Runs of `split` frames each (summing to the area's frames), one buffer per frame, with
/// their neighbours, packed 24-bit; with the per-frame clip counts.
fn split_runs(
    area: &[u8],
    ch: usize,
    rate: PcmRate,
    gain: f64,
    tc0: u64,
    split: &[usize],
) -> (Vec<i32>, Vec<u64>) {
    let mut c = conv(ch, rate, gain);
    let fb = FRAME * ch;
    let frames: Vec<&[u8]> = area.chunks_exact(fb).collect();
    let spf = c.samples_per_frame();
    let mut out = vec![0u8; frames.len() * spf * ch * 3];
    let mut fc = vec![0u64; frames.len()];
    let mut f0 = 0;
    for &n in split {
        let run = Run::new(&frames[f0..f0 + n])
            .with_before(f0.checked_sub(1).map(|i| frames[i]))
            .with_after(frames.get(f0 + n).copied())
            .at_frame(tc0 + f0 as u64);
        c.convert(
            run,
            PcmOut::S24le(&mut out[f0 * spf * ch * 3..][..n * spf * ch * 3]),
            Some(&mut fc[f0..f0 + n]),
        )
        .unwrap();
        f0 += n;
    }
    assert_eq!(f0, frames.len());
    (unpack(&out), fc)
}

/// The whole-file conversion (frame 0 at the file's start).
fn file(area: &[u8], ch: usize, rate: PcmRate, gain: f64, threads: bool) -> (Vec<i32>, u64) {
    let n = area.len() / ch;
    let chans: Vec<Vec<u8>> = (0..ch)
        .map(|c| (0..n).map(|i| area[i * ch + c]).collect())
        .collect();
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch as u16, 2_822_400, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish(n as u64 * 8).unwrap();
    let r = DsfReader::new(Cursor::new(w.into_inner().into_inner())).unwrap();
    let mut wav = WavWriter::new(
        Cursor::new(Vec::new()),
        WavFormat::I24,
        ch as u16,
        rate.hz(),
    )
    .unwrap();
    let mut opts = PcmOptions::new(rate);
    opts.gain_db = gain;
    opts.threads = threads;
    opts.filter = Filter::Pysacd;
    let rep = dsd64_to_pcm(Box::new(r), &mut wav, &opts).unwrap();
    wav.finish().unwrap();
    (
        unpack(&wav.into_inner().into_inner()[44..]),
        rep.clipped.iter().sum(),
    )
}

/// Every input, channel count, rate and gain, as one run, at every SIMD level of this CPU
/// (each forced in turn).
#[test]
fn reference_at_every_simd_level() {
    for level in SimdLevel::available() {
        with_simd_level(level, || {
            for kind in ["silence", "ff", "music", "random"] {
                for ch in [2usize, 6] {
                    let area = gen(kind, ch, FRAMES);
                    for rate in RATES {
                        for gain in [0, -3, 6] {
                            let (q, _) = whole_run(&area, ch, rate, f64::from(gain), 0);
                            assert_eq!(
                                q.len(),
                                FRAMES * FRAME * ch / (rate.decimation() as usize / 8)
                            );
                            let n = name(kind, ch, rate, gain, 0);
                            assert_eq!(sha(&q), reference(&n), "{n} at {level:?}");
                        }
                    }
                }
            }
        })
        .unwrap();
    }
}

/// The file conversion (threaded or not), frame-by-frame runs and other splits give the
/// bytes and clip counts of the whole run.
#[test]
fn split_invariance() {
    for level in SimdLevel::available() {
        with_simd_level(level, || {
            for kind in ["ff", "music", "random"] {
                for ch in [2usize, 6] {
                    let area = gen(kind, ch, FRAMES);
                    for rate in RATES {
                        let gain = 6;
                        let want = reference(&name(kind, ch, rate, gain, 0));
                        let g = f64::from(gain);
                        let (whole, clipped) = whole_run(&area, ch, rate, g, 0);
                        assert_eq!(sha(&whole), want);
                        for threads in [false, true] {
                            let (q, k) = file(&area, ch, rate, g, threads);
                            assert_eq!(sha(&q), want, "file {kind} {ch} {rate:?} {threads}");
                            assert_eq!(k, clipped);
                        }
                        let mut fcs = Vec::new();
                        for split in [&[1usize, 1, 1, 1, 1][..], &[2, 3], &[4, 1], &[5]] {
                            let (q, fc) = split_runs(&area, ch, rate, g, 0, split);
                            assert_eq!(sha(&q), want, "{kind} {ch} {rate:?} {split:?}");
                            assert_eq!(fc.iter().sum::<u64>(), clipped);
                            fcs.push(fc);
                        }
                        assert!(fcs.windows(2).all(|w| w[0] == w[1]));
                        if kind == "ff" {
                            assert!(clipped > 0, "+6 dB of all-ones clips");
                        }
                    }
                }
            }
        })
        .unwrap();
    }
}

/// The dither is seeded from the frame's index in the area (wrapping at 2^32).
#[test]
fn seeds_follow_the_frame_index() {
    let cases = [
        ("music", 2usize, PcmRate::Hz88200, 0, 1000u64),
        ("music", 6, PcmRate::Hz176400, -3, 4_294_967_293),
    ];
    for (kind, ch, rate, gain, tc0) in cases {
        let area = gen(kind, ch, FRAMES);
        let want = reference(&name(kind, ch, rate, gain, tc0));
        let g = f64::from(gain);
        assert_eq!(sha(&whole_run(&area, ch, rate, g, tc0).0), want);
        assert_eq!(
            sha(&split_runs(&area, ch, rate, g, tc0, &[1, 3, 1]).0),
            want
        );
        assert_ne!(sha(&whole_run(&area, ch, rate, g, 0).0), want);
    }
}

#[test]
fn rejects_other_rates_and_cutoffs() {
    for (rate, cutoff) in [
        (PcmRate::Hz352800, Cutoff::Standard),
        (PcmRate::Hz176400, Cutoff::Wide),
        (PcmRate::Hz88200, Cutoff::Low),
    ] {
        let cfg = PcmConfig::new(rate, cutoff, 2).with_filter(Filter::Pysacd);
        assert!(Converter::new(cfg).is_err());
        let mut opts = PcmOptions::new(rate);
        opts.cutoff = cutoff;
        opts.filter = Filter::Pysacd;
        assert!(check_options(&opts).is_err());
    }
}

/// The context a frame needs from its neighbours is less than a frame, and the output does
/// not depend on bytes beyond it.
#[test]
fn context_bytes() {
    for rate in RATES {
        let c = conv(2, rate, 0.0);
        let ctx = c.context_bytes();
        assert!(ctx > 0 && ctx < FRAME, "{ctx}");
        let area = gen("random", 2, 3);
        let mut c = conv(2, rate, 0.0);
        let spf = c.samples_per_frame();
        let mid = &area[2 * FRAME..4 * FRAME];
        let mut a = vec![0i32; spf * 2];
        let mut b = vec![0i32; spf * 2];
        let before = &area[..2 * FRAME];
        let after = &area[4 * FRAME..];
        // Neighbours that differ beyond the context.
        let mut before2 = before.to_vec();
        let mut after2 = after.to_vec();
        for x in &mut before2[..2 * (FRAME - ctx)] {
            *x ^= 0x5a;
        }
        for x in &mut after2[2 * ctx..] {
            *x ^= 0x5a;
        }
        let run = |b4: &[u8], af: &[u8], out: &mut [i32], c: &mut Converter| {
            let r = Run::new(mid)
                .with_before(Some(b4))
                .with_after(Some(af))
                .at_frame(1);
            c.convert(r, PcmOut::I32(out), None).unwrap();
        };
        run(before, after, &mut a, &mut c);
        run(&before2, &after2, &mut b, &mut c);
        assert_eq!(a, b);
    }
}
