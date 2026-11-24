//! DSF files whose sample count is not a multiple of 8: the reader delivers the padding
//! bits of the last byte as the idle pattern's, so they convert as silence whatever the
//! file holds there.

use dsd2pcm::convert::{check_options, dsd64_to_pcm};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite};
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, Filter, PcmOptions, PcmRate, Threads};
use std::io::Cursor;

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

/// A stereo DSF of `ch0` and `ch1` (equal lengths) with `samples` samples.
fn dsf(ch0: &[u8], ch1: &[u8], samples: u64) -> Vec<u8> {
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), 2, 2_822_400, None).unwrap();
    w.write_bytes(&[ch0.to_vec(), ch1.to_vec()]).unwrap();
    w.finish(samples).unwrap();
    w.into_inner().into_inner()
}

fn convert(file: &[u8], opts: &PcmOptions) -> Vec<u8> {
    let r = DsfReader::new(Cursor::new(file.to_vec())).unwrap();
    let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, 2, opts.rate.hz()).unwrap();
    dsd64_to_pcm(Box::new(r), &mut w, opts).unwrap();
    w.finish().unwrap();
    let off = w.data_offset() as usize;
    w.into_inner().into_inner()[off..].to_vec()
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect()
}

/// Sets the low `p` bits (the padding, MSB-first) of the last byte to `pad`'s.
fn with_pad(bytes: &[u8], p: u32, pad: u8) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let mask = ((1u16 << p) - 1) as u8;
    let last = b.last_mut().unwrap();
    *last = (*last & !mask) | (pad & mask);
    b
}

#[test]
fn reader_masks_the_padding_bits() {
    for p in 1u32..=7 {
        for len in [1usize, 4095, 4096, 4097, 10_000] {
            let c0 = with_pad(&noise(len, p.into()), p, 0x00);
            let c1 = with_pad(&noise(len, 99 + u64::from(p)), p, 0xFF);
            let samples = 8 * len as u64 - u64::from(p);
            let mut r = DsfReader::new(Cursor::new(dsf(&c0, &c1, samples))).unwrap();
            let mut out = vec![Vec::new(), Vec::new()];
            // Small reads first, then the rest.
            for _ in 0..3 {
                r.read_bytes(1, &mut out).unwrap();
            }
            while r.read_bytes(1 << 20, &mut out).unwrap() > 0 {}
            assert_eq!(out[0], with_pad(&c0, p, 0x69), "p={p} len={len}");
            assert_eq!(out[1], with_pad(&c1, p, 0x69), "p={p} len={len}");
        }
    }
    // A multiple of 8 samples: the last byte is delivered as it is.
    let c0 = with_pad(&noise(100, 9), 7, 0x00);
    let mut r = DsfReader::new(Cursor::new(dsf(&c0, &c0, 800))).unwrap();
    let mut out = vec![Vec::new(), Vec::new()];
    while r.read_bytes(1 << 20, &mut out).unwrap() > 0 {}
    assert_eq!(out[0], c0);
}

#[test]
fn padding_converts_as_silence() {
    for filter in [Filter::Default, Filter::Precise, Filter::Pysacd] {
        for (rate, cutoff) in ALL {
            let mut opts = PcmOptions::new(rate);
            opts.cutoff = cutoff;
            opts.filter = filter;
            opts.threads = Threads::Single;
            if check_options(&opts).is_err() {
                continue;
            }
            for p in 1u32..=7 {
                let len = 3 * 4704 + 37 + p as usize;
                let samples = 8 * len as u64 - u64::from(p);
                // Idle content with zero padding: digital silence (the default and precise
                // filters; pysacd's quantiser adds dither).
                if filter != Filter::Pysacd {
                    let idle = with_pad(&vec![0x69; len], p, 0x00);
                    let out = convert(&dsf(&idle, &idle, samples), &opts);
                    assert!(
                        out.iter().all(|&b| b == 0),
                        "{filter:?} {rate:?} {cutoff:?} p={p}"
                    );
                }
                // Noise: the padding's value does not matter.
                let (n0, n1) = (noise(len, u64::from(p) * 77), noise(len, 5));
                let file = |pad| dsf(&with_pad(&n0, p, pad), &with_pad(&n1, p, pad), samples);
                let want = convert(&file(0x69), &opts);
                for pad in [0x00, 0xFF, 0x96] {
                    let got = convert(&file(pad), &opts);
                    assert!(
                        got == want,
                        "{filter:?} {rate:?} {cutoff:?} p={p} pad={pad:#x}"
                    );
                }
            }
        }
    }
}
