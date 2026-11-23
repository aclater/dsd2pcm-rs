//! Frame-run conversion as a FUSE-style WAV reader drives it, against scarletbook-pcm 0.7.0,
//! single-threaded, in memory.
//!
//! `cargo run --release --example sacdfs_path -- FILE.dsf [SECONDS] [all]`: reads the first
//! SECONDS (default 30) of FILE as Super Audio CD frames (channels interleaved byte by byte,
//! 4704 bytes per channel each) and converts that area with both crates:
//!
//! - `run 1`: frame by frame, each frame in its own buffer with its neighbours;
//! - `run 4`: runs of four frames, each in its own buffer, with the neighbours;
//! - `area`: the whole area as one contiguous run without neighbours.
//!
//! Every mode must give the same bytes in both crates and in every mode. Prints the speed as
//! a multiple of real time (best of three). 176.4 kHz Standard only, or every rate and
//! cutoff with `all`. With `BENCH_OURS_FIRST=1` in the environment, dsd2pcm is timed
//! before scarletbook-pcm (the order alternates between rounds in
//! `.github/workflows/bench.yml`).

use dsd2pcm::dsf::DsfReader;
use dsd2pcm::frames::{self, Converter, PcmConfig, PcmOut, Run};
use dsd2pcm::io::DsdRead;
use dsd2pcm::{Cutoff, PcmRate};
use scarletbook_pcm as sb;
use std::time::Instant;

const FRAME: usize = frames::FRAME_BYTES_PER_CHANNEL;

fn best(mut f: impl FnMut()) -> f64 {
    let mut t = f64::MAX;
    for _ in 0..3 {
        let s = Instant::now();
        f();
        t = t.min(s.elapsed().as_secs_f64());
    }
    t
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: sacdfs_path FILE.dsf [SECONDS] [all]");
    let secs: usize = args.get(2).map_or(30, |s| s.parse().unwrap());
    let all = args.get(3).is_some_and(|s| s == "all");
    let mut r =
        DsfReader::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let ch = usize::from(r.info().channels);
    let mut chans = vec![Vec::new(); ch];
    while r.read_bytes(1 << 20, &mut chans).unwrap() > 0 {}
    let frames = (chans[0].len() / FRAME).min(secs * 75);
    // Super Audio CD layout: frame by frame, channels interleaved byte by byte.
    let mut area = vec![0u8; frames * FRAME * ch];
    for (c, d) in chans.iter().enumerate() {
        for (i, &b) in d[..frames * FRAME].iter().enumerate() {
            area[i * ch + c] = b;
        }
    }
    drop(chans);
    let list: Vec<&[u8]> = area.chunks_exact(FRAME * ch).collect();
    let audio = frames as f64 / 75.0;
    let ours_first = std::env::var_os("BENCH_OURS_FIRST").is_some_and(|v| v == "1");
    println!("{path}: {ch} ch, {frames} frames ({audio:.1} s); x real time, one thread, best of 3");
    println!("rate    cutoff    mode    scarletbook-pcm      ours   ours/sb");
    let configs: &[(PcmRate, Cutoff)] = if all {
        &[
            (PcmRate::Hz88200, Cutoff::Low),
            (PcmRate::Hz88200, Cutoff::Standard),
            (PcmRate::Hz176400, Cutoff::Low),
            (PcmRate::Hz176400, Cutoff::Standard),
            (PcmRate::Hz176400, Cutoff::Wide),
            (PcmRate::Hz352800, Cutoff::Low),
            (PcmRate::Hz352800, Cutoff::Standard),
            (PcmRate::Hz352800, Cutoff::Wide),
        ]
    } else {
        &[(PcmRate::Hz176400, Cutoff::Standard)]
    };
    for &(rate, cutoff) in configs {
        let mut ours = Converter::new(PcmConfig::new(rate, cutoff, ch as u8)).unwrap();
        let sr = sb::PcmRate::from_hz(rate.hz()).unwrap();
        let sc = match cutoff {
            Cutoff::Standard => sb::Cutoff::Standard,
            Cutoff::Low => sb::Cutoff::Low,
            Cutoff::Wide => sb::Cutoff::Wide,
        };
        let mut theirs = sb::Converter::new(sb::PcmConfig::new(sr, sc, ch as u8)).unwrap();
        let fb = ours.samples_per_frame() * ch * 3;
        let mut reference: Option<Vec<u8>> = None;
        for mode in [1usize, 4, 0] {
            let mut a = vec![0u8; frames * fb];
            let mut b = vec![0u8; frames * fb];
            let mut fa = vec![0u64; frames];
            let mut fbc = vec![0u64; frames];
            let step = if mode == 0 { frames } else { mode };
            let mut run_sb = || {
                let mut t = 0;
                while t < frames {
                    let u = (t + step).min(frames);
                    let run = if mode == 0 {
                        sb::Run::new(&area[..])
                    } else {
                        sb::Run::new(&list[t..u])
                            .with_before(t.checked_sub(1).map(|p| list[p]))
                            .with_after(list.get(u).copied())
                    };
                    theirs
                        .convert(
                            run,
                            sb::PcmOut::S24le(&mut b[t * fb..u * fb]),
                            Some(&mut fbc[t..u]),
                        )
                        .unwrap();
                    t = u;
                }
            };
            let mut run_ours = || {
                let mut t = 0;
                while t < frames {
                    let u = (t + step).min(frames);
                    let run = if mode == 0 {
                        Run::new(&area[..])
                    } else {
                        Run::new(&list[t..u])
                            .with_before(t.checked_sub(1).map(|p| list[p]))
                            .with_after(list.get(u).copied())
                    };
                    ours.convert(
                        run,
                        PcmOut::S24le(&mut a[t * fb..u * fb]),
                        Some(&mut fa[t..u]),
                    )
                    .unwrap();
                    t = u;
                }
            };
            let (t_sb, t_ours) = if ours_first {
                let o = best(&mut run_ours);
                (best(&mut run_sb), o)
            } else {
                let t = best(&mut run_sb);
                (t, best(&mut run_ours))
            };
            assert!(a == b && fa == fbc, "output differs: {rate:?} {cutoff:?}");
            match &reference {
                Some(r) => assert!(*r == a, "mode differs: {rate:?} {cutoff:?}"),
                None => reference = Some(a),
            }
            println!(
                "{:<7} {:<9} {:<7} {:>15.1} {:>9.1} {:>8.2}",
                rate.hz(),
                format!("{cutoff:?}"),
                if mode == 0 {
                    "area".to_string()
                } else {
                    format!("run {mode}")
                },
                audio / t_sb,
                audio / t_ours,
                t_sb / t_ours
            );
        }
    }
}
