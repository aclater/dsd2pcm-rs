//! `frames::Converter::new` sizes every buffer, so `Converter::convert` allocates nothing,
//! from the first call on: whatever the runs (length, neighbours, buffer layout, output
//! format, position in the area, order of the calls), at every rate, cutoff, filter, channel
//! count and SIMD level (`.github/scripts/test-simd-levels.sh` runs it at each). Before 0.2.0 the buffers grew on later calls: a run longer than any
//! before, a short run after a long one (buffers that trade places with the carry), a run at
//! the edge of an area.

use dsd2pcm::frames::{Converter, PcmConfig, PcmOut, Run, FRAME_BYTES_PER_CHANNEL};
use dsd2pcm::{Cutoff, Filter, PcmRate};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts allocations per thread, so that tests running in parallel do not interfere.
struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
}

fn allocs() -> usize {
    ALLOCS.with(Cell::get)
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        // SAFETY: the caller's contract for `alloc`, passed on unchanged.
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count();
        // SAFETY: as for `alloc`.
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: `p` and `l` come from this allocator, per the caller's contract.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        // SAFETY: the caller's contract for `realloc`, passed on unchanged.
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Frames in the test area.
const AREA: usize = 14;

/// The calls, in order: `(first frame, frames, neighbours, separate buffers, i32 output)`.
/// The shortest run first, with no neighbours (the edges of the area); then the patterns
/// that grew buffers before: a long run (several blocks, a short last one) after it; frame
/// by frame with neighbours, as the soak test runs; whole areas of 3 to 14 frames, edges
/// and all; a short run with neighbours after a long one, and the reverse.
fn calls() -> Vec<(usize, usize, bool, bool, bool)> {
    let mut v = vec![(0, 1, false, false, false), (0, AREA, false, false, false)];
    v.extend((0..AREA).step_by(2).map(|f| (f, 1, true, true, true)));
    v.extend([3, 14, 6, 7, 13].map(|n| (0, n, false, false, false)));
    v.extend([
        (2, 1, true, false, false),
        (1, 12, true, true, false),
        (5, 2, true, false, true),
        (0, 12, true, false, true),
        (7, 7, true, true, true),
        (13, 1, true, false, false),
    ]);
    v
}

/// Every converter with `filter` allocates nothing in any of `calls()`; returns how many
/// converters ran.
fn case(filter: Filter) -> usize {
    let mut made = 0;
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
        for cutoff in [Cutoff::Standard, Cutoff::Low, Cutoff::Wide] {
            for ch in [1u8, 2, 6] {
                let cfg = PcmConfig::new(rate, cutoff, ch).with_filter(filter);
                // Invalid combinations (wide at 88.2 kHz, pysacd's limits) are refused.
                let Ok(mut c) = Converter::new(cfg) else {
                    continue;
                };
                made += 1;
                let ch = usize::from(ch);
                let f = FRAME_BYTES_PER_CHANNEL * ch;
                // Noise, full scale and idle, as in the soak test.
                let area: Vec<u8> = (0..AREA * f)
                    .map(|i| match (i / ch / 2000) % 5 {
                        3 => 0xFF,
                        4 => 0x69,
                        _ => (i.wrapping_mul(2_654_435_761) >> 13) as u8,
                    })
                    .collect();
                let list: Vec<&[u8]> = area.chunks_exact(f).collect();
                let spf = c.samples_per_frame();
                let mut s24 = vec![0u8; AREA * spf * ch * 3];
                let mut i32s = vec![0i32; AREA * spf * ch];
                let mut fc = [0u64; AREA];
                let calls = calls();
                let before = allocs();
                for &(s, n, around, separate, as_i32) in &calls {
                    let run = if separate {
                        Run::new(&list[s..s + n])
                    } else {
                        Run::new(&area[s * f..(s + n) * f])
                    };
                    let run = if around {
                        run.with_before(s.checked_sub(1).map(|p| list[p]))
                            .with_after(list.get(s + n).copied())
                    } else {
                        run
                    };
                    let out = if as_i32 {
                        PcmOut::I32(&mut i32s[..n * spf * ch])
                    } else {
                        PcmOut::S24le(&mut s24[..n * spf * ch * 3])
                    };
                    c.convert(run.at_frame(s as u64), out, Some(&mut fc[..n]))
                        .unwrap();
                    assert_eq!(
                        allocs(),
                        before,
                        "{rate:?} {cutoff:?} {filter:?} {ch} channels: \
                         allocated in the call ({s}, {n}, {around}, {separate}, {as_i32})"
                    );
                }
            }
        }
    }
    made
}

#[test]
fn no_allocation_precise() {
    assert!(case(Filter::Precise) > 0);
}

#[test]
fn no_allocation_fast() {
    assert!(case(Filter::Fast) > 0);
}

#[test]
fn no_allocation_pysacd() {
    assert!(case(Filter::Pysacd) > 0);
}
