//! Multithreaded frame-run conversion as a file system with pooled converters drives it,
//! against scarletbook-pcm 0.7.0, in memory: the wall time of one whole area. dsd2pcm runs
//! `Filter::Fast` (scarletbook-pcm's filters, the same bytes), not the default filter.
//!
//! `cargo run --release --example threads -- FILE.dsf CRATE MODE [THREADS] [RATE] [CUTOFF] [REPEAT]`
//!
//! - CRATE: `ours` or `sb`; MODE: `cold` (converters created on demand inside the timed
//!   read, as a fresh mount does), `warm` (the pool already holds THREADS converters that
//!   have converted once) or `first` (one thread: the time to create a converter and convert
//!   one run, for the first converter of the process and for the next one, and one run on a
//!   converter in use);
//! - THREADS (default 12) worker threads take runs of 4 frames (each frame in its own
//!   buffer, with the neighbours, 24-bit output into an exact-size buffer, per-frame clip
//!   counts) in order, 4 runs per 16 frames; each run takes a converter from a shared
//!   last-in first-out pool and puts it back, so a converter rarely sees adjacent runs;
//! - RATE (default 176400) and CUTOFF (`low`, `standard`, `wide`; default `standard`);
//! - REPEAT (default 1): the area is the file's frames that many times over.
//!
//! Prints the wall time of the conversion and a hash of the output, which must be the same
//! for both crates. Pin the process with `taskset` to choose the CPUs.

use dsd2pcm::dsf::DsfReader;
use dsd2pcm::frames::{self, Converter, PcmConfig, PcmOut, Run};
use dsd2pcm::io::DsdRead;
use dsd2pcm::{Cutoff, Filter, PcmRate};
use scarletbook_pcm as sb;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const FRAME: usize = frames::FRAME_BYTES_PER_CHANNEL;

/// One converter of either crate.
enum Conv {
    Ours(Box<Converter>),
    Sb(Box<sb::Converter>),
}

impl Conv {
    fn new(ours: bool, rate: PcmRate, cutoff: Cutoff, ch: usize) -> Conv {
        if ours {
            return Conv::Ours(Box::new(
                Converter::new(PcmConfig::new(rate, cutoff, ch as u8).with_filter(Filter::Fast))
                    .unwrap(),
            ));
        }
        let sc = match cutoff {
            Cutoff::Standard => sb::Cutoff::Standard,
            Cutoff::Low => sb::Cutoff::Low,
            Cutoff::Wide => sb::Cutoff::Wide,
            c => unreachable!("cutoff {c:?}"),
        };
        let sr = sb::PcmRate::from_hz(rate.hz()).unwrap();
        Conv::Sb(Box::new(
            sb::Converter::new(sb::PcmConfig::new(sr, sc, ch as u8)).unwrap(),
        ))
    }

    /// Converts `list` with its neighbours into a new exact-size buffer.
    fn run(
        &mut self,
        list: &[&[u8]],
        before: Option<&[u8]>,
        after: Option<&[u8]>,
        fb: usize,
    ) -> Vec<u8> {
        let mut out = vec![0u8; list.len() * fb];
        let mut fc = vec![0u64; list.len()];
        match self {
            Conv::Ours(c) => {
                let run = Run::new(list).with_before(before).with_after(after);
                c.convert(run, PcmOut::S24le(&mut out), Some(&mut fc))
                    .unwrap();
            }
            Conv::Sb(c) => {
                let run = sb::Run::new(list).with_before(before).with_after(after);
                c.convert(run, sb::PcmOut::S24le(&mut out), Some(&mut fc))
                    .unwrap();
            }
        }
        out
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: threads FILE.dsf ours|sb cold|warm [THREADS] [RATE] [CUTOFF]";
    let path = args.get(1).expect(usage);
    let ours = match args.get(2).map(String::as_str) {
        Some("ours") => true,
        Some("sb") => false,
        _ => panic!("{usage}"),
    };
    let mode = args.get(3).map(String::as_str);
    let warm = match mode {
        Some("warm") => true,
        Some("cold" | "first") => false,
        _ => panic!("{usage}"),
    };
    let threads: usize = args.get(4).map_or(12, |s| s.parse().unwrap());
    let rate = PcmRate::from_hz(args.get(5).map_or(176_400, |s| s.parse().unwrap())).unwrap();
    let cutoff = match args.get(6).map(String::as_str) {
        Some("low") => Cutoff::Low,
        Some("wide") => Cutoff::Wide,
        _ => Cutoff::Standard,
    };
    let repeat: usize = args.get(7).map_or(1, |s| s.parse().unwrap());
    let mut r =
        DsfReader::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let ch = usize::from(r.info().channels);
    let mut chans = vec![Vec::new(); ch];
    while r.read_bytes(1 << 20, &mut chans).unwrap() > 0 {}
    let frames = chans[0].len() / FRAME;
    // Each frame in its own shared buffer, channels interleaved byte by byte.
    let list: Vec<Arc<[u8]>> = (0..frames)
        .map(|f| {
            let mut v = vec![0u8; FRAME * ch];
            for (c, d) in chans.iter().enumerate() {
                for (i, &b) in d[f * FRAME..(f + 1) * FRAME].iter().enumerate() {
                    v[i * ch + c] = b;
                }
            }
            v.into()
        })
        .collect();
    drop(chans);
    let list: Vec<Arc<[u8]>> = (0..repeat).flat_map(|_| list.iter().cloned()).collect();
    let frames = list.len();
    let spf = Converter::new(PcmConfig::new(rate, cutoff, ch as u8).with_filter(Filter::Fast))
        .unwrap()
        .samples_per_frame();
    let fb = spf * ch * 3;
    let runs = frames.div_ceil(4);
    if mode == Some("first") {
        let l: Vec<&[u8]> = list[8..12].iter().map(|f| &f[..]).collect();
        let (b, a) = (Some(&list[7][..]), Some(&list[12][..]));
        let mut ms = Vec::new();
        let mut c = None;
        for _ in 0..2 {
            let t = Instant::now();
            let mut n = Conv::new(ours, rate, cutoff, ch);
            n.run(&l, b, a, fb);
            ms.push(t.elapsed().as_secs_f64() * 1e3);
            c = Some(n);
        }
        let mut c = c.unwrap();
        let t = Instant::now();
        let l2: Vec<&[u8]> = list[20..24].iter().map(|f| &f[..]).collect();
        c.run(&l2, Some(&list[19][..]), Some(&list[24][..]), fb);
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        println!(
            "{} first: first converter {:.3} ms, next new converter {:.3} ms, in use {:.3} ms",
            if ours { "ours" } else { "sb" },
            ms[0],
            ms[1],
            ms[2]
        );
        return;
    }
    let pool: Mutex<Vec<Conv>> = Mutex::new(Vec::new());
    if warm {
        // THREADS converters that have each converted once.
        let mut v: Vec<Conv> = (0..threads)
            .map(|_| Conv::new(ours, rate, cutoff, ch))
            .collect();
        for c in &mut v {
            let l: Vec<&[u8]> = list[..4.min(frames)].iter().map(|f| &f[..]).collect();
            c.run(&l, None, list.get(4).map(|f| &f[..]), fb);
        }
        *pool.lock().unwrap() = v;
    }
    let outs: Vec<Mutex<Vec<u8>>> = (0..runs).map(|_| Mutex::new(Vec::new())).collect();
    let next = AtomicUsize::new(0);
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= runs {
                    break;
                }
                let (a, b) = (4 * j, (4 * j + 4).min(frames));
                let mut conv = pool
                    .lock()
                    .unwrap()
                    .pop()
                    .unwrap_or_else(|| Conv::new(ours, rate, cutoff, ch));
                let l: Vec<&[u8]> = list[a..b].iter().map(|f| &f[..]).collect();
                let before = a.checked_sub(1).map(|p| &list[p][..]);
                let after = list.get(b).map(|f| &f[..]);
                let out = conv.run(&l, before, after, fb);
                pool.lock().unwrap().push(conv);
                *outs[j].lock().unwrap() = out;
            });
        }
    });
    let wall = t.elapsed().as_secs_f64();
    // FNV-1a over the whole output.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for o in &outs {
        for &b in o.lock().unwrap().iter() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    println!(
        "{} {} {} threads {} {:?}: {:.4} s, {:.1} x real time, hash {h:016x}",
        if ours { "ours" } else { "sb" },
        if warm { "warm" } else { "cold" },
        threads,
        rate.hz(),
        cutoff,
        wall,
        frames as f64 / 75.0 / wall
    );
}
