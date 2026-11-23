//! Opening a file with many small chunks reads it a bounded number of times, through a
//! large `BufReader<File>` as a caller would build one, and too many chunks are refused.
//! The bytes the file delivers are counted, so the test does not depend on timing.

use dsd2pcm::dff::DffReader;
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::WavReader;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;

/// A file that counts the bytes read from it.
struct Counted {
    f: File,
    read: u64,
}

impl Read for Counted {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.f.read(b)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.f.seek(p)
    }
}

/// Writes `bytes` to a file, opens it through a 1 MiB `BufReader` with `open`, and returns
/// whether it opened and how many times over the file was read.
fn open_counted(
    name: &str,
    bytes: &[u8],
    open: impl FnOnce(&mut BufReader<Counted>) -> bool,
) -> (bool, f64) {
    let path: PathBuf =
        std::env::temp_dir().join(format!("dsd2pcm-walk-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let f = File::open(&path).unwrap();
    let mut r = BufReader::with_capacity(1 << 20, Counted { f, read: 0 });
    let ok = open(&mut r);
    let times = r.get_ref().read as f64 / bytes.len() as f64;
    std::fs::remove_file(&path).unwrap();
    (ok, times)
}

fn be_ck(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u64).to_be_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// A DFF with `top` empty chunks before `PROP` and `sub` empty property chunks in it.
fn dff(top: usize, sub: usize) -> Vec<u8> {
    let mut props = be_ck(b"FS  ", &2_822_400u32.to_be_bytes());
    props.extend(be_ck(b"CHNL", b"\x00\x02SLFTSRGT"));
    props.extend(be_ck(b"CMPR", b"DSD \x0enot compressed\x00"));
    for _ in 0..sub {
        props.extend(be_ck(b"XXXX", b""));
    }
    let mut body = b"DSD ".to_vec();
    body.extend(be_ck(b"FVER", &0x0105_0000u32.to_be_bytes()));
    for _ in 0..top {
        body.extend(be_ck(b"YYYY", b""));
    }
    body.extend(be_ck(b"PROP", &[b"SND ".as_slice(), &props].concat()));
    body.extend(be_ck(b"DSD ", &[0x69; 8]));
    be_ck(b"FRM8", &body)
}

/// A stereo 16-bit WAV with `n` empty chunks before `data` and `after` after it, with
/// `data` size `size` (that many bytes of samples follow it).
fn wav(n: usize, size: u32, after: usize) -> Vec<u8> {
    let mut b = b"WAVEfmt ".to_vec();
    b.extend(16u32.to_le_bytes());
    for v in [1u16, 2] {
        b.extend(v.to_le_bytes());
    }
    b.extend(44_100u32.to_le_bytes());
    b.extend(176_400u32.to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    for _ in 0..n {
        b.extend(b"JUNK\0\0\0\0");
    }
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    b.extend(vec![0u8; size as usize]);
    for _ in 0..after {
        b.extend(b"JUNK\0\0\0\0");
    }
    let mut v = b"RIFF".to_vec();
    v.extend((b.len() as u32).to_le_bytes());
    v.extend(b);
    v
}

/// The file is read at most a few times over (the readers seek to the end and back once).
const MAX_TIMES: f64 = 4.0;

#[test]
fn dff_chunk_walk_is_bounded() {
    for (top, sub) in [(4000, 0), (0, 4000), (2000, 2000)] {
        let (ok, times) = open_counted("a.dff", &dff(top, sub), |r| DffReader::new(r).is_ok());
        assert!(ok, "{top} {sub}");
        assert!(times <= MAX_TIMES, "{top} {sub}: read {times:.1} times");
    }
    for (top, sub) in [(5000, 0), (0, 5000), (100_000, 0), (0, 100_000)] {
        let (ok, times) = open_counted("b.dff", &dff(top, sub), |r| DffReader::new(r).is_ok());
        assert!(!ok, "{top} {sub}: too many chunks accepted");
        assert!(times <= MAX_TIMES, "{top} {sub}: read {times:.1} times");
    }
}

#[test]
fn wav_chunk_walk_is_bounded() {
    let (ok, times) = open_counted("a.wav", &wav(4000, 8, 0), |r| WavReader::new(r).is_ok());
    assert!(ok && times <= MAX_TIMES, "read {times:.1} times");
    // A placeholder data size: the chunks after it are walked too.
    let (ok, times) = open_counted("b.wav", &wav(10, 0, 4000), |r| WavReader::new(r).is_ok());
    assert!(!ok && times <= MAX_TIMES, "read {times:.1} times");
    for n in [5000, 100_000] {
        let (ok, times) = open_counted("c.wav", &wav(n, 8, 0), |r| WavReader::new(r).is_ok());
        assert!(!ok, "{n} chunks accepted");
        assert!(times <= MAX_TIMES, "{n}: read {times:.1} times");
    }
}

#[cfg(feature = "flac")]
#[test]
fn flac_metadata_walk_is_bounded() {
    use dsd2pcm::flac::FlacReader;
    // STREAMINFO, then empty PADDING blocks; no frames, so opening fails after the walk.
    let flac = |blocks: usize| {
        let mut v = b"fLaC".to_vec();
        v.extend([0, 0, 0, 34]);
        v.extend([
            0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0x0A, 0xC4, 0x42, 0xF0,
        ]);
        v.extend([0u8; 20]);
        for i in 0..blocks {
            v.extend([if i + 1 == blocks { 0x81 } else { 0x01 }, 0, 0, 0]);
        }
        v
    };
    for n in [4000, 5000, 100_000] {
        let (_, times) = open_counted("a.flac", &flac(n), |r| FlacReader::new(r).is_ok());
        assert!(times <= MAX_TIMES, "{n}: read {times:.1} times");
    }
}

#[test]
fn dsf_open_reads_the_file_a_bounded_number_of_times() {
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), 2, 2_822_400, None).unwrap();
    w.set_metadata(Some(b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec()));
    w.write_bytes(&[vec![0x69; 50_000], vec![0x69; 50_000]])
        .unwrap();
    w.finish(400_000).unwrap();
    let f = w.into_inner().into_inner();
    let (ok, times) = open_counted("a.dsf", &f, |r| DsfReader::new(r).is_ok());
    assert!(ok && times <= MAX_TIMES, "read {times:.1} times");
}
