//! FlacReader on arbitrary bytes: open, then read every frame. Must never panic, hang or
//! allocate without bound.
#![no_main]

use dsd2pcm::flac::FlacReader;
use dsd2pcm::io::PcmRead;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let Ok(mut r) = FlacReader::new(Cursor::new(data)) else {
        return;
    };
    let ch = usize::from(r.info().channels);
    let mut out = vec![Vec::new(); ch];
    // Small reads cross block boundaries; clear so memory stays bounded.
    while let Ok(n) = r.read_frames(1000, &mut out) {
        if n == 0 {
            break;
        }
        for o in &mut out {
            o.clear();
        }
    }
});
