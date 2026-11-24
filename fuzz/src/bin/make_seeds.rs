//! Writes small valid DSF, DFF and WAV files, made with the crate's own writers, and the
//! FLAC test files (with variants) into `corpus/<target>/` as seed inputs. Run from the `fuzz` directory:
//! `cargo run --release --bin make_seeds`.

use dsd2pcm::dff::DffWriter;
use dsd2pcm::dsf::DsfWriter;
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::{WavFormat, WavWriter};
use std::fs;
use std::io::Cursor;
use std::path::Path;

fn pattern(ch: usize, n: usize) -> Vec<Vec<u8>> {
    (0..ch)
        .map(|c| (0..n).map(|i| (i * 37 + c * 11) as u8).collect())
        .collect()
}

fn dsf(ch: usize, n: usize, rate: u32, trim: u64, meta: Option<&[u8]>) -> Vec<u8> {
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch as u16, rate, None).unwrap();
    w.set_metadata(meta.map(<[u8]>::to_vec));
    w.write_bytes(&pattern(ch, n)).unwrap();
    w.finish(n as u64 * 8 - trim).unwrap();
    w.into_inner().into_inner()
}

fn dff(ch: usize, n: usize, rate: u32) -> Vec<u8> {
    let mut w = DffWriter::new(Cursor::new(Vec::new()), ch as u16, rate).unwrap();
    w.write_bytes(&pattern(ch, n)).unwrap();
    w.finish(n as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

/// A DFF with an odd-sized comment chunk before `PROP` and an odd-sized unknown property.
fn dff_extra_chunks() -> Vec<u8> {
    let plain = dff(2, 9, 2_822_400);
    let prop_at = plain.windows(4).position(|w| w == b"PROP").unwrap();
    let mut out = plain[..prop_at].to_vec();
    out.extend_from_slice(b"COMT");
    out.extend_from_slice(&3u64.to_be_bytes());
    out.extend_from_slice(b"abc\0");
    // PROP with an extra 'ABSS'-like odd property in front of the others.
    let size = u64::from_be_bytes(plain[prop_at + 4..prop_at + 12].try_into().unwrap());
    out.extend_from_slice(b"PROP");
    out.extend_from_slice(&(size + 14).to_be_bytes());
    out.extend_from_slice(b"SND ");
    out.extend_from_slice(b"XTRA");
    out.extend_from_slice(&1u64.to_be_bytes());
    out.extend_from_slice(&[7, 0]);
    out.extend_from_slice(&plain[prop_at + 16..]);
    let form = out.len() as u64 - 12;
    out[4..12].copy_from_slice(&form.to_be_bytes());
    out
}

fn wav(format: WavFormat, ch: u16, rate: u32, frames: usize) -> Vec<u8> {
    let mut w = WavWriter::new(Cursor::new(Vec::new()), format, ch, rate).unwrap();
    let raw: Vec<u8> = (0..frames * usize::from(ch) * format.bytes())
        .map(|i| (i * 53) as u8)
        .collect();
    w.write_raw(&raw).unwrap();
    w.finish().unwrap();
    w.into_inner().into_inner()
}

/// A 24-bit stereo WAV with a `WAVE_FORMAT_EXTENSIBLE` header (`valid` valid bits) and an
/// odd-sized `LIST` chunk before the data.
fn wav_extensible(valid: u16) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&88_200u32.to_le_bytes());
    fmt.extend_from_slice(&(88_200u32 * 6).to_le_bytes());
    fmt.extend_from_slice(&6u16.to_le_bytes());
    fmt.extend_from_slice(&24u16.to_le_bytes());
    fmt.extend_from_slice(&22u16.to_le_bytes());
    fmt.extend_from_slice(&valid.to_le_bytes());
    fmt.extend_from_slice(&3u32.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ]);
    let data: Vec<u8> = (0..60u8).collect();
    let mut body = b"WAVE".to_vec();
    for (id, b) in [
        (b"fmt ", &fmt[..]),
        (b"LIST", b"INFOx"),
        (b"data", &data[..]),
    ] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(b.len() as u32).to_le_bytes());
        body.extend_from_slice(b);
        if b.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// `wav` with the `data` and RIFF sizes set to `size`, as streaming writers and some
/// encoders past 4 GiB leave them, optionally followed by a `LIST` chunk.
fn wav_size(size: u32, list_after: bool) -> Vec<u8> {
    let mut b = wav(WavFormat::I16, 2, 44_100, if list_after { 0 } else { 11 });
    b[4..8].copy_from_slice(&size.to_le_bytes());
    b[40..44].copy_from_slice(&size.to_le_bytes());
    if list_after {
        b.extend_from_slice(b"LIST\x05\0\0\0INFOx\0");
    }
    b
}

/// A DFF whose `DSD ` chunk ends in a partial turn (two bytes of three channels).
fn dff_partial_turn() -> Vec<u8> {
    let mut b = dff(3, 10, 2_822_400);
    let at = b.windows(4).rposition(|w| w == b"DSD ").unwrap();
    b[at + 4..at + 12].copy_from_slice(&32u64.to_be_bytes());
    b.extend_from_slice(&[0xAA, 0xBB]);
    let form = b.len() as u64 - 12;
    b[4..12].copy_from_slice(&form.to_be_bytes());
    b
}

/// The FLAC test files, each also with an unknown length, a leading ID3v2 tag, a
/// trailing ID3v1 tag and a total shorter than the stream.
fn flac_seeds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/dsd2pcm/tests/data/flac");
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".flac"))
        .collect();
    names.sort();
    for name in names {
        let f = fs::read(dir.join(&name)).unwrap();
        let stem = name.trim_end_matches(".flac");
        // STREAMINFO's 36-bit total sample count: low nibble of byte 21, bytes 22..26.
        let mut unknown = f.clone();
        unknown[21] &= 0xF0;
        unknown[22..26].fill(0);
        let mut id3 = b"ID3\x04\0\0\0\0\0\x10".to_vec();
        id3.extend_from_slice(&[0; 16]);
        id3.extend_from_slice(&f);
        let mut tagged = unknown.clone();
        tagged.extend_from_slice(b"TAG");
        tagged.resize(tagged.len() + 125, b' ');
        put("flac", stem, &f);
        put("flac", &format!("{stem}-unknown-length"), &unknown);
        put("flac", &format!("{stem}-id3v2"), &id3);
        put("flac", &format!("{stem}-unknown-id3v1"), &tagged);
        let mut short = f.clone();
        short[21] &= 0xF0;
        short[22..26].copy_from_slice(&1000u32.to_be_bytes());
        put("flac", &format!("{stem}-total-1000"), &short);
    }
}

fn put(dir: &str, name: &str, bytes: &[u8]) {
    let d = Path::new("corpus").join(dir);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(name), bytes).unwrap();
}

fn main() {
    put("dsf_read", "mono-dsd64", &dsf(1, 40, 2_822_400, 0, None));
    put(
        "dsf_read",
        "stereo-dsd128-trim",
        &dsf(2, 33, 5_644_800, 5, None),
    );
    put(
        "dsf_read",
        "stereo-id3",
        &dsf(2, 20, 2_822_400, 0, Some(b"ID3\x04\0\0\0\0\0\x01x")),
    );
    put("dsf_read", "6ch-dsd64-48k", &dsf(6, 3, 3_072_000, 7, None));
    put("dsf_read", "two-blocks", &dsf(1, 4100, 2_822_400, 0, None));
    put("dff_read", "mono", &dff(1, 31, 2_822_400));
    put("dff_read", "stereo-dsd256", &dff(2, 40, 11_289_600));
    put("dff_read", "5ch-odd", &dff(5, 7, 2_822_400));
    put("dff_read", "6ch", &dff(6, 4, 6_144_000));
    put("dff_read", "extra-chunks", &dff_extra_chunks());
    put("dff_read", "partial-turn", &dff_partial_turn());
    put("wav_read", "i16-mono", &wav(WavFormat::I16, 1, 44_100, 9));
    put("wav_read", "i24-stereo", &wav(WavFormat::I24, 2, 88_200, 7));
    put("wav_read", "i32-3ch", &wav(WavFormat::I32, 3, 176_400, 5));
    put(
        "wav_read",
        "f32-stereo",
        &wav(WavFormat::F32, 2, 352_800, 6),
    );
    put("wav_read", "f64-6ch", &wav(WavFormat::F64, 6, 96_000, 3));
    put("wav_read", "extensible-list", &wav_extensible(24));
    put("wav_read", "extensible-valid-20", &wav_extensible(20));
    let mut cut = wav(WavFormat::I24, 2, 88_200, 7);
    cut.truncate(cut.len() - 8);
    put("wav_read", "data-past-the-end", &cut);
    put("wav_read", "data-size-0", &wav_size(0, false));
    put("wav_read", "data-size-ffffffff", &wav_size(u32::MAX, false));
    put("wav_read", "data-size-0-then-list", &wav_size(0, true));
    flac_seeds();
    println!("seed corpora written under corpus/");
}
