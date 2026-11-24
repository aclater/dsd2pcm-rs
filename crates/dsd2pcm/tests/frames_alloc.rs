//! After the first call, `frames::Converter::convert` allocates nothing, for runs of any
//! length up to the first one's, with the default filter and each filter named. Run at every
//! SIMD level by `.github/scripts/test-simd-levels.sh` (the precise filter once grew a
//! buffer at the avx2fma level after a short run with neighbours).

use dsd2pcm::frames::{Converter, PcmConfig, PcmOut, Run, FRAME_BYTES_PER_CHANNEL};
use dsd2pcm::{Cutoff, Filter, PcmRate};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn no_allocation_after_warm_up() {
    for (rate, filter) in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800]
        .into_iter()
        .flat_map(|r| {
            [
                (r, None),
                (r, Some(Filter::Fast)),
                (r, Some(Filter::Precise)),
            ]
        })
    {
        let ch = 6;
        let f = FRAME_BYTES_PER_CHANNEL * ch;
        let area: Vec<u8> = (0..20 * f).map(|i| (i * 7 + i / 13) as u8).collect();
        let mut cfg = PcmConfig::new(rate, Cutoff::Standard, ch as u8);
        if let Some(f) = filter {
            cfg = cfg.with_filter(f);
        }
        let mut c = Converter::new(cfg).unwrap();
        let fb = c.samples_per_frame() * ch * 3;
        let mut out = vec![0u8; 18 * fb];
        let mut fc = vec![0u64; 18];
        let list: Vec<&[u8]> = area.chunks_exact(f).collect();
        // Warm-up: the longest block.
        c.convert(
            Run::new(&area[f..19 * f]),
            PcmOut::S24le(&mut out),
            Some(&mut fc),
        )
        .unwrap();
        let before = ALLOCS.load(Ordering::Relaxed);
        for (s, n) in [(1, 1), (3, 4), (1, 18), (0, 7)] {
            let run = Run::new(&list[s..s + n])
                .with_before(s.checked_sub(1).map(|p| list[p]))
                .with_after(Some(list[s + n]));
            c.convert(run, PcmOut::S24le(&mut out[..n * fb]), Some(&mut fc[..n]))
                .unwrap();
        }
        assert_eq!(
            ALLOCS.load(Ordering::Relaxed),
            before,
            "{rate:?} {filter:?}"
        );
    }
}
