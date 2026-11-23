//! Soak test for long-lived processes: thousands of conversions in one process, and the
//! resident memory levels off and every worker thread ends.
//!
//! Ignored by default. Run it with
//!
//! ```text
//! DSD2PCM_SOAK_ITERATIONS=2000 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture
//! ```
//!
//! `DSD2PCM_SOAK_ITERATIONS` (default 300) sets the iterations; with `DSD2PCM_SOAK_SECONDS`
//! the run stops after that many seconds instead (the iterations, if set, are a ceiling). Each iteration converts one
//! pseudo-random area with one setting of the cycle below (every rate, cutoff, filter and
//! channel count, invalid combinations included as error paths): with a reused and a fresh
//! `frames::Converter` (contiguous and per-frame buffers, a whole run and frame by frame),
//! as an in-memory DSF or DFF file through `dsd64_to_pcm` (threaded and on one thread, now
//! and then cancelled from the progress hook), reads the WAV back, reads a FLAC file, feeds
//! the readers truncated files, and every tenth iteration converts a file on disk with
//! `convert_file_with`. The outputs of the paths are compared with each other.
//!
//! It checks, after the warm-up (the first 20 % of iterations):
//! - the live heap (bytes allocated and not freed, counted by a global allocator) after
//!   each iteration of the second half is at most 64 KiB above the highest value of 20 to
//!   50 % of the run;
//!
//! and on Linux, from `/proc/self`:
//! - the resident set size (`VmRSS`) of the second half stays within 10 % or 32 MiB
//!   (whichever is larger) of the highest value of 20 to 50 %
//!   (`DSD2PCM_SOAK_RSS_REPORT_ONLY` only reports it, for sanitizer builds);
//! - the thread count is back at its value from before the run after every threaded
//!   conversion.

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
}

/// One iteration of the cycle.
fn iteration(
    i: usize,
    rng: &mut Rng,
    reused: &mut HashMap<(Setting, u64), Converter>,
    tmp: &std::path::Path,
    base_threads: Option<usize>,
    st: &mut Stats,
) {
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
    let stats = conv
        .convert(
            Run::new(&area[..]),
            PcmOut::S24le(&mut whole),
            Some(&mut fc),
        )
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
        conv.convert(run, PcmOut::I32(&mut one), None).unwrap();
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

#[test]
#[ignore = "soak test: minutes to hours; run with --ignored (see the file's comment)"]
fn soak() {
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
    let tmp = std::env::temp_dir().join(format!("dsd2pcm-soak-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut reused = HashMap::new();
    let mut st = Stats {
        conversions: 0,
        threaded: 0,
        cancelled: 0,
        errors: 0,
    };
    let base_threads = threads();
    // Written before the run, so its pages do not add to the resident set on the way.
    let mut rss = vec![0u64; max_iterations];
    let mut live = vec![0usize; max_iterations];
    let have_rss = rss_kib().is_some();
    let t0 = Instant::now();
    let mut iterations = 0;
    for (i, (r, h)) in rss.iter_mut().zip(live.iter_mut()).enumerate() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
        iteration(i, &mut rng, &mut reused, &tmp, base_threads, &mut st);
        *r = rss_kib().unwrap_or(0);
        *h = LIVE.load(Ordering::Relaxed);
        iterations += 1;
    }
    rss.truncate(iterations);
    live.truncate(iterations);
    assert!(iterations >= 50, "at least 50 iterations, ran {iterations}");
    std::fs::remove_dir_all(&tmp).unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let max_ch = usize::from(MAX_CHANNELS);
    println!(
        "soak: {iterations} iterations in {secs:.1} s: {} conversions ({} threaded), {} cancelled, \
         {} error paths, {} reused converters, up to {max_ch} channels",
        st.conversions,
        st.threaded,
        st.cancelled,
        st.errors,
        reused.len()
    );
    if let Some(base) = base_threads {
        println!(
            "soak: threads {base} before, {} after",
            threads().unwrap_or(0)
        );
    }
    let warm = iterations / 5;
    let half = iterations / 2;
    // The live heap after each iteration: the converters kept for the run and the test's
    // own state. Once every kept converter has met its longest run it does not move.
    let deciles: Vec<String> = (1..=10)
        .map(|d| format!("{}", live[d * iterations / 10 - 1] / 1024))
        .collect();
    let live_plateau = *live[warm..half].iter().max().unwrap();
    let live_late = *live[half..].iter().max().unwrap();
    println!(
        "soak: live heap KiB at each tenth of the run: {}; plateau (20-50 %) {} KiB; second-half \
         max {} KiB",
        deciles.join(" "),
        live_plateau / 1024,
        live_late / 1024
    );
    assert!(
        live_late <= live_plateau + 64 * 1024,
        "live heap grew: {live_late} bytes in the second half, plateau {live_plateau} bytes"
    );
    if have_rss {
        let deciles: Vec<String> = (1..=10)
            .map(|d| format!("{}", rss[d * iterations / 10 - 1] / 1024))
            .collect();
        let plateau = *rss[warm..half].iter().max().unwrap();
        let late = *rss[half..].iter().max().unwrap();
        let mut sorted = rss[half..].to_vec();
        sorted.sort_unstable();
        // Looser than the live heap's bound: the resident set also holds what the allocator
        // keeps of freed memory, spread over the worker threads' arenas.
        let bound = plateau + (plateau / 10).max(32 * 1024);
        println!(
            "soak: RSS MiB at each tenth of the run: {}; plateau (20-50 %) {} MiB; second half \
             min {} median {} max {} MiB; bound {} MiB",
            deciles.join(" "),
            plateau / 1024,
            sorted[0] / 1024,
            sorted[sorted.len() / 2] / 1024,
            late / 1024,
            bound / 1024
        );
        // Under a sanitizer the resident set also holds its shadow memory and history,
        // which grow with the run: DSD2PCM_SOAK_RSS_REPORT_ONLY reports without checking.
        assert!(
            late <= bound || std::env::var_os("DSD2PCM_SOAK_RSS_REPORT_ONLY").is_some(),
            "RSS grew: {late} KiB in the second half, plateau {plateau} KiB"
        );
    } else {
        println!("soak: no /proc/self/status: RSS not checked");
    }
}
