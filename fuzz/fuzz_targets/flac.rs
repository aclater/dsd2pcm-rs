//! Parses arbitrary bytes as FLAC and reads all samples with small and odd request sizes.
#![no_main]

use dsd2pcm::flac::{FlacReader, MAX_CHUNK};
use dsd2pcm::io::PcmRead;
use dsd2pcm_fuzz::{mem_check, mem_limit, mem_start, read_sizes};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let base = mem_start();
    // FLAC expands its input, so memory is bounded by the reader's limits instead: one
    // decoded block (at most 65536 frames of 8 channels of i32) and at most MAX_CHUNK
    // frames per call in the output vectors (8 bytes each, doubled by vector growth).
    let mut limit = mem_limit(data.len()) + (4 << 20);
    if let Ok(mut r) = FlacReader::new(Cursor::new(data)) {
        let info = r.info();
        let ch = usize::from(info.channels);
        assert!((1..=6).contains(&ch));
        assert!(info.sample_rate > 0);
        assert!(info.frames > 0);
        limit += ch * 16 * (info.frames.min(MAX_CHUNK as u64) as usize);
        let mut out = vec![Vec::new(); ch];
        let mut total = 0u64;
        for max in read_sizes(data) {
            match r.read_frames(max, &mut out) {
                Ok(0) => {
                    assert_eq!(total, info.frames, "FLAC delivered a different frame count");
                    break;
                }
                Ok(n) => {
                    assert!(out.iter().all(|o| o.len() == n), "unequal channel counts");
                    for s in out.iter().flatten() {
                        assert!((-1.0..1.0).contains(s), "sample outside full scale");
                    }
                    total += n as u64;
                    assert!(
                        total <= info.frames,
                        "FLAC delivered more than the frame count"
                    );
                    out.iter_mut().for_each(Vec::clear);
                }
                // Frames are decoded as they are read, so damage after the headers is
                // reported here.
                Err(_) => break,
            }
        }
    }
    mem_check(base, limit, "FLAC");
});
