//! Speed of the fast and precise (default) filters, in memory, on one DSF file.
//!
//! `cargo run --release --example bench_filters -- FILE.dsf [SECONDS]`: converts the first
//! SECONDS (default 30) of FILE with both filters at every rate and cutoff, on one thread
//! and on all cores, and prints the speed as a multiple of real time (best of three runs)
//! and precise's cost relative to fast (time ratio). Pin it with `taskset` to time
//! one kind of core; `DSD2PCM_SIMD` caps the SIMD level.

use dsd2pcm::convert::dsd64_to_pcm;
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite};
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, Filter, PcmOptions, PcmRate, Threads};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Instant;

fn best(f: impl Fn()) -> f64 {
    (0..3)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64()
        })
        .fold(f64::MAX, f64::min)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: bench_filters FILE.dsf [SECONDS]");
    let secs: usize = args.get(2).map_or(30, |s| s.parse().unwrap());
    let mut r =
        DsfReader::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let ch = usize::from(r.info().channels);
    let mut chans = vec![Vec::new(); ch];
    while r.read_bytes(1 << 20, &mut chans).unwrap() > 0 {}
    let n = chans[0].len().min(secs * 352_800);
    for c in &mut chans {
        c.truncate(n);
    }
    let audio = n as f64 / 352_800.0;
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch as u16, 2_822_400, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish(n as u64 * 8).unwrap();
    let dsf: Arc<[u8]> = w.into_inner().into_inner().into();
    let level = dsd2pcm::decimator::SimdLevel::available();
    println!(
        "{path}: {ch} ch, {audio:.1} s, levels {level:?}; x real time (best of 3), ratio = precise time / fast time"
    );
    println!("rate    cutoff    threads     fast  precise  ratio");
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
        for cutoff in [Cutoff::Low, Cutoff::Standard, Cutoff::Wide] {
            if cutoff == Cutoff::Wide && rate == PcmRate::Hz88200 {
                continue;
            }
            for threads in [false, true] {
                let run = |filter| {
                    let opts = PcmOptions::new(rate)
                        .with_cutoff(cutoff)
                        .with_gain_db(0.0)
                        .with_threads(if threads {
                            Threads::All
                        } else {
                            Threads::Single
                        })
                        .with_filter(filter);
                    let size = 68 + n * 8 / rate.decimation() as usize * ch * 3;
                    best(|| {
                        let r = DsfReader::new(Cursor::new(dsf.clone())).unwrap();
                        let mut wav = WavWriter::new(
                            Cursor::new(Vec::with_capacity(size)),
                            WavFormat::I24,
                            ch as u16,
                            rate.hz(),
                        )
                        .unwrap();
                        dsd64_to_pcm(Box::new(r), &mut wav, &opts).unwrap();
                        wav.finish().unwrap();
                    })
                };
                let (d, p) = (run(Filter::Fast), run(Filter::Precise));
                println!(
                    "{:<7} {:<9} {:<8} {:>7.1} {:>8.1} {:>6.2}",
                    rate.hz(),
                    format!("{cutoff:?}"),
                    if threads { "all" } else { "1" },
                    audio / d,
                    audio / p,
                    p / d
                );
            }
        }
    }
}
