//! Opens arbitrary bytes with every reader through a 1 MiB `BufReader`, as callers wrap a
//! file, and checks that opening reads the input a bounded number of times over: a reader
//! that seeks once per chunk would refill the whole buffer for every chunk.
#![no_main]

use dsd2pcm::dff::DffReader;
use dsd2pcm::dsf::DsfReader;
use dsd2pcm::wav::WavReader;
use libfuzzer_sys::fuzz_target;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};

/// A file-like source that counts the bytes read from it.
struct Counted<'a> {
    c: Cursor<&'a [u8]>,
    read: usize,
}

impl Read for Counted<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.c.read(b)?;
        self.read += n;
        Ok(n)
    }
}

impl Seek for Counted<'_> {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.c.seek(p)
    }
}

fn check<T, E>(
    data: &[u8],
    what: &str,
    open: impl FnOnce(&mut BufReader<Counted>) -> Result<T, E>,
) {
    let mut r = BufReader::with_capacity(
        1 << 20,
        Counted {
            c: Cursor::new(data),
            read: 0,
        },
    );
    let _ = open(&mut r).map(drop);
    let read = r.get_ref().read;
    // A handful of whole passes: the readers seek to the end and back, and a placeholder
    // WAV data size or an unknown FLAC length walks the file once more.
    assert!(
        read <= 8 * data.len() + 4096,
        "{what}: read {read} bytes to open {} bytes",
        data.len()
    );
}

fuzz_target!(|data: &[u8]| {
    check(data, "DSF", |r| DsfReader::new(r).map(|_| ()));
    check(data, "DFF", |r| DffReader::new(r).map(|_| ()));
    check(data, "WAV", |r| WavReader::new(r).map(|_| ()));
    check(data, "FLAC", |r| {
        dsd2pcm::flac::FlacReader::new(r).map(|_| ())
    });
});
