//! FLAC reader: decoded samples against the generator formula and against WAV, the
//! unknown-length rule, detection, and malformed input. Test files: `tests/data/flac`
//! (regenerate with `make.sh` there).
#![cfg(feature = "flac")]

use dsd2pcm::flac::FlacReader;
use dsd2pcm::io::PcmRead;
use dsd2pcm::wav::{WavFormat, WavReader, WavWriter};
use dsd2pcm::{open_input, Error, Input};
use std::io::Cursor;

/// The sample formula of `tests/data/flac/gen.py`.
fn sample(i: u64, c: u64, bits: u32) -> i64 {
    let full = 1i64 << (bits - 1);
    let period = 64 + 16 * c as i64;
    let mut v = (i as i64 % period) * 2 * full / period - full;
    v += (((i * 1_103_515_245 + c * 12_345) >> 7) % 16) as i64 - 8;
    if i == 0 {
        v = -full;
    }
    if i == 1 {
        v = full - 1;
    }
    v.clamp(-full, full - 1)
}

/// A test file, embedded so the tests need no file system (and run under Miri).
fn fixture(name: &str) -> Vec<u8> {
    let bytes: &[u8] = match name {
        "s8_1ch_8000.flac" => include_bytes!("data/flac/s8_1ch_8000.flac"),
        "s12_1ch_22050.flac" => include_bytes!("data/flac/s12_1ch_22050.flac"),
        "s16_2ch_44100.flac" => include_bytes!("data/flac/s16_2ch_44100.flac"),
        "s20_2ch_192000.flac" => include_bytes!("data/flac/s20_2ch_192000.flac"),
        "s24_6ch_96000.flac" => include_bytes!("data/flac/s24_6ch_96000.flac"),
        "s32_1ch_48000.flac" => include_bytes!("data/flac/s32_1ch_48000.flac"),
        _ => panic!("no test file {name}"),
    };
    bytes.to_vec()
}

/// (file, bits, channels, rate, frames)
const FILES: [(&str, u32, u16, u32, u64); 5] = [
    ("s16_2ch_44100.flac", 16, 2, 44_100, 4100),
    ("s24_6ch_96000.flac", 24, 6, 96_000, 1500),
    ("s8_1ch_8000.flac", 8, 1, 8_000, 1200),
    ("s12_1ch_22050.flac", 12, 1, 22_050, 1200),
    ("s20_2ch_192000.flac", 20, 2, 192_000, 1200),
];

fn read_all(r: &mut dyn PcmRead, max: usize) -> dsd2pcm::Result<Vec<Vec<f64>>> {
    let mut out = vec![Vec::new(); usize::from(r.info().channels)];
    while r.read_frames(max, &mut out)? > 0 {}
    Ok(out)
}

fn expected(bits: u32, ch: u16, frames: u64) -> Vec<Vec<f64>> {
    let scale = f64::from(1u32 << (bits - 1));
    (0..u64::from(ch))
        .map(|c| {
            (0..frames)
                .map(|i| sample(i, c, bits) as f64 / scale)
                .collect()
        })
        .collect()
}

#[test]
fn decodes_every_bit_depth_exactly() {
    for (name, bits, ch, rate, frames) in FILES {
        let f = fixture(name);
        let want = expected(bits, ch, frames);
        for max in [1, 1000, 1152, 1153, 1 << 20, usize::MAX] {
            let mut r = FlacReader::new(Cursor::new(f.clone())).unwrap();
            let i = r.info();
            assert_eq!(
                (i.channels, i.sample_rate, i.frames, r.bits_per_sample()),
                (ch, rate, frames, bits),
                "{name}"
            );
            let got = read_all(&mut r, max).unwrap();
            assert_eq!(got, want, "{name} max {max}");
            assert_eq!(
                r.read_frames(10, &mut vec![Vec::new(); ch.into()]).unwrap(),
                0
            );
        }
    }
}

/// A FLAC and a WAV of the same 16 or 24-bit samples read identically.
#[test]
fn matches_wav_of_the_same_samples() {
    for (name, bits, ch, rate, frames) in FILES {
        let fmt = match bits {
            16 => WavFormat::I16,
            24 => WavFormat::I24,
            _ => continue,
        };
        let mut w = WavWriter::new(Cursor::new(Vec::new()), fmt, ch, rate).unwrap();
        let mut raw = Vec::new();
        for i in 0..frames {
            for c in 0..u64::from(ch) {
                let v = sample(i, c, bits) as i32;
                raw.extend_from_slice(&v.to_le_bytes()[..fmt.bytes()]);
            }
        }
        w.write_raw(&raw).unwrap();
        w.finish().unwrap();
        let mut wav = WavReader::new(Cursor::new(w.into_inner().into_inner())).unwrap();
        let mut flac = FlacReader::new(Cursor::new(fixture(name))).unwrap();
        assert_eq!(wav.info(), flac.info(), "{name}");
        let (a, b) = (
            read_all(&mut wav, 777).unwrap(),
            read_all(&mut flac, 777).unwrap(),
        );
        let bits_of = |v: &Vec<Vec<f64>>| -> Vec<Vec<u64>> {
            v.iter()
                .map(|c| c.iter().map(|x| x.to_bits()).collect())
                .collect()
        };
        assert_eq!(bits_of(&a), bits_of(&b), "{name}");
    }
}

/// STREAMINFO starts at byte 8 of a file without ID3 tag.
const SI: usize = 8;

fn set_total(f: &mut [u8], total: u64) {
    f[SI + 13] = (f[SI + 13] & 0xF0) | ((total >> 32) as u8 & 0x0F);
    f[SI + 14..SI + 18].copy_from_slice(&(total as u32).to_be_bytes());
}

#[test]
fn unknown_length_is_counted() {
    let mut f = fixture("s16_2ch_44100.flac");
    set_total(&mut f, 0);
    let mut r = FlacReader::new(Cursor::new(f)).unwrap();
    assert_eq!(r.info().frames, 4100);
    assert_eq!(read_all(&mut r, 1000).unwrap(), expected(16, 2, 4100));
}

/// Where the first audio frame starts (after the last metadata block).
fn frames_at(f: &[u8]) -> usize {
    let mut p = 4;
    loop {
        let len = u32::from_be_bytes([0, f[p + 1], f[p + 2], f[p + 3]]) as usize;
        let last = f[p] & 0x80 != 0;
        p += 4 + len;
        if last {
            return p;
        }
    }
}

#[test]
fn unknown_length_without_frames_is_an_error() {
    let mut f = fixture("s16_2ch_44100.flac");
    set_total(&mut f, 0);
    f.truncate(frames_at(&f));
    let e = FlacReader::new(Cursor::new(f)).unwrap_err();
    assert!(
        matches!(e, Error::Format(ref m) if m.contains("no audio frames")),
        "{e}"
    );
}

#[test]
fn short_stream_is_an_error() {
    let f = fixture("s16_2ch_44100.flac");
    // Cut at a frame boundary: the reader sees a clean end after 1152 frames.
    let mut r = FlacReader::new(Cursor::new(f.clone())).unwrap();
    let mut out = vec![Vec::new(), Vec::new()];
    assert_eq!(r.read_frames(1152, &mut out).unwrap(), 1152);
    // Declared longer than the stream.
    let mut g = f.clone();
    set_total(&mut g, 5000);
    let mut r = FlacReader::new(Cursor::new(g)).unwrap();
    let e = read_all(&mut r, 1 << 20).unwrap_err();
    assert!(
        matches!(e, Error::Format(ref m) if m.contains("ended after 4100 of 5000")),
        "{e}"
    );
    // Declared shorter: the rest is ignored.
    let mut g = f.clone();
    set_total(&mut g, 2000);
    let mut r = FlacReader::new(Cursor::new(g)).unwrap();
    let got = read_all(&mut r, 999).unwrap();
    assert_eq!(got[0], expected(16, 2, 2000)[0]);
    // Truncated inside a frame.
    let mut r = FlacReader::new(Cursor::new(f[..f.len() - 100].to_vec())).unwrap();
    assert!(matches!(read_all(&mut r, 1 << 20), Err(Error::Format(_))));
}

#[test]
fn rejects_unsupported_streams() {
    let e = FlacReader::new(Cursor::new(fixture("s32_1ch_48000.flac"))).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "{e}");
    // 7 channels.
    let mut f = fixture("s16_2ch_44100.flac");
    f[SI + 12] = (f[SI + 12] & 0xF1) | (6 << 1);
    let e = FlacReader::new(Cursor::new(f)).unwrap_err();
    assert!(
        matches!(e, Error::Format(ref m) if m.contains("channel count 7")),
        "{e}"
    );
    // A frame with a channel count other than STREAMINFO's.
    let mut f = fixture("s16_2ch_44100.flac");
    f[SI + 12] &= 0xF1;
    let mut r = FlacReader::new(Cursor::new(f)).unwrap();
    assert_eq!(r.info().channels, 1);
    assert!(matches!(read_all(&mut r, 100), Err(Error::Format(_))));
    // Samples beyond STREAMINFO's bit depth (24-bit frames, 16-bit STREAMINFO).
    let mut f = fixture("s24_6ch_96000.flac");
    // Bits per sample - 1 = 15: the top bit (0) ends byte 12, the rest opens byte 13.
    f[SI + 12] &= 0xFE;
    f[SI + 13] = (f[SI + 13] & 0x0F) | (15 << 4);
    let mut r = FlacReader::new(Cursor::new(f)).unwrap();
    assert_eq!(r.bits_per_sample(), 16);
    let e = read_all(&mut r, 100).unwrap_err();
    assert!(
        matches!(e, Error::Format(ref m) if m.contains("bit depth")),
        "{e}"
    );
}

fn id3(body: usize, footer: bool) -> Vec<u8> {
    let mut t = b"ID3\x04\x00".to_vec();
    t.push(if footer { 0x10 } else { 0 });
    t.extend((0..4).rev().map(|k| ((body >> (7 * k)) & 0x7F) as u8));
    t.extend(std::iter::repeat_n(0xAAu8, body));
    if footer {
        t.extend_from_slice(b"3DI\x04\x00\x10\0\0\0\0");
    }
    t
}

#[test]
fn skips_leading_id3_tags() {
    let f = fixture("s20_2ch_192000.flac");
    let mut g = id3(300, false);
    g.extend(id3(1000, true));
    g.extend_from_slice(&f);
    let mut r = FlacReader::new(Cursor::new(g)).unwrap();
    assert_eq!(read_all(&mut r, 500).unwrap(), expected(20, 2, 1200));
    // Size bytes must be syncsafe; a tag past the end of the file is an error.
    let mut g = id3(10, false);
    g[6] = 0x80;
    g.extend_from_slice(&f);
    assert!(matches!(
        FlacReader::new(Cursor::new(g)),
        Err(Error::Format(_))
    ));
    let mut g = id3(10, false);
    g[9] = 0x7F;
    g[8] = 0x7F;
    g.extend_from_slice(&f[..100]);
    assert!(matches!(
        FlacReader::new(Cursor::new(g)),
        Err(Error::Format(_))
    ));
    // An ID3 tag not followed by FLAC.
    let mut g = id3(10, false);
    g.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    assert!(matches!(
        FlacReader::new(Cursor::new(g)),
        Err(Error::Format(_))
    ));
}

#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn open_input_detects_flac() {
    let dir = env!("CARGO_TARGET_TMPDIR");
    let f = fixture("s24_6ch_96000.flac");
    let cases: [(&str, Vec<u8>); 2] = [
        ("plain", f.clone()),
        ("id3", {
            let mut g = id3(64, false);
            g.extend_from_slice(&f);
            g
        }),
    ];
    for (tag, bytes) in cases {
        let p = std::path::Path::new(dir).join(format!("flac-detect-{tag}.flac"));
        std::fs::write(&p, bytes).unwrap();
        let Input::Pcm(mut r) = open_input(&p).unwrap() else {
            panic!("{tag}: not PCM")
        };
        assert_eq!(read_all(r.as_mut(), 4096).unwrap(), expected(24, 6, 1500));
        std::fs::remove_file(&p).unwrap();
    }
    let p = std::path::Path::new(dir).join("flac-detect-junk.bin");
    std::fs::write(&p, b"OggS0000").unwrap();
    let e = open_input(&p).unwrap_err();
    assert!(e.to_string().contains("DSF, DFF, WAV or FLAC"), "{e}");
    std::fs::remove_file(&p).unwrap();
}

/// Every truncation and many single-byte corruptions give a result or an error, never a
/// panic, with known and unknown length.
#[test]
#[cfg_attr(
    miri,
    ignore = "too slow under Miri: thousands of corrupted files; the fuzz targets cover it"
)]
fn malformed_inputs_do_not_panic() {
    let mut files = Vec::new();
    for (name, ..) in FILES {
        let f = fixture(name);
        let mut g = f.clone();
        set_total(&mut g, 0);
        files.push(f);
        files.push(g);
    }
    let try_one = |b: Vec<u8>| {
        if let Ok(mut r) = FlacReader::new(Cursor::new(b)) {
            let mut o = vec![Vec::new(); usize::from(r.info().channels)];
            while let Ok(n) = r.read_frames(4096, &mut o) {
                if n == 0 {
                    break;
                }
            }
        }
    };
    for f in &files {
        let audio = frames_at(f);
        let cuts = (0..(audio + 64).min(f.len())).chain((audio..f.len()).step_by(97));
        for cut in cuts {
            try_one(f[..cut].to_vec());
        }
        let positions = (0..(audio + 64).min(f.len())).chain((audio..f.len()).step_by(61));
        for pos in positions {
            for v in [0x00u8, 0xFF, 0x7F, 0x80, 0x01] {
                let mut b = f.clone();
                b[pos] = v;
                try_one(b);
            }
        }
    }
}

/// A reader that claims to have read 100 bytes more than the buffer holds.
struct Liar(Cursor<Vec<u8>>);

impl std::io::Read for Liar {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.0.read(buf)?;
        Ok(if n > 0 { buf.len() + 100 } else { 0 })
    }
}

impl std::io::Seek for Liar {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

/// An over-reporting `Read` gives an error or wrong samples, never out-of-bounds access
/// (claxon trusts the count). Run under Miri too:
/// `cargo +nightly miri test -p dsd2pcm --test flac lying_reader`.
#[test]
fn lying_reader_is_safe() {
    static F: &[u8] = include_bytes!("data/flac/s8_1ch_8000.flac");
    let mut unknown = F.to_vec();
    set_total(&mut unknown, 0);
    for bytes in [F.to_vec(), unknown] {
        if let Ok(mut r) = FlacReader::new(Liar(Cursor::new(bytes))) {
            let mut o = vec![Vec::new(); usize::from(r.info().channels)];
            while let Ok(n) = r.read_frames(500, &mut o) {
                if n == 0 {
                    break;
                }
            }
        }
    }
}

/// With an unknown length, a trailing ID3v1 tag is not taken for a frame.
#[test]
fn trailing_id3v1_tag() {
    let mut tag = b"TAG".to_vec();
    tag.resize(128, b'x');
    for unknown in [false, true] {
        let mut f = fixture("s16_2ch_44100.flac");
        if unknown {
            set_total(&mut f, 0);
        }
        f.extend_from_slice(&tag);
        let mut r = FlacReader::new(Cursor::new(f)).unwrap();
        assert_eq!(r.info().frames, 4100);
        assert_eq!(read_all(&mut r, 3000).unwrap(), expected(16, 2, 4100));
    }
    // Other trailing junk with an unknown length is still an error.
    let mut f = fixture("s16_2ch_44100.flac");
    set_total(&mut f, 0);
    f.extend_from_slice(&[0x55; 128]);
    assert!(matches!(
        FlacReader::new(Cursor::new(f)),
        Err(Error::Format(_))
    ));
}

/// `f` with its Vorbis comment blocks turned into padding.
fn without_comments(f: &[u8]) -> Vec<u8> {
    let mut f = f.to_vec();
    let mut at = 4;
    loop {
        let (last, kind) = (f[at] & 0x80, f[at] & 0x7F);
        let len = u32::from_be_bytes([0, f[at + 1], f[at + 2], f[at + 3]]) as usize;
        if kind == 4 {
            f[at] = last | 1;
        }
        if last != 0 {
            return f;
        }
        at += 4 + len;
    }
}

/// `f` with a Vorbis comment block of `comments` inserted after STREAMINFO.
fn with_comments(f: &[u8], comments: &[&[u8]], count: Option<u32>) -> Vec<u8> {
    let mut body = Vec::new();
    let vendor = b"reference libFLAC 1.4.3 20230623";
    body.extend((vendor.len() as u32).to_le_bytes());
    body.extend(vendor);
    body.extend(count.unwrap_or(comments.len() as u32).to_le_bytes());
    for c in comments {
        body.extend((c.len() as u32).to_le_bytes());
        body.extend(*c);
    }
    // STREAMINFO's header is at 4; the new block takes its "last" flag.
    let last = f[4] & 0x80;
    let mut out = f[..42].to_vec();
    out[4] &= 0x7F;
    out.push(last | 4);
    out.extend(&(body.len() as u32).to_be_bytes()[1..]);
    out.extend(body);
    out.extend(&f[42..]);
    out
}

/// The `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` Vorbis comment, as the reference encoder writes
/// it, gives the channel mask; a malformed comment block is ignored, as before.
#[test]
fn channel_mask_from_vorbis_comment() {
    let f = fixture("s16_2ch_44100.flac");
    let mask = |f: Vec<u8>| -> Option<u32> {
        let r = FlacReader::new(Cursor::new(f)).unwrap();
        let m = r.channel_mask();
        assert_eq!(PcmRead::channel_mask(&r), m);
        m
    };
    // The reference encoder's own comment (make.sh: from a WAVE_FORMAT_EXTENSIBLE input).
    assert_eq!(mask(f.clone()), Some(0x03));
    let six = FlacReader::new(Cursor::new(fixture("s24_6ch_96000.flac"))).unwrap();
    assert_eq!(six.channel_mask(), Some(0x3F));
    let mono = FlacReader::new(Cursor::new(fixture("s8_1ch_8000.flac"))).unwrap();
    assert_eq!(mono.channel_mask(), Some(0x01));
    // Without it: none.
    let f = without_comments(&f);
    assert_eq!(mask(f.clone()), None);
    let long = [b'x'; 200];
    for (comments, want) in [
        (
            &[&b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0003"[..]][..],
            Some(3),
        ),
        (
            &[
                &b"TITLE=t"[..],
                &long[..],
                &b"waveformatextensible_channel_mask=0X30"[..],
            ][..],
            Some(0x30),
        ),
        (
            &[&b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=51"[..]][..],
            Some(51),
        ),
        (&[&b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0xZZ"[..]][..], None),
        (&[&b"TITLE=no mask"[..]][..], None),
        (&[][..], None),
    ] {
        assert_eq!(
            mask(with_comments(&f, comments, None)),
            want,
            "{comments:?}"
        );
    }
    // The samples are unaffected by the comment block.
    let c = &[&b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x3"[..]][..];
    let mut r = FlacReader::new(Cursor::new(with_comments(&f, c, None))).unwrap();
    assert_eq!(read_all(&mut r, 1000).unwrap(), expected(16, 2, 4100));
    // A count past the block, a comment length past the block: ignored.
    assert_eq!(mask(with_comments(&f, c, Some(u32::MAX))), Some(3));
    assert_eq!(mask(with_comments(&f, &[][..], Some(u32::MAX))), None);
    let mut bad = with_comments(&f, c, None);
    let at = 42 + 4 + 4 + 32 + 4;
    bad[at..at + 4].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(mask(bad), None);
}
