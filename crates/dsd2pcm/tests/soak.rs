//! Soak test for long-lived processes: thousands of conversions in one process, and the
//! memory levels off and every worker thread ends.
//!
//! Ignored by default. Run it with
//!
//! ```text
//! DSD2PCM_SOAK_ITERATIONS=1000 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture
//! ```
//!
//! `DSD2PCM_SOAK_ITERATIONS` (default 300) sets the iterations; with `DSD2PCM_SOAK_SECONDS`
//! the run stops after that many seconds instead (the iterations, if set, are a ceiling).
//! Each iteration converts one pseudo-random area with one setting of the cycle below (every rate, cutoff, filter and
//! channel count, invalid combinations included as error paths): with a reused and a fresh
//! `frames::Converter` (contiguous and per-frame buffers, a whole run and frame by frame),
//! as an in-memory DSF or DFF file through `dsd64_to_pcm` (threaded and on one thread, now
//! and then cancelled from the progress hook), reads the WAV back, reads a FLAC file, feeds
//! the readers truncated files, and every tenth iteration converts a file on disk with
//! `convert_file_with`. The outputs of the paths are compared with each other.
//!
//! It judges the memory of runs of 1000 iterations or more (`MIN_JUDGED`; a shorter run,
//! the default of 300 included, prints the figures without judging them). After a warm-up
//! of two rounds of the cycle (216 iterations) the run is cut in two windows, the plateau
//! and the late one, and:
//! - the live heap (bytes allocated and not freed, counted by a global allocator), less
//!   what the kept converters grew by, may rise at most 64 KiB from the plateau's highest
//!   value to the late window's: the leak measure;
//! - the kept converters, whose buffers grow while they meet calls that need more, may grow
//!   on at most 1 % of their calls in the late window, and by at most 4 MiB;
//!
//! and on Linux, from `/proc/self`:
//! - the resident set size's (`VmRSS`) median in the late window is within 25 % or 64 MiB
//!   plus 2 MiB per worker thread (whichever is larger) of its median in the plateau;
//! - the thread count is back at its value from before the run after every threaded
//!   conversion.
//!
//! `DSD2PCM_SOAK_HEAP_REPORT_ONLY` (the live heap and the kept converters) and
//! `DSD2PCM_SOAK_RSS_REPORT_ONLY` (the resident set, for sanitizer builds) print without
//! checking. `soak_catches_a_leak` puts leaks in on purpose and checks that each check
//! catches its own.

use dsd2pcm::convert::{dsd64_to_pcm, dsd64_to_pcm_with};
use dsd2pcm::dff::{DffReader, DffWriter};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::frames::{
    Converter, Gain, PcmConfig, PcmOut, Run, FRAME_BYTES_PER_CHANNEL, MAX_CHANNELS,
};
use dsd2pcm::io::{DsdRead, DsdWrite, PcmRead};
use dsd2pcm::wav::{WavFormat, WavReader, WavWriter};
use dsd2pcm::{convert_file_with, Cutoff, Error, Filter, PcmOptions, PcmRate};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// The system allocator, counting the bytes allocated and not yet freed: the live heap,
/// which a leak raises whatever the allocator does with freed memory.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc`, passed on unchanged.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        // SAFETY: `p` and `l` come from this allocator, per the caller's contract.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // SAFETY: the caller's contract for `realloc`, passed on unchanged.
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_add(n, Ordering::Relaxed);
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

const RATES: [PcmRate; 3] = [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800];
const CUTOFFS: [Cutoff; 3] = [Cutoff::Standard, Cutoff::Low, Cutoff::Wide];
const FILTERS: [Filter; 3] = [Filter::Default, Filter::Precise, Filter::Pysacd];
const CHANNELS: [u8; 4] = [1, 2, 5, 6];

#[cfg(feature = "flac")]
const FLAC: [&[u8]; 6] = [
    include_bytes!("data/flac/s8_1ch_8000.flac"),
    include_bytes!("data/flac/s12_1ch_22050.flac"),
    include_bytes!("data/flac/s16_2ch_44100.flac"),
    include_bytes!("data/flac/s20_2ch_192000.flac"),
    include_bytes!("data/flac/s24_6ch_96000.flac"),
    include_bytes!("data/flac/s32_1ch_48000.flac"), // refused: 32 bits
];

/// One setting of the cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Setting {
    rate: PcmRate,
    cutoff: Cutoff,
    filter: Filter,
    channels: u8,
}

impl Setting {
    /// Setting `i` of the cycle over every combination.
    fn nth(i: usize) -> Setting {
        let n = [RATES.len(), CUTOFFS.len(), FILTERS.len(), CHANNELS.len()];
        let mut k = i;
        let mut pick = [0usize; 4];
        for (p, n) in pick.iter_mut().zip(n) {
            *p = k % n;
            k /= n;
        }
        Setting {
            rate: RATES[pick[0]],
            cutoff: CUTOFFS[pick[1]],
            filter: FILTERS[pick[2]],
            channels: CHANNELS[pick[3]],
        }
    }

    fn config(self, gain: Gain) -> PcmConfig {
        PcmConfig::new(self.rate, self.cutoff, self.channels)
            .with_filter(self.filter)
            .with_gain(gain)
    }

    /// Whether the library accepts it: no `Wide` at 88.2 kHz; pysacd's filter at 88.2 and
    /// 176.4 kHz with the standard cutoff only.
    fn valid(self) -> bool {
        let wide_ok = !(self.cutoff == Cutoff::Wide && self.rate == PcmRate::Hz88200);
        let pysacd_ok = self.filter != Filter::Pysacd
            || (self.rate != PcmRate::Hz352800 && self.cutoff == Cutoff::Standard);
        wide_ok && pysacd_ok
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// An area of `frames` frames, interleaved, with stretches of full scale and of idle.
fn area(rng: &mut Rng, ch: usize, frames: usize) -> Vec<u8> {
    let len = frames * FRAME_BYTES_PER_CHANNEL * ch;
    (0..len)
        .map(|i| match (i / ch / 2000) % 5 {
            3 => 0xFF,
            4 => 0x69,
            _ => rng.next() as u8,
        })
        .collect()
}

/// The area's bytes per channel.
fn deinterleave(area: &[u8], ch: usize) -> Vec<Vec<u8>> {
    (0..ch)
        .map(|c| area.iter().skip(c).step_by(ch).copied().collect())
        .collect()
}

fn dsf(data: &[Vec<u8>]) -> Vec<u8> {
    let mut w =
        DsfWriter::new(Cursor::new(Vec::new()), data.len() as u16, 2_822_400, None).unwrap();
    w.write_bytes(data).unwrap();
    w.finish(data[0].len() as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

fn dff(data: &[Vec<u8>]) -> Vec<u8> {
    let mut w = DffWriter::new(Cursor::new(Vec::new()), data.len() as u16, 2_822_400).unwrap();
    w.write_bytes(data).unwrap();
    w.finish(data[0].len() as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

/// Kilobytes resident (`VmRSS`), on Linux.
fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Threads of this process, on Linux.
fn threads() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/task").ok()?.count())
}

/// Waits up to a second for the thread count to fall back to `base`: a joined thread can
/// stay listed for a moment after `join` returns. Returns the last count.
fn threads_settle(base: usize) -> usize {
    let t0 = Instant::now();
    loop {
        let n = threads().unwrap_or(base);
        if n <= base || t0.elapsed().as_secs() >= 1 {
            return n;
        }
        std::thread::yield_now();
    }
}

/// Every PCM sample of a reader, per channel.
fn read_pcm(r: &mut dyn PcmRead, max: usize) -> dsd2pcm::Result<Vec<Vec<f64>>> {
    let mut out = vec![Vec::new(); usize::from(r.info().channels)];
    while r.read_frames(max, &mut out)? > 0 {}
    Ok(out)
}

/// One whole-file conversion through `dsd64_to_pcm`: the WAV file's bytes.
fn whole_file(
    reader: Box<dyn DsdRead + Send>,
    ch: u16,
    opts: &PcmOptions,
    cancel_after: Option<usize>,
) -> dsd2pcm::Result<Vec<u8>> {
    let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, ch, opts.rate.hz())?;
    match cancel_after {
        None => dsd64_to_pcm(reader, &mut w, opts)?,
        Some(n) => {
            let mut calls = 0;
            dsd64_to_pcm_with(reader, &mut w, opts, &mut |_| {
                calls += 1;
                if calls > n {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })?
        }
    };
    w.finish()?;
    Ok(w.into_inner().into_inner())
}

/// The data chunk of a 24-bit WAV file made by `WavWriter`.
fn wav_data(wav: &[u8]) -> &[u8] {
    let r = WavReader::new(Cursor::new(wav)).unwrap();
    &wav[dsd2pcm::wav::header_len(WavFormat::I24, r.info().channels) as usize..]
}

struct Stats {
    conversions: u64,
    threaded: u64,
    cancelled: u64,
    errors: u64,
    /// Bytes the kept converters' calls left allocated, and how many of those calls did.
    kept_bytes: isize,
    kept_growths: u64,
    kept_calls: u64,
}

/// A leak put in on purpose, for `soak_catches_a_leak`: bytes kept per iteration by the
/// test, and per call of a kept converter (counted as the converter's own growth).
#[derive(Clone, Copy, Default)]
struct Leak {
    per_iteration: usize,
    per_kept_call: usize,
}

/// One call of a kept converter, its growth (bytes it left allocated) added to `st`. Only
/// this thread runs: a frames converter starts no threads.
fn kept_call<R>(st: &mut Stats, leak: Leak, held: &mut Vec<Vec<u8>>, f: impl FnOnce() -> R) -> R {
    let before = LIVE.load(Ordering::Relaxed);
    let r = f();
    if leak.per_kept_call > 0 {
        held.push(vec![1u8; leak.per_kept_call]);
    }
    let grew = LIVE.load(Ordering::Relaxed) as isize - before as isize;
    st.kept_bytes += grew;
    st.kept_growths += u64::from(grew > 0);
    st.kept_calls += 1;
    r
}

/// One iteration of the cycle.
#[allow(clippy::too_many_arguments)]
fn iteration(
    i: usize,
    rng: &mut Rng,
    reused: &mut HashMap<(Setting, u64), Converter>,
    tmp: &std::path::Path,
    base_threads: Option<usize>,
    st: &mut Stats,
    leak: Leak,
    held: &mut Vec<Vec<u8>>,
) {
    if leak.per_iteration > 0 {
        // Written, so that it is resident as well.
        held.push(vec![1u8; leak.per_iteration]);
    }
    let s = Setting::nth(i);
    let ch = usize::from(s.channels);
    let gain = Gain::from_db([0.0, 6.0, -3.5][i % 3]).unwrap();
    let gain_key = gain.db().to_bits();
    let cfg = s.config(gain);

    if !s.valid() {
        // Error paths: the converter and the whole-file path both refuse.
        assert!(matches!(
            Converter::new(cfg),
            Err(Error::InvalidArgument(_))
        ));
        let data = vec![vec![0x69u8; 64]; ch];
        let mut opts = PcmOptions::new(s.rate);
        opts.cutoff = s.cutoff;
        opts.filter = s.filter;
        let r = DsfReader::new(Cursor::new(dsf(&data))).unwrap();
        assert!(whole_file(Box::new(r), ch as u16, &opts, None).is_err());
        st.errors += 2;
        return;
    }

    // Every 16th valid iteration a long area (over 1 MiB per channel: several blocks of
    // the threaded file path), otherwise 3 to 14 frames.
    let frames = if i % 16 == 5 { 230 } else { 3 + rng.below(12) };
    let area = area(rng, ch, frames);
    let frame = FRAME_BYTES_PER_CHANNEL * ch;

    // Frames: a reused converter (one per setting and gain, kept for the whole run) and a
    // fresh one, on the whole area in one buffer.
    let conv = reused
        .entry((s, gain_key))
        .or_insert_with(|| Converter::new(cfg).unwrap());
    let spf = conv.samples_per_frame();
    let mut whole = vec![0u8; frames * spf * ch * 3];
    let mut fc = vec![0u64; frames];
    let stats = kept_call(st, leak, held, || {
        conv.convert(
            Run::new(&area[..]),
            PcmOut::S24le(&mut whole),
            Some(&mut fc),
        )
    })
    .unwrap();
    assert_eq!(fc.iter().sum::<u64>(), stats.clipped_total());
    let mut fresh = Converter::new(cfg).unwrap();
    let mut again = vec![0u8; whole.len()];
    fresh
        .convert(Run::new(&area[..]), PcmOut::S24le(&mut again), None)
        .unwrap();
    assert!(whole == again, "{s:?}: reused and fresh converters differ");
    st.conversions += 2;

    // Frame by frame with neighbours, one buffer per frame, as i32: the same samples.
    let list: Vec<&[u8]> = area.chunks_exact(frame).collect();
    let mut one = vec![0i32; spf * ch];
    let step = if frames > 20 { 37 } else { 1 };
    for f in (0..frames).step_by(step) {
        let run = Run::new(&list[f..f + 1])
            .at_frame(f as u64)
            .with_before(f.checked_sub(1).map(|p| list[p]))
            .with_after(list.get(f + 1).copied());
        kept_call(st, leak, held, || {
            conv.convert(run, PcmOut::I32(&mut one), None)
        })
        .unwrap();
        let want = &whole[f * spf * ch * 3..][..spf * ch * 3];
        let same = one.iter().zip(want.chunks_exact(3)).all(|(v, b)| {
            let w = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
            *v == w
        });
        assert!(same, "{s:?}: frame {f} differs");
        st.conversions += 1;
    }

    // Whole file, from an in-memory DSF or DFF, threaded or not: the same bytes.
    let data = deinterleave(&area, ch);
    let file = if i % 2 == 0 { dsf(&data) } else { dff(&data) };
    let reader = |file: &[u8]| -> Box<dyn DsdRead + Send> {
        if i % 2 == 0 {
            Box::new(DsfReader::new(Cursor::new(file.to_vec())).unwrap())
        } else {
            Box::new(DffReader::new(Cursor::new(file.to_vec())).unwrap())
        }
    };
    let mut opts = PcmOptions::new(s.rate);
    opts.cutoff = s.cutoff;
    opts.filter = s.filter;
    opts.gain_db = gain.db();
    for threaded in [false, true] {
        opts.threads = threaded;
        let wav = whole_file(reader(&file), ch as u16, &opts, None).unwrap();
        assert!(wav_data(&wav) == &whole[..], "{s:?}: file path differs");
        st.conversions += 1;
        if threaded {
            st.threaded += 1;
            if let Some(base) = base_threads {
                let now = threads_settle(base);
                assert!(now <= base, "iteration {i}: {now} threads, {base} before");
            }
        }
        // The WAV file reads back.
        let mut r = WavReader::new(Cursor::new(wav)).unwrap();
        let pcm = read_pcm(&mut r, 1 + rng.below(5000)).unwrap();
        assert_eq!(pcm[0].len(), frames * spf);
    }
    // Cancelled from the hook, before or after the first block.
    opts.threads = i % 3 != 0;
    let r = whole_file(reader(&file), ch as u16, &opts, Some(i % 2));
    if i % 2 == 0 {
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        st.cancelled += 1;
    }
    if let Some(base) = base_threads {
        let now = threads_settle(base);
        assert!(now <= base, "iteration {i}: {now} threads, {base} before");
    }

    // Truncated files: refused, never a panic.
    let cut = &file[..file.len() / 2 + rng.below(file.len() / 2)];
    let r: dsd2pcm::Result<Box<dyn DsdRead + Send>> = if i % 2 == 0 {
        DsfReader::new(Cursor::new(cut.to_vec())).map(|r| Box::new(r) as _)
    } else {
        DffReader::new(Cursor::new(cut.to_vec())).map(|r| Box::new(r) as _)
    };
    if let Ok(r) = r {
        opts.threads = false;
        let _ = whole_file(r, ch as u16, &opts, None);
    }
    st.errors += 1;

    // FLAC, whole and truncated.
    #[cfg(feature = "flac")]
    {
        let bytes = FLAC[i % FLAC.len()];
        match dsd2pcm::flac::FlacReader::new(Cursor::new(bytes)) {
            Ok(mut r) => {
                let n = r.info().frames as usize;
                let pcm = read_pcm(&mut r, 1 + rng.below(3000)).unwrap();
                assert_eq!(pcm[0].len(), n);
            }
            // 32-bit FLAC is refused.
            Err(e) => {
                assert!(matches!(e, Error::Unsupported(_)), "{e:?}");
                st.errors += 1;
            }
        }
        let cut = &bytes[..bytes.len() * 2 / 3];
        if let Ok(mut r) = dsd2pcm::flac::FlacReader::new(Cursor::new(cut)) {
            assert!(read_pcm(&mut r, 4096).is_err());
        }
    }

    // A file on disk now and then, with a progress hook.
    if i % 10 == 7 {
        let (inp, out) = (tmp.join("in.dsf"), tmp.join("out.wav"));
        std::fs::write(&inp, dsf(&data)).unwrap();
        opts.threads = true;
        let mut calls = 0u32;
        let rep = convert_file_with(&inp, &out, &opts, &mut |_| {
            calls += 1;
            ControlFlow::Continue(())
        })
        .unwrap();
        assert!(calls >= 2);
        assert_eq!(rep.out_samples, (frames * spf) as u64);
        let wav = std::fs::read(&out).unwrap();
        assert!(wav_data(&wav) == &whole[..]);
        st.conversions += 1;
        st.threaded += 1;
    }
}

/// Settings in the cycle.
const CYCLE: usize = RATES.len() * CUTOFFS.len() * FILTERS.len() * CHANNELS.len();
/// The warm-up: two rounds of the cycle, after which every kept converter exists and has run
/// twice, and the library has built its tables.
const WARM_UP: usize = 2 * CYCLE;
/// The fewest iterations the memory is judged on: the warm-up, then two windows of about
/// 400 iterations to compare. A shorter run reports the figures without judging them.
const MIN_JUDGED: usize = 1000;
/// How far the live heap (less the kept converters' growth) may rise.
const HEAP_SLACK: usize = 64 * 1024;
/// How much the kept converters may grow in the second window, and on what share of their
/// calls there (1 %).
const KEPT_SLACK: isize = 4 << 20;
const KEPT_GROWTH_SHARE: u64 = 100;

/// The soak tests share the counters of one process: one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What a run measured after each iteration.
struct Measured {
    iterations: usize,
    /// The live heap less what the kept converters grew by: the test's own state, every
    /// other path of the library, and the kept converters as created.
    heap: Vec<usize>,
    /// The kept converters' growth so far: bytes, calls that grew, all calls.
    kept_bytes: Vec<isize>,
    kept_growths: Vec<u64>,
    kept_calls: Vec<u64>,
    /// `VmRSS`, KiB, where `/proc/self/status` exists.
    rss: Option<Vec<u64>>,
    /// Worker threads of a threaded conversion.
    workers: usize,
}

/// Runs the cycle for `max_iterations` or until `deadline`, with `leak` put in.
fn soak_run(name: &str, max_iterations: usize, deadline: Option<Instant>, leak: Leak) -> Measured {
    let tmp = std::env::temp_dir().join(format!("dsd2pcm-soak-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut reused = HashMap::new();
    let mut held = Vec::new();
    let mut st = Stats {
        conversions: 0,
        threaded: 0,
        cancelled: 0,
        errors: 0,
        kept_bytes: 0,
        kept_growths: 0,
        kept_calls: 0,
    };
    let base_threads = threads();
    // Written before the run (not zero, which the allocator can hand out as untouched
    // pages), so their pages do not add to the resident set on the way.
    let mut rss = vec![u64::MAX; max_iterations];
    let mut heap = vec![usize::MAX; max_iterations];
    let mut kept_bytes = vec![isize::MAX; max_iterations];
    let mut kept_growths = vec![u64::MAX; max_iterations];
    let mut kept_calls = vec![u64::MAX; max_iterations];
    let have_rss = rss_kib().is_some();
    let t0 = Instant::now();
    let mut iterations = 0;
    for i in 0..max_iterations {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
        iteration(
            i,
            &mut rng,
            &mut reused,
            &tmp,
            base_threads,
            &mut st,
            leak,
            &mut held,
        );
        rss[i] = rss_kib().unwrap_or(0);
        heap[i] = (LIVE.load(Ordering::Relaxed) as isize - st.kept_bytes) as usize;
        kept_bytes[i] = st.kept_bytes;
        kept_growths[i] = st.kept_growths;
        kept_calls[i] = st.kept_calls;
        iterations += 1;
    }
    std::fs::remove_dir_all(&tmp).unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let max_ch = usize::from(MAX_CHANNELS);
    println!(
        "{name}: {iterations} iterations in {secs:.1} s: {} conversions ({} threaded), {} cancelled, \
         {} error paths, {} reused converters, up to {max_ch} channels, {} worker threads",
        st.conversions,
        st.threaded,
        st.cancelled,
        st.errors,
        reused.len(),
        workers
    );
    if let Some(base) = base_threads {
        println!(
            "{name}: threads {base} before, {} after",
            threads().unwrap_or(0)
        );
    }
    heap.truncate(iterations);
    kept_bytes.truncate(iterations);
    kept_growths.truncate(iterations);
    kept_calls.truncate(iterations);
    rss.truncate(iterations);
    Measured {
        iterations,
        heap,
        kept_bytes,
        kept_growths,
        kept_calls,
        rss: have_rss.then_some(rss),
        workers,
    }
}

/// The checks' verdicts: true where the memory levels off.
struct Verdicts {
    heap: bool,
    kept: bool,
    rss: Option<bool>,
}

fn median(v: &[u64]) -> u64 {
    let mut s = v.to_vec();
    s.sort_unstable();
    s[s.len() / 2]
}

/// Judges a run of `MIN_JUDGED` iterations or more (`None` for a shorter one), printing the
/// figures. After the warm-up the run is cut in two windows, the plateau and the late one:
/// - the live heap less the kept converters' growth (the leak measure: a leak raises it
///   whatever the allocator does with freed memory) may rise at most `HEAP_SLACK` from the
///   plateau's highest value to the late window's;
/// - the kept converters grow only while their buffers reach the sizes their calls need,
///   which gets rarer as the run goes on: in the late window they may grow on at most 1 %
///   of their calls and by at most `KEPT_SLACK` (a leak in a call grows on every call);
/// - the resident set's median in the late window may be at most 25 % or 64 MiB plus 2 MiB
///   per worker thread, whichever is larger, above its median in the plateau. Medians,
///   because the allocator holds on to freed memory per worker thread for a while: one
///   block of a few tens of MiB now and then, without a leak; and a wide bound, because the
///   medians themselves drift without one while the worker threads' arenas fill (up to 46
///   MiB with 4 workers and 49 MiB with 16 in runs of 1000 iterations). The live heap is the
///   precise check; this one is a coarse one for memory outside it.
fn judge(name: &str, m: &Measured) -> Option<Verdicts> {
    let n = m.iterations;
    let deciles = |v: &dyn Fn(usize) -> u64| -> String {
        (1..=10)
            .map(|d| format!("{}", v(d * n / 10 - 1)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "{name}: live heap (less kept converters' growth) KiB at each tenth: {}",
        deciles(&|i| m.heap[i] as u64 / 1024)
    );
    println!(
        "{name}: kept converters' growth KiB at each tenth: {}",
        deciles(&|i| (m.kept_bytes[i] / 1024) as u64)
    );
    if let Some(rss) = &m.rss {
        println!(
            "{name}: RSS MiB at each tenth: {}",
            deciles(&|i| rss[i] / 1024)
        );
    }
    if n < MIN_JUDGED {
        println!(
            "{name}: memory not judged: {n} iterations, the checks need {MIN_JUDGED} or more \
             (a warm-up of {WARM_UP}, then two windows)"
        );
        return None;
    }
    let mid = WARM_UP + (n - WARM_UP) / 2;
    let (plateau, late) = (WARM_UP..mid, mid..n);

    let heap_plateau = *m.heap[plateau.clone()].iter().max().unwrap();
    let heap_late = *m.heap[late.clone()].iter().max().unwrap();
    let heap = heap_late <= heap_plateau + HEAP_SLACK;
    println!(
        "{name}: live heap: plateau (iterations {WARM_UP}-{mid}) max {heap_plateau} bytes, late \
         max {heap_late} bytes: {:+} bytes, bound +{HEAP_SLACK}: {}",
        heap_late as isize - heap_plateau as isize,
        if heap { "levels off" } else { "GREW" }
    );

    let at = |v: &[u64], i: usize| if i == 0 { 0 } else { v[i - 1] };
    let calls = at(&m.kept_calls, n) - at(&m.kept_calls, mid);
    let growths = at(&m.kept_growths, n) - at(&m.kept_growths, mid);
    let grew = m.kept_bytes[n - 1] - m.kept_bytes[mid - 1];
    let kept = growths * KEPT_GROWTH_SHARE <= calls && grew <= KEPT_SLACK;
    println!(
        "{name}: kept converters, late window: grew on {growths} of {calls} calls, by {grew} \
         bytes; bound 1 % of the calls and {KEPT_SLACK} bytes: {}",
        if kept { "levels off" } else { "GREW" }
    );

    let rss = m.rss.as_ref().map(|rss| {
        let p = median(&rss[plateau.clone()]);
        let l = median(&rss[late.clone()]);
        let bound = p + (p / 4).max((64 + 2 * m.workers as u64) * 1024);
        let ok = l <= bound;
        println!(
            "{name}: RSS: plateau median {} MiB (max {}), late median {} MiB (max {}), bound {} \
             MiB: {}",
            p / 1024,
            rss[plateau.clone()].iter().max().unwrap() / 1024,
            l / 1024,
            rss[late.clone()].iter().max().unwrap() / 1024,
            bound / 1024,
            if ok { "levels off" } else { "GREW" }
        );
        ok
    });
    if rss.is_none() {
        println!("{name}: no /proc/self/status: RSS not checked");
    }
    Some(Verdicts { heap, kept, rss })
}

#[test]
#[ignore = "soak test: minutes to hours; run with --ignored (see the file's comment)"]
fn soak() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let env = |name: &str| -> Option<u64> {
        let v = std::env::var(name).ok()?;
        Some(v.parse().unwrap_or_else(|_| panic!("{name}: a number")))
    };
    let seconds = env("DSD2PCM_SOAK_SECONDS");
    // With a duration, the iterations are only a ceiling.
    let max_iterations =
        env("DSD2PCM_SOAK_ITERATIONS").unwrap_or(if seconds.is_some() { 4_000_000 } else { 300 })
            as usize;
    let deadline = seconds.map(|s| Instant::now() + std::time::Duration::from_secs(s));
    let m = soak_run("soak", max_iterations, deadline, Leak::default());
    assert!(
        m.iterations >= 50,
        "at least 50 iterations, ran {}",
        m.iterations
    );
    let Some(v) = judge("soak", &m) else { return };
    // Under a sanitizer the resident set also holds its shadow memory and history, which
    // grow with the run: DSD2PCM_SOAK_RSS_REPORT_ONLY reports it without the check, and
    // DSD2PCM_SOAK_HEAP_REPORT_ONLY does the same for the live heap and kept converters.
    let heap_only = std::env::var_os("DSD2PCM_SOAK_HEAP_REPORT_ONLY").is_some();
    let rss_only = std::env::var_os("DSD2PCM_SOAK_RSS_REPORT_ONLY").is_some();
    assert!(v.heap || heap_only, "live heap grew (figures above)");
    assert!(v.kept || heap_only, "kept converters grew (figures above)");
    assert!(
        v.rss.unwrap_or(true) || rss_only,
        "RSS grew (figures above)"
    );
}

/// The checks catch a leak: runs of `MIN_JUDGED` iterations with one put in.
#[test]
#[ignore = "soak test: about a minute; run with --ignored (see the file's comment)"]
fn soak_catches_a_leak() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 1 MiB per iteration, written: the live heap and the resident set both catch it.
    let name = "leak of 1 MiB per iteration";
    let leak = Leak {
        per_iteration: 1 << 20,
        per_kept_call: 0,
    };
    let v = judge(name, &soak_run(name, MIN_JUDGED, None, leak)).unwrap();
    assert!(!v.heap, "{name}: passed the live-heap check");
    assert!(!v.rss.unwrap_or(false), "{name}: passed the RSS check");
    // 1 KiB per iteration, and 64 bytes per call of a kept converter: the live heap and
    // the kept converters' check each catch theirs.
    let name = "leak of 1 KiB per iteration and 64 bytes per kept call";
    let leak = Leak {
        per_iteration: 1 << 10,
        per_kept_call: 64,
    };
    let v = judge(name, &soak_run(name, MIN_JUDGED, None, leak)).unwrap();
    assert!(!v.heap, "{name}: passed the live-heap check");
    assert!(!v.kept, "{name}: passed the kept converters' check");
}
