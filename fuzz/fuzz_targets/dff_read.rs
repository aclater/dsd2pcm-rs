//! Parses arbitrary bytes as DSDIFF and reads all audio with small and odd request sizes.
#![no_main]

use dsd2pcm::dff::DffReader;
use dsd2pcm::io::DsdRead;
use dsd2pcm_fuzz::{mem_check, mem_limit, mem_start, read_sizes};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let base = mem_start();
    if let Ok(mut r) = DffReader::new(Cursor::new(data)) {
        let info = r.info();
        let ch = usize::from(info.channels);
        assert!((1..=6).contains(&ch));
        let want = info.samples.div_ceil(8);
        assert!(want * ch as u64 <= data.len() as u64);
        let mut out = vec![Vec::new(); ch];
        let mut total = 0u64;
        for max in read_sizes(data) {
            match r.read_bytes(max, &mut out) {
                Ok(0) => {
                    assert_eq!(total, want, "DFF delivered a different byte count");
                    break;
                }
                Ok(n) => {
                    assert!(out.iter().all(|o| o.len() == n), "unequal channel counts");
                    total += n as u64;
                    assert!(total <= want, "DFF delivered more than the sample count");
                    out.iter_mut().for_each(Vec::clear);
                }
                Err(e) => panic!("read failed after a successful open: {e}"),
            }
        }
    }
    mem_check(base, mem_limit(data.len()), "DFF");
});
