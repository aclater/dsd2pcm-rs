//! Heap-flat, exact (the lab's memory-testing standard): after one warm-up conversion, the
//! live heap (bytes allocated and not freed, counted by a global allocator) read when no
//! conversion is running is the SAME number of bytes after every further conversion, for
//! every setting and for the failure paths as well as success. Equality is the condition;
//! no margin. A leak of one byte per conversion fails it.
//!
//! The condition is itself tested: `the_gate_catches_a_leak` (run first, in the same test) runs the same code with a
//! deliberate leak (one byte, and a few KiB, per conversion) and requires the gate to FAIL.
//!
//! One `#[test]` runs everything, so the harness has no other test running that could
//! allocate between two readings. Run with the default `cargo test -p dsd2pcm --test
//! heap_flat`; `HEAP_FLAT_CONVERSIONS` sets the conversions per failure-path case (30 by
//! default, for every pull request; the release gate sets 100, the standard's count).

use dsd2pcm::convert::{dsd64_to_pcm, dsd64_to_pcm_with};
use dsd2pcm::dff::{DffReader, DffWriter};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite};
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, Filter, PcmOptions, PcmRate, Threads};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The system allocator, counting the live bytes.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc`, passed on unchanged.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::SeqCst);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        // SAFETY: `p` and `l` come from this allocator, per the caller's contract.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // SAFETY: the caller's contract for `realloc`, passed on unchanged.
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_add(n, Ordering::SeqCst);
            LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Bytes per channel of one SACD frame, as the inputs are made of whole frames.
const FRAME: usize = 4704;

/// A leak put in on purpose, per conversion.
#[derive(Clone, Copy)]
enum Leak {
    None,
    Bytes(usize),
}

#[derive(Clone, Copy, Debug)]
enum Container {
    Dsf,
    Dff,
}

/// What the conversion meets.
#[derive(Clone, Copy, Debug)]
enum Path {
    /// A good file, converted whole.
    Success,
    /// Cancelled from the progress hook on its first call.
    Cancelled,
    /// A file cut off in the middle of its audio.
    Truncated,
    /// A file whose first bytes are not a container header.
    Corrupt,
    /// An output that fails after 100 bytes.
    OutputFails,
}

/// Interleaved-by-channel bytes: pseudo-random, with stretches of idle and full scale.
fn channels(ch: usize, frames: usize) -> Vec<Vec<u8>> {
    let mut s = 0x2545_f491_4f6c_dd1du64;
    (0..ch)
        .map(|c| {
            (0..frames * FRAME)
                .map(|i| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    match (i / 1500 + c) % 5 {
                        3 => 0xFF,
                        4 => 0x69,
                        _ => s as u8,
                    }
                })
                .collect()
        })
        .collect()
}

fn file(container: Container, data: &[Vec<u8>]) -> Vec<u8> {
    let bits = data[0].len() as u64 * 8;
    match container {
        Container::Dsf => {
            let mut w = DsfWriter::new(Cursor::new(Vec::new()), data.len() as u16, 2_822_400, None)
                .unwrap();
            w.write_bytes(data).unwrap();
            w.finish(bits).unwrap();
            w.into_inner().into_inner()
        }
        Container::Dff => {
            let mut w =
                DffWriter::new(Cursor::new(Vec::new()), data.len() as u16, 2_822_400).unwrap();
            w.write_bytes(data).unwrap();
            w.finish(bits).unwrap();
            w.into_inner().into_inner()
        }
    }
}

fn reader(container: Container, bytes: Vec<u8>) -> dsd2pcm::Result<Box<dyn DsdRead + Send>> {
    Ok(match container {
        Container::Dsf => Box::new(DsfReader::new(Cursor::new(bytes))?),
        Container::Dff => Box::new(DffReader::new(Cursor::new(bytes))?),
    })
}

/// A seekable output that fails after `limit` bytes.
struct Failing {
    inner: Cursor<Vec<u8>>,
    limit: usize,
}

impl Write for Failing {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.inner.get_ref().len() + b.len() > self.limit {
            return Err(std::io::Error::other("the output fails"));
        }
        self.inner.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for Failing {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(p)
    }
}

/// One conversion on `path`, everything it allocated dropped before it returns.
fn convert_once(container: Container, good: &[u8], ch: u16, opts: &PcmOptions, path: Path) {
    let input = match path {
        Path::Truncated => good[..good.len() * 2 / 3].to_vec(),
        Path::Corrupt => {
            let mut b = good.to_vec();
            b[..8].fill(0xA5);
            b
        }
        _ => good.to_vec(),
    };
    let Ok(r) = reader(container, input) else {
        return; // the header was refused: that is the Corrupt path's outcome
    };
    match path {
        Path::OutputFails => {
            let out = Failing {
                inner: Cursor::new(Vec::new()),
                limit: 100,
            };
            if let Ok(mut w) = WavWriter::new(out, WavFormat::I24, ch, opts.rate.hz()) {
                let _ = dsd64_to_pcm(r, &mut w, opts);
            }
        }
        Path::Cancelled => {
            let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, ch, opts.rate.hz())
                .unwrap();
            let _ = dsd64_to_pcm_with(r, &mut w, opts, &mut |_| ControlFlow::Break(()));
        }
        _ => {
            let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, ch, opts.rate.hz())
                .unwrap();
            let res = dsd64_to_pcm(r, &mut w, opts);
            if matches!(path, Path::Success) {
                res.unwrap();
            }
        }
    }
}

/// The gate: every reading equals the first. `Err` names the first difference.
fn gate(series: &[usize]) -> Result<(), String> {
    match series.iter().position(|&x| x != series[0]) {
        None => Ok(()),
        Some(i) => Err(format!(
            "live heap {} bytes after conversion {} but {} after the first (a rise of {})",
            series[i],
            i + 1,
            series[0],
            series[i] as i64 - series[0] as i64
        )),
    }
}

/// Warm-up conversion, then `n` conversions with a reading after each.
fn run_case(
    container: Container,
    data: &[Vec<u8>],
    opts: &PcmOptions,
    path: Path,
    n: usize,
    leak: Leak,
) -> Vec<usize> {
    let good = file(container, data);
    let ch = data.len() as u16;
    convert_once(container, &good, ch, opts, path); // warm-up: the one-time allocations
    (0..n)
        .map(|_| {
            convert_once(container, &good, ch, opts, path);
            if let Leak::Bytes(b) = leak {
                std::mem::forget(vec![0u8; b]); // the deliberate leak
            }
            LIVE.load(Ordering::SeqCst)
        })
        .collect()
}

fn conversions() -> usize {
    std::env::var("HEAP_FLAT_CONVERSIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

/// The settings the library takes: no wide at 88.2 kHz; pysacd at 88.2 and 176.4 kHz, standard cutoff.
fn valid(rate: PcmRate, cutoff: Cutoff, filter: Filter) -> bool {
    !(cutoff == Cutoff::Wide && rate == PcmRate::Hz88200)
        && (filter != Filter::Pysacd || (rate != PcmRate::Hz352800 && cutoff == Cutoff::Standard))
}

fn live_heap_is_flat_after_warm_up() {
    let n = conversions();
    let mut failures = Vec::new();
    let mut cases = 0usize;
    // Success: every filter, rate, cutoff and channel count, threaded and on one thread.
    for filter in [Filter::Precise, Filter::Fast, Filter::Pysacd] {
        for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
            for cutoff in [Cutoff::Standard, Cutoff::Low, Cutoff::Wide] {
                if !valid(rate, cutoff, filter) {
                    continue;
                }
                for ch in [1usize, 2, 6] {
                    for threads in [Threads::Single, Threads::All] {
                        let mut opts = PcmOptions::new(rate);
                        opts.cutoff = cutoff;
                        opts.filter = filter;
                        opts.threads = threads;
                        let container = if ch == 2 {
                            Container::Dff
                        } else {
                            Container::Dsf
                        };
                        let data = channels(ch, 2);
                        let s = run_case(
                            container,
                            &data,
                            &opts,
                            Path::Success,
                            n.min(10),
                            Leak::None,
                        );
                        cases += 1;
                        if let Err(e) = gate(&s) {
                            failures.push(format!(
                                "{filter:?} {rate:?} {cutoff:?} {ch}ch {threads:?} Success: {e}"
                            ));
                        }
                    }
                }
            }
        }
    }
    // The failure paths, each on its own, with the full count of conversions.
    for filter in [Filter::Precise, Filter::Fast, Filter::Pysacd] {
        for ch in [2usize, 6] {
            for threads in [Threads::Single, Threads::All] {
                for container in [Container::Dsf, Container::Dff] {
                    for path in [
                        Path::Cancelled,
                        Path::Truncated,
                        Path::Corrupt,
                        Path::OutputFails,
                    ] {
                        let mut opts = PcmOptions::new(PcmRate::Hz176400);
                        opts.filter = filter;
                        opts.threads = threads;
                        let data = channels(ch, 2);
                        let s = run_case(container, &data, &opts, path, n, Leak::None);
                        cases += 1;
                        if let Err(e) = gate(&s) {
                            failures.push(format!(
                                "{filter:?} {ch}ch {threads:?} {container:?} {path:?}: {e}"
                            ));
                        }
                    }
                }
            }
        }
    }
    eprintln!("heap_flat: {cases} cases, {n} conversions per failure-path case");
    assert!(
        failures.is_empty(),
        "the live heap moved:\n{}",
        failures.join("\n")
    );
}

/// Positive control: the same code with a leak must FAIL the gate: one byte and a few KiB
/// per conversion (the smallest leak the gate can see is one byte per conversion: equality
/// is the condition).
fn the_gate_catches_a_leak() {
    let data = channels(2, 2);
    let opts = PcmOptions::new(PcmRate::Hz176400);
    for bytes in [1usize, 4096] {
        let s = run_case(
            Container::Dsf,
            &data,
            &opts,
            Path::Success,
            20,
            Leak::Bytes(bytes),
        );
        let verdict = gate(&s);
        assert!(
            verdict.is_err(),
            "the gate passed a leak of {bytes} byte(s) per conversion"
        );
    }
    // And the same case without the leak passes, so the control is the leak and nothing else.
    let s = run_case(Container::Dsf, &data, &opts, Path::Success, 20, Leak::None);
    assert_eq!(gate(&s), Ok(()));
}

/// The only test: the control first (it must fail the gate), then the gate itself, in one
/// test so that nothing else in the process allocates between two readings.
#[test]
fn heap_flat() {
    the_gate_catches_a_leak();
    live_heap_is_flat_after_warm_up();
}
