// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reference conversion for tools/quality/decode.py: converts a raw DSD64 file (channels
//! interleaved byte by byte, earliest sample in the MSB, as in DSDIFF and Super Audio CD
//! frames; a whole number of 4704-byte frames per channel) with scarletbook-pcm 0.7.0 as
//! one run, and writes 24-bit little-endian PCM, interleaved.
//!
//! Usage: sbpcm-ref IN.raw OUT.s24 CHANNELS RATE CUTOFF   (CUTOFF: 30, 40 or wide)

use scarletbook_pcm::{pack_s24le, Converter, Cutoff, FrameRun, PcmConfig, PcmRate};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert_eq!(a.len(), 6, "usage: sbpcm-ref IN.raw OUT.s24 CHANNELS RATE CUTOFF");
    let dsd = std::fs::read(&a[1]).expect("read input");
    let ch: u8 = a[3].parse().expect("channels");
    let rate = PcmRate::from_hz(a[4].parse().expect("rate")).expect("rate");
    let cutoff = match a[5].as_str() {
        "30" => Cutoff::Low,
        "40" => Cutoff::Standard,
        "wide" => Cutoff::Wide,
        c => panic!("cutoff {c}"),
    };
    let frame = 4704 * usize::from(ch);
    assert_eq!(dsd.len() % frame, 0, "input must be whole frames");
    let mut conv = Converter::new(PcmConfig::new(rate, cutoff, ch)).expect("config");
    let n = dsd.len() / frame * conv.samples_per_frame() * usize::from(ch);
    let mut pcm = vec![0i32; n];
    let stats = conv.convert_run(FrameRun::new(&dsd), &mut pcm).expect("convert");
    let mut out = vec![0u8; n * 3];
    pack_s24le(&pcm, &mut out);
    std::fs::write(&a[2], out).expect("write output");
    eprintln!("clipped {}", stats.clipped_total());
}
