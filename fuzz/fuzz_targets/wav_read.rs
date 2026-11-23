//! Parses arbitrary bytes as WAV and reads all samples with small and odd request sizes.
#![no_main]

use dsd2pcm::io::PcmRead;
use dsd2pcm::wav::WavReader;
use dsd2pcm_fuzz::{mem_check, mem_limit, mem_start, read_sizes};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let base = mem_start();
    if let Ok(mut r) = WavReader::new(Cursor::new(data)) {
        let info = r.info();
        let ch = usize::from(info.channels);
        assert!((1..=6).contains(&ch));
        assert!(info.sample_rate > 0);
        let frame = (ch * r.format().bytes()) as u64;
        assert!(info.frames * frame <= data.len() as u64);
        let mut out = vec![Vec::new(); ch];
        let mut total = 0u64;
        for max in read_sizes(data) {
            match r.read_frames(max, &mut out) {
                Ok(0) => {
                    assert_eq!(total, info.frames, "WAV delivered a different frame count");
                    break;
                }
                Ok(n) => {
                    assert!(out.iter().all(|o| o.len() == n), "unequal channel counts");
                    for s in out.iter().flatten() {
                        assert!(s.is_finite(), "non-finite sample");
                    }
                    total += n as u64;
                    assert!(
                        total <= info.frames,
                        "WAV delivered more than the frame count"
                    );
                    out.iter_mut().for_each(Vec::clear);
                }
                Err(e) => panic!("read failed after a successful open: {e}"),
            }
        }
    }
    mem_check(base, mem_limit(data.len()), "WAV");
});
