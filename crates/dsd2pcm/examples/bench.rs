//! Speed of DSD64 to 24-bit PCM against scarletbook-pcm 0.7.0, in memory, on one DSF file.
//!
//! `cargo run --release --example bench -- FILE.dsf [SECONDS]`: converts the first
//! SECONDS (default 60) of FILE with every rate and cutoff, single-threaded and with one
//! thread per channel, checks that both crates give the same bytes, and prints the speed as
//! a multiple of real time (best of three runs). With `BENCH_OURS_FIRST=1` in the
//! environment, dsd2pcm is timed before scarletbook-pcm (the order alternates between rounds
//! in `.github/workflows/bench.yml`).

use dsd2pcm::convert::dsd64_to_pcm;
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite};
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, PcmOptions, PcmRate, Threads};
use scarletbook_pcm as sb;
use std::io::Cursor;
use std::time::Instant;

const FRAME: usize = 4704;

fn best<T>(f: impl Fn() -> T) -> (f64, T) {
    let mut t = f64::MAX;
    let mut out = None;
    for _ in 0..3 {
        let s = Instant::now();
        let r = f();
        t = t.min(s.elapsed().as_secs_f64());
        out = Some(r);
    }
    (t, out.unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: bench FILE.dsf [SECONDS]");
    let secs: usize = args.get(2).map_or(60, |s| s.parse().unwrap());
    let mut r =
        DsfReader::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let ch = usize::from(r.info().channels);
    let mut chans = vec![Vec::new(); ch];
    while r.read_bytes(1 << 20, &mut chans).unwrap() > 0 {}
    let frames = (chans[0].len() / FRAME).min(secs * 75);
    for c in &mut chans {
        c.truncate(frames * FRAME);
    }
    let audio = frames as f64 / 75.0;
    let ours_first = std::env::var_os("BENCH_OURS_FIRST").is_some_and(|v| v == "1");
    // our input as an in-memory DSF
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch as u16, 2_822_400, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish(chans[0].len() as u64 * 8).unwrap();
    let dsf: std::sync::Arc<[u8]> = w.into_inner().into_inner().into();
    // theirs: interleaved
    let n = chans[0].len();
    let mut inter = vec![0u8; n * ch];
    for (c, d) in chans.iter().enumerate() {
        for (i, &b) in d.iter().enumerate() {
            inter[i * ch + c] = b;
        }
    }
    println!("{path}: {ch} ch, {audio:.1} s; speed in x real time (best of 3)");
    println!("rate    cutoff    scarletbook-pcm  ours-1-thread  ours-threads");
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
        for cutoff in [Cutoff::Low, Cutoff::Standard, Cutoff::Wide] {
            if cutoff == Cutoff::Wide && rate == PcmRate::Hz88200 {
                continue;
            }
            let ours = |threads: bool| {
                let opts = PcmOptions::new(rate)
                    .with_cutoff(cutoff)
                    .with_gain_db(0.0)
                    .with_threads(if threads {
                        Threads::All
                    } else {
                        Threads::Single
                    })
                    .with_filter(dsd2pcm::Filter::Default);
                let r = DsfReader::new(Cursor::new(dsf.clone())).unwrap();
                // Sized up front, as scarletbook-pcm's output buffer is.
                let size = 68 + n * 8 / rate.decimation() as usize * ch * 3;
                let mut wav = WavWriter::new(
                    Cursor::new(Vec::with_capacity(size)),
                    WavFormat::I24,
                    ch as u16,
                    rate.hz(),
                )
                .unwrap();
                dsd64_to_pcm(Box::new(r), &mut wav, &opts).unwrap();
                wav.finish().unwrap();
                let off = wav.data_offset() as usize;
                let mut v = wav.into_inner().into_inner();
                v.drain(..off);
                v
            };
            let theirs = || {
                let sr = sb::PcmRate::from_hz(rate.hz()).unwrap();
                let sc = match cutoff {
                    Cutoff::Standard => sb::Cutoff::Standard,
                    Cutoff::Low => sb::Cutoff::Low,
                    Cutoff::Wide => sb::Cutoff::Wide,
                    c => unreachable!("cutoff {c:?}"),
                };
                let mut conv = sb::Converter::new(sb::PcmConfig::new(sr, sc, ch as u8)).unwrap();
                let mut out = vec![0u8; frames * conv.samples_per_frame() * ch * 3];
                conv.convert(sb::Run::new(&inter[..]), sb::PcmOut::S24le(&mut out), None)
                    .unwrap();
                out
            };
            let ((t0, b), (t1, a1), (t2, a2)) = if ours_first {
                let o1 = best(|| ours(false));
                let o2 = best(|| ours(true));
                (best(theirs), o1, o2)
            } else {
                let t = best(theirs);
                (t, best(|| ours(false)), best(|| ours(true)))
            };
            assert!(a1 == b && a2 == b, "output differs: {rate:?} {cutoff:?}");
            println!(
                "{:<7} {:<9} {:>15.1} {:>14.1} {:>13.1}",
                rate.hz(),
                format!("{cutoff:?}"),
                audio / t0,
                audio / t1,
                audio / t2
            );
        }
    }
}
