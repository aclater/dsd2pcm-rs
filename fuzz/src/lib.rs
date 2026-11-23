//! Shared helpers for the dsd2pcm fuzz targets.
//!
//! A counting global allocator checks that a reader's memory use stays proportional to its
//! input, and the read-size sequence drives the readers with small, odd and oversized
//! requests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let now = CURRENT.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            let now = CURRENT.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CURRENT.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new >= l.size() {
                let now = CURRENT.fetch_add(new - l.size(), Ordering::Relaxed) + new - l.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Starts measuring heap growth; returns the baseline.
pub fn mem_start() -> usize {
    let now = CURRENT.load(Ordering::Relaxed);
    PEAK.store(now, Ordering::Relaxed);
    now
}

/// Panics if the heap grew by more than `limit` bytes since [`mem_start`].
pub fn mem_check(base: usize, limit: usize, what: &str) {
    let grew = PEAK.load(Ordering::Relaxed).saturating_sub(base);
    assert!(
        grew <= limit,
        "{what}: heap grew by {grew} bytes, limit {limit}"
    );
}

/// Heap budget for reading an input of `len` bytes: output vectors hold at most 8 bytes
/// (one `f64`) per input byte, doubled by vector growth, plus the reader's buffer and a
/// fixed allowance.
pub fn mem_limit(len: usize) -> usize {
    32 * len + (1 << 20)
}

const SIZES: [usize; 16] = [
    1,
    3,
    0,
    7,
    2,
    13,
    4095,
    5,
    4097,
    11,
    usize::MAX,
    17,
    65_537,
    1,
    (16 << 20) + 1,
    9,
];

/// Request sizes for successive read calls: small, odd, zero and oversized, in an order
/// that depends on the input so different inputs exercise different sequences.
pub fn read_sizes(data: &[u8]) -> impl Iterator<Item = usize> {
    let seed = data.iter().fold(0usize, |a, &b| {
        a.wrapping_mul(31).wrapping_add(usize::from(b))
    });
    (0..).map(move |i: usize| SIZES[(seed.wrapping_add(i)) % SIZES.len()])
}
