//! `frames::Converter` gives the bytes and clip counts of scarletbook-pcm 0.7.0's
//! `Converter::convert` for runs of every length, with and without neighbours, in both
//! buffer layouts and both sample layouts, at every rate, cutoff, gain, channel count and
//! SIMD level; and frame by frame equals the whole area equals `dsd64_to_pcm`.

use dsd2pcm::convert::dsd64_to_pcm;
use dsd2pcm::decimator::{with_simd_level, SimdLevel};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::frames::{self, Converter, FrameRun, Gain, PcmConfig, PcmOut, Run};
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, PcmOptions, PcmRate};
use scarletbook_pcm as sb;
use std::io::Cursor;

const FRAME: usize = frames::FRAME_BYTES_PER_CHANNEL;

const ALL: [(PcmRate, Cutoff); 8] = [
    (PcmRate::Hz88200, Cutoff::Low),
    (PcmRate::Hz88200, Cutoff::Standard),
    (PcmRate::Hz176400, Cutoff::Low),
    (PcmRate::Hz176400, Cutoff::Standard),
    (PcmRate::Hz176400, Cutoff::Wide),
    (PcmRate::Hz352800, Cutoff::Low),
    (PcmRate::Hz352800, Cutoff::Standard),
    (PcmRate::Hz352800, Cutoff::Wide),
];

/// An interleaved area of `frames` frames: random bytes, idle, the other idle pattern,
/// full-scale runs (which clip at positive gains).
fn area(ch: usize, frames: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    (0..frames * FRAME * ch)
        .map(|i| {
            let i = i / ch;
            match (i / 3000) % 6 {
                0 | 1 => rnd() as u8,
                2 => 0x69,
                3 => [0x55, 0x69, 0x96, 0xAA][(rnd() % 4) as usize],
                4 => {
                    if (i / 700) % 2 == 0 {
                        0xFF
                    } else {
                        0x00
                    }
                }
                _ => (rnd() as u8) | 0x11,
            }
        })
        .collect()
}

fn sb_conv(rate: PcmRate, cutoff: Cutoff, ch: usize, gain_db: f64) -> sb::Converter {
    let rate = sb::PcmRate::from_hz(rate.hz()).unwrap();
    let cutoff = match cutoff {
        Cutoff::Standard => sb::Cutoff::Standard,
        Cutoff::Low => sb::Cutoff::Low,
        Cutoff::Wide => sb::Cutoff::Wide,
    };
    let cfg =
        sb::PcmConfig::new(rate, cutoff, ch as u8).with_gain(sb::Gain::from_db(gain_db).unwrap());
    sb::Converter::new(cfg).unwrap()
}

fn our_conv(rate: PcmRate, cutoff: Cutoff, ch: usize, gain_db: f64) -> Converter {
    let cfg = PcmConfig::new(rate, cutoff, ch as u8).with_gain(Gain::from_db(gain_db).unwrap());
    Converter::new(cfg).unwrap()
}

/// Output bytes, per-frame clip counts and per-channel clip counts.
type Out = (Vec<u8>, Vec<u64>, Vec<u64>);

/// One run of `area`: frames `s .. s + len`, with the neighbours if `nb.0` / `nb.1`.
struct Case<'a> {
    area: &'a [u8],
    ch: usize,
    s: usize,
    len: usize,
    nb: (bool, bool),
    separate: bool,
}

impl Case<'_> {
    fn frame(&self, t: usize) -> &[u8] {
        &self.area[t * FRAME * self.ch..][..FRAME * self.ch]
    }
    fn before(&self) -> Option<&[u8]> {
        self.nb.0.then(|| self.frame(self.s - 1))
    }
    fn after(&self) -> Option<&[u8]> {
        self.nb.1.then(|| self.frame(self.s + self.len))
    }
    fn contiguous(&self) -> &[u8] {
        &self.area[self.s * FRAME * self.ch..][..self.len * FRAME * self.ch]
    }
    fn list(&self) -> Vec<&[u8]> {
        (self.s..self.s + self.len).map(|t| self.frame(t)).collect()
    }
    fn out_len(&self, spf: usize) -> usize {
        self.len * spf * self.ch
    }

    fn theirs(&self, c: &mut sb::Converter) -> Out {
        let mut out = vec![0u8; 3 * self.out_len(c.samples_per_frame())];
        let mut fc = vec![u64::MAX; self.len];
        let list = self.list();
        let run = if self.separate {
            sb::Run::new(&list[..])
        } else {
            sb::Run::new(self.contiguous())
        };
        let run = run.with_before(self.before()).with_after(self.after());
        let st = c
            .convert(run, sb::PcmOut::S24le(&mut out), Some(&mut fc))
            .unwrap();
        (out, fc, st.clipped[..self.ch].to_vec())
    }

    fn ours(&self, c: &mut Converter) -> Out {
        let mut out = vec![0xAAu8; 3 * self.out_len(c.samples_per_frame())];
        let mut fc = vec![u64::MAX; self.len];
        let list = self.list();
        let run = if self.separate {
            Run::new(&list[..])
        } else {
            Run::new(self.contiguous())
        };
        let run = run.with_before(self.before()).with_after(self.after());
        let st = c
            .convert(run, PcmOut::S24le(&mut out), Some(&mut fc))
            .unwrap();
        assert_eq!(st.clipped_total(), fc.iter().sum::<u64>());
        (out, fc, st.clipped[..self.ch].to_vec())
    }

    /// The same run with `I32` output, packed.
    fn ours_i32(&self, c: &mut Converter) -> Vec<u8> {
        let mut out = vec![0x5A5A_5A5Ai32; self.out_len(c.samples_per_frame())];
        let list = self.list();
        let run = if self.separate {
            Run::new(&list[..])
        } else {
            Run::new(self.contiguous())
        };
        let run = run.with_before(self.before()).with_after(self.after());
        c.convert(run, PcmOut::I32(&mut out), None).unwrap();
        let mut b = vec![0u8; 3 * out.len()];
        frames::pack_s24le(&out, &mut b);
        b
    }
}

/// Runs of 1, 2, 7 and 64 frames from 66-frame areas, all four neighbour cases, alternating
/// buffer layouts, at every rate and cutoff; gains 0, -3 and +7.5 dB (clips) in stereo and
/// 0 and +7.5 dB in mono and 6 channels; at every SIMD level of this CPU. Every byte, every
/// per-frame and per-channel clip count equals scarletbook-pcm's.
#[test]
fn runs_identical_to_scarletbook_pcm() {
    let levels = SimdLevel::available();
    let mut cases = 0usize;
    let mut clipped = 0u64;
    for ch in [1usize, 2, 6] {
        let area = area(ch, 66, 0xC0FF_EE00 ^ ch as u64);
        let gains: &[f64] = if ch == 2 {
            &[0.0, -3.0, 7.5]
        } else {
            &[0.0, 7.5]
        };
        for (rate, cutoff) in ALL {
            for &gain in gains {
                let mut theirs = sb_conv(rate, cutoff, ch, gain);
                let mut ours: Vec<Converter> = levels
                    .iter()
                    .map(|&l| with_simd_level(l, || our_conv(rate, cutoff, ch, gain)).unwrap())
                    .collect();
                for (k, len) in [1usize, 2, 7, 64].into_iter().enumerate() {
                    for (j, nb) in [(false, false), (true, false), (false, true), (true, true)]
                        .into_iter()
                        .enumerate()
                    {
                        let case = Case {
                            area: &area,
                            ch,
                            s: 1,
                            len,
                            nb,
                            separate: (j + k) % 2 == 1,
                        };
                        let want = case.theirs(&mut theirs);
                        clipped += want.1.iter().sum::<u64>();
                        for (level, c) in levels.iter().zip(&mut ours) {
                            let got = case.ours(c);
                            let what = format!(
                                "{ch} ch {rate:?} {cutoff:?} {gain} dB, {len} frames, \
                                 neighbours {nb:?}, separate {}, {level:?}",
                                case.separate
                            );
                            assert!(got.0 == want.0, "bytes: {what}");
                            assert_eq!(got.1, want.1, "frame clips: {what}");
                            assert_eq!(got.2, want.2, "channel clips: {what}");
                            assert!(case.ours_i32(c) == want.0, "I32 output: {what}");
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(clipped > 0, "the +7.5 dB cases clip");
    eprintln!("{cases} run conversions identical");
}

/// A short frame padded with the idle pattern (as a caller does with a partial last frame)
/// converts as scarletbook-pcm converts the same padded frame.
#[test]
fn padded_short_frame() {
    let ch = 2;
    let mut a = area(ch, 3, 77);
    // Frame 2 has only 1000 bytes per channel of audio; the rest is padding.
    let pad_from = (2 * FRAME + 1000) * ch;
    a[pad_from..].fill(0x69);
    for (rate, cutoff) in ALL {
        let case = Case {
            area: &a,
            ch,
            s: 2,
            len: 1,
            nb: (true, false),
            separate: true,
        };
        let want = case.theirs(&mut sb_conv(rate, cutoff, ch, 0.0));
        let got = case.ours(&mut our_conv(rate, cutoff, ch, 0.0));
        assert!(got == want, "{rate:?} {cutoff:?}");
    }
}

/// Our whole-file conversion of an area given as interleaved frames.
fn whole_file(area: &[u8], ch: usize, rate: PcmRate, cutoff: Cutoff, gain_db: f64) -> Vec<u8> {
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
    let opts = PcmOptions {
        rate,
        cutoff,
        gain_db,
        threads: false,
        filter: dsd2pcm::Filter::Default,
    };
    dsd64_to_pcm(Box::new(r), &mut wav, &opts).unwrap();
    wav.finish().unwrap();
    wav.into_inner().into_inner()[44..].to_vec()
}

/// Frame by frame with neighbours (`None` at the edges), runs of 4 with neighbours, and the
/// whole area as one run all give the same bytes and per-frame clip counts, and equal
/// `dsd64_to_pcm`.
#[test]
fn frame_by_frame_equals_whole_area() {
    for ch in [1usize, 2, 6] {
        let frames = 13;
        let a = area(ch, frames, 0xABCD ^ ch as u64);
        for (rate, cutoff) in ALL {
            let gain = 7.5;
            let mut c = our_conv(rate, cutoff, ch, gain);
            let fb = c.samples_per_frame() * ch * 3;
            let mut whole = vec![0u8; frames * fb];
            let mut whole_fc = vec![0u64; frames];
            c.convert(
                Run::new(&a[..]),
                PcmOut::S24le(&mut whole),
                Some(&mut whole_fc),
            )
            .unwrap();
            let frame = |t: usize| &a[t * FRAME * ch..][..FRAME * ch];
            for step in [1usize, 4] {
                let mut pieces = vec![0u8; frames * fb];
                let mut fc = vec![0u64; frames];
                let mut t = 0;
                while t < frames {
                    let u = (t + step).min(frames);
                    let list: Vec<&[u8]> = (t..u).map(frame).collect();
                    let run = Run::new(&list[..])
                        .with_before(t.checked_sub(1).map(frame))
                        .with_after((u < frames).then(|| frame(u)));
                    c.convert(
                        run,
                        PcmOut::S24le(&mut pieces[t * fb..u * fb]),
                        Some(&mut fc[t..u]),
                    )
                    .unwrap();
                    t = u;
                }
                assert!(
                    pieces == whole,
                    "{ch} ch {rate:?} {cutoff:?} runs of {step}"
                );
                assert_eq!(fc, whole_fc, "{ch} ch {rate:?} {cutoff:?} runs of {step}");
            }
            // FrameRun and convert_frame: the same samples.
            let mut one = vec![0i32; c.samples_per_frame() * ch];
            c.convert_frame(Some(frame(4)), frame(5), Some(frame(6)), &mut one)
                .unwrap();
            let mut b = vec![0u8; fb];
            frames::pack_s24le(&one, &mut b);
            assert!(b == whole[5 * fb..6 * fb]);
            let mut two = vec![0i32; 2 * c.samples_per_frame() * ch];
            c.convert_run(
                FrameRun::new(&a[7 * FRAME * ch..9 * FRAME * ch])
                    .with_before(Some(frame(6)))
                    .with_after(Some(frame(9))),
                &mut two,
            )
            .unwrap();
            let mut b = vec![0u8; 2 * fb];
            frames::pack_s24le(&two, &mut b);
            assert!(b == whole[7 * fb..9 * fb]);
            assert!(
                whole_file(&a, ch, rate, cutoff, gain) == whole,
                "{ch} ch {rate:?} {cutoff:?}: dsd64_to_pcm"
            );
        }
    }
}

/// The neighbour context is less than one frame and matches scarletbook-pcm's; only that
/// many bytes of each neighbour matter.
#[test]
fn context_bytes() {
    let ch = 2;
    let a = area(ch, 5, 5);
    for (rate, cutoff) in ALL {
        let mut c = our_conv(rate, cutoff, ch, 0.0);
        let ctx = c.context_bytes();
        assert!(ctx < FRAME, "{rate:?} {cutoff:?}: {ctx}");
        assert_eq!(ctx, sb_conv(rate, cutoff, ch, 0.0).context_bytes());
        let frame = |t: usize| a[t * FRAME * ch..][..FRAME * ch].to_vec();
        let conv = |c: &mut Converter, b: &[u8], n: &[u8]| {
            let mut out = vec![0i32; c.samples_per_frame() * ch];
            c.convert_frame(Some(b), &frame(2), Some(n), &mut out)
                .unwrap();
            out
        };
        let want = conv(&mut c, &frame(1), &frame(3));
        // Bytes of the neighbours beyond the context changed: the same output.
        let (mut b, mut n) = (frame(1), frame(3));
        b[..(FRAME - ctx) * ch].fill(0);
        n[ctx * ch..].fill(0xFF);
        assert!(conv(&mut c, &b, &n) == want, "{rate:?} {cutoff:?}");
    }
}

/// Bad lengths are errors, and nothing is written.
#[test]
fn errors_write_nothing() {
    let ch = 2;
    let a = area(ch, 3, 9);
    let mut c = our_conv(PcmRate::Hz176400, Cutoff::Standard, ch, 0.0);
    let fb = c.samples_per_frame() * ch * 3;
    let mut out = vec![7u8; 2 * fb];
    let f = FRAME * ch;
    let bad: [(Run<'_>, usize, usize); 5] = [
        (Run::new(&a[..2 * f - 1]), 2 * fb, 2),
        (
            Run::new(&a[..2 * f]).with_before(Some(&a[..f - 2])),
            2 * fb,
            2,
        ),
        (
            Run::new(&a[..2 * f]).with_after(Some(&a[..f + 1])),
            2 * fb,
            2,
        ),
        (Run::new(&a[..2 * f]), 2 * fb - 3, 2),
        (Run::new(&a[..2 * f]), 2 * fb, 3),
    ];
    for (i, (run, len, counts)) in bad.into_iter().enumerate() {
        let mut fc = vec![9u64; counts];
        assert!(
            c.convert(run, PcmOut::S24le(&mut out[..len]), Some(&mut fc))
                .is_err(),
            "case {i}"
        );
        assert!(out.iter().all(|&b| b == 7) && fc.iter().all(|&k| k == 9));
    }
    let list = [&a[..f], &a[..f - 1]];
    assert!(c
        .convert(Run::new(&list[..]), PcmOut::S24le(&mut out), None)
        .is_err());
    for chs in [0u8, 7] {
        assert!(Converter::new(PcmConfig::new(PcmRate::Hz176400, Cutoff::Standard, chs)).is_err());
    }
    assert!(Converter::new(PcmConfig::new(PcmRate::Hz88200, Cutoff::Wide, 2)).is_err());
    assert!(Gain::from_db(24.5).is_err() && Gain::from_db(f64::NAN).is_err());
    // An empty run converts to nothing.
    let st = c
        .convert(Run::new(&a[..0]), PcmOut::S24le(&mut []), Some(&mut []))
        .unwrap();
    assert_eq!(st.clipped_total(), 0);
}
