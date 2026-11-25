//! Container round trips and malformed-input handling.

use dsd2pcm::dff::{DffReader, DffWriter};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite, PcmRead};
use dsd2pcm::wav::{WavFormat, WavReader, WavWriter};
use dsd2pcm::Error;
use std::io::Cursor;

fn pattern(ch: usize, n: usize) -> Vec<Vec<u8>> {
    (0..ch)
        .map(|c| (0..n).map(|i| (i * 31 + c * 7 + (i >> 8)) as u8).collect())
        .collect()
}

fn read_all(r: &mut dyn DsdRead, max: usize) -> Vec<Vec<u8>> {
    let ch = usize::from(r.info().channels);
    let mut out = vec![Vec::new(); ch];
    while r.read_bytes(max, &mut out).unwrap() > 0 {}
    out
}

fn dsf_bytes(data: &[Vec<u8>], samples: u64, rate: u32, meta: Option<Vec<u8>>) -> Vec<u8> {
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), data.len() as u16, rate, None).unwrap();
    w.set_metadata(meta);
    w.write_bytes(data).unwrap();
    w.finish(samples).unwrap();
    w.into_inner().into_inner()
}

fn dff_bytes(data: &[Vec<u8>], rate: u32) -> Vec<u8> {
    let mut w = DffWriter::new(Cursor::new(Vec::new()), data.len() as u16, rate).unwrap();
    w.write_bytes(data).unwrap();
    w.finish(data[0].len() as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

#[test]
fn dsf_round_trip_all_rates_and_channels() {
    for rate in [2_822_400u32, 5_644_800, 11_289_600, 22_579_200, 45_158_400] {
        for ch in [1usize, 2, 6] {
            let n = 4096 * 2 + 777;
            let data = pattern(ch, n);
            let samples = n as u64 * 8 - 3;
            let f = dsf_bytes(&data, samples, rate, None);
            assert_eq!(f.len(), 92 + 3 * 4096 * ch);
            let mut r = DsfReader::new(Cursor::new(f)).unwrap();
            let i = r.info();
            assert_eq!(
                (i.channels as usize, i.sample_rate, i.samples),
                (ch, rate, samples)
            );
            // The 3 padding bits of the last byte are delivered as 0x69's.
            let mut want = data.clone();
            for c in &mut want {
                let last = c.last_mut().unwrap();
                *last = (*last & !0b111) | (0x69 & 0b111);
            }
            for max in [1, 5000, 1 << 20] {
                let mut r2 =
                    DsfReader::new(Cursor::new(dsf_bytes(&data, samples, rate, None))).unwrap();
                assert_eq!(read_all(&mut r2, max), want);
            }
            assert_eq!(read_all(&mut r, 4096), want);
        }
    }
}

#[test]
fn dsf_metadata_round_trip() {
    let data = pattern(2, 100);
    let tag = b"ID3\x04\x00\x00\x00\x00\x00\x05hello".to_vec();
    let f = dsf_bytes(&data, 800, 2_822_400, Some(tag.clone()));
    let r = DsfReader::new(Cursor::new(f)).unwrap();
    assert_eq!(r.metadata(), Some(&tag[..]));
}

#[test]
fn dsf_lsb_first_on_disk() {
    // first sample +1, rest -1: MSB-first 0x80 must be stored as 0x01
    let data = vec![vec![0x80u8]];
    let f = dsf_bytes(&data, 8, 2_822_400, None);
    assert_eq!(f[92], 0x01);
}

#[test]
fn dff_round_trip() {
    for ch in [1usize, 2, 3, 5, 6] {
        let data = pattern(ch, 1001);
        let f = dff_bytes(&data, 5_644_800);
        let mut r = DffReader::new(Cursor::new(f)).unwrap();
        let i = r.info();
        assert_eq!(
            (i.channels as usize, i.sample_rate, i.samples),
            (ch, 5_644_800, 1001 * 8)
        );
        assert_eq!(read_all(&mut r, 333), data);
    }
}

#[test]
fn dff_rejects_dst() {
    let mut f = dff_bytes(&pattern(2, 10), 2_822_400);
    let at = f.windows(4).position(|w| w == b"CMPR").unwrap() + 12;
    f[at..at + 4].copy_from_slice(b"DST ");
    match DffReader::new(Cursor::new(f)) {
        Err(Error::Unsupported(m)) => assert!(m.message().contains("DST")),
        other => panic!("expected DST rejection, got {other:?}"),
    }
}

#[test]
fn wav_round_trip_formats() {
    let vals = [0.0f64, 0.5, -0.5, 0.999, -1.0, 0.25];
    for fmt in [
        WavFormat::I16,
        WavFormat::I24,
        WavFormat::I32,
        WavFormat::F32,
        WavFormat::F64,
    ] {
        let mut w = WavWriter::new(Cursor::new(Vec::new()), fmt, 2, 44_100).unwrap();
        let mut raw = Vec::new();
        for &v in &vals {
            for _ in 0..2 {
                match fmt {
                    WavFormat::I16 => raw.extend_from_slice(&((v * 32768.0) as i16).to_le_bytes()),
                    WavFormat::I24 => {
                        raw.extend_from_slice(&((v * 8_388_608.0) as i32).to_le_bytes()[..3])
                    }
                    WavFormat::I32 => {
                        raw.extend_from_slice(&((v * 2_147_483_648.0) as i32).to_le_bytes())
                    }
                    WavFormat::F32 => raw.extend_from_slice(&(v as f32).to_le_bytes()),
                    WavFormat::F64 => raw.extend_from_slice(&v.to_le_bytes()),
                    f => unreachable!("format {f:?}"),
                }
            }
        }
        w.write_raw(&raw).unwrap();
        w.finish().unwrap();
        let f = w.into_inner().into_inner();
        let mut r = WavReader::new(Cursor::new(f)).unwrap();
        assert_eq!(r.info().frames, vals.len() as u64);
        let mut out = vec![Vec::new(), Vec::new()];
        while r.read_frames(4, &mut out).unwrap() > 0 {}
        for (i, &v) in vals.iter().enumerate() {
            let tol = match fmt {
                WavFormat::I16 => 1.0 / 32768.0,
                WavFormat::F32 => 1e-7,
                _ => 1.0 / 8_388_608.0,
            };
            assert!(
                (out[0][i] - v).abs() <= tol && out[1][i] == out[0][i],
                "{fmt:?} {i}"
            );
        }
    }
}

/// Every truncation and many single-byte corruptions of valid files give a result or an
/// error, never a panic.
#[test]
#[cfg_attr(
    miri,
    ignore = "too slow under Miri: thousands of corrupted files; the fuzz targets cover it"
)]
fn malformed_inputs_do_not_panic() {
    let files = [
        dsf_bytes(
            &pattern(2, 5000),
            40_000,
            2_822_400,
            Some(b"ID3xxxx".to_vec()),
        ),
        dff_bytes(&pattern(2, 500), 2_822_400),
        {
            let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, 2, 88_200).unwrap();
            w.write_raw(&[1u8; 600]).unwrap();
            w.finish().unwrap();
            w.into_inner().into_inner()
        },
    ];
    let try_all = |b: Vec<u8>| {
        if let Ok(mut r) = DsfReader::new(Cursor::new(b.clone())) {
            let mut o = vec![Vec::new(); usize::from(r.info().channels)];
            while let Ok(n) = r.read_bytes(4096, &mut o) {
                if n == 0 {
                    break;
                }
            }
        }
        if let Ok(mut r) = DffReader::new(Cursor::new(b.clone())) {
            let mut o = vec![Vec::new(); usize::from(r.info().channels)];
            while let Ok(n) = r.read_bytes(4096, &mut o) {
                if n == 0 {
                    break;
                }
            }
        }
        if let Ok(mut r) = WavReader::new(Cursor::new(b)) {
            let mut o = vec![Vec::new(); usize::from(r.info().channels)];
            while let Ok(n) = r.read_frames(4096, &mut o) {
                if n == 0 {
                    break;
                }
            }
        }
    };
    for f in &files {
        let hdr = 200.min(f.len());
        for cut in 0..f.len().min(400) {
            try_all(f[..cut].to_vec());
        }
        for pos in 0..hdr {
            for v in [0x00u8, 0xFF, 0x7F, 0x80, 0x01] {
                let mut b = f.clone();
                b[pos] = v;
                try_all(b);
            }
        }
    }
}

/// A source that declares an hour of stereo DSD64 and delivers one byte per channel.
struct ShortSource(bool);

impl dsd2pcm::io::DsdRead for ShortSource {
    fn info(&self) -> dsd2pcm::io::DsdInfo {
        dsd2pcm::io::DsdInfo::new(2, 2_822_400, 2_822_400 * 3600)
    }
    fn read_bytes(&mut self, _max: usize, out: &mut [Vec<u8>]) -> dsd2pcm::Result<usize> {
        if std::mem::replace(&mut self.0, true) {
            return Ok(0);
        }
        for o in out.iter_mut() {
            o.push(0x69);
        }
        Ok(1)
    }
}

#[test]
fn source_ending_early_is_an_error() {
    let mut wav = dsd2pcm::wav::WavWriter::new(
        std::io::Cursor::new(Vec::new()),
        dsd2pcm::wav::WavFormat::I24,
        2,
        88_200,
    )
    .unwrap();
    let opts = dsd2pcm::PcmOptions::new(dsd2pcm::PcmRate::Hz88200);
    let r = dsd2pcm::convert::dsd64_to_pcm(Box::new(ShortSource(false)), &mut wav, &opts);
    assert!(matches!(r, Err(dsd2pcm::Error::Format(_))), "{r:?}");
    assert!(wav.into_inner().into_inner().len() < 1 << 20);
}

/// A DSD128 source: refused by every filter (only DSD64 is converted so far).
struct Dsd128Source;

impl dsd2pcm::io::DsdRead for Dsd128Source {
    fn info(&self) -> dsd2pcm::io::DsdInfo {
        dsd2pcm::io::DsdInfo::new(2, 5_644_800, 5_644_800)
    }
    fn read_bytes(&mut self, _max: usize, out: &mut [Vec<u8>]) -> dsd2pcm::Result<usize> {
        for o in out.iter_mut() {
            o.push(0x69);
        }
        Ok(1)
    }
}

#[test]
fn dsd128_input_is_refused_by_every_filter() {
    for filter in [
        dsd2pcm::Filter::Precise,
        dsd2pcm::Filter::Fast,
        dsd2pcm::Filter::Pysacd,
    ] {
        let mut wav = dsd2pcm::wav::WavWriter::new(
            std::io::Cursor::new(Vec::new()),
            dsd2pcm::wav::WavFormat::I24,
            2,
            88_200,
        )
        .unwrap();
        let mut opts = dsd2pcm::PcmOptions::new(dsd2pcm::PcmRate::Hz88200);
        opts.filter = filter;
        let r = dsd2pcm::convert::dsd64_to_pcm(Box::new(Dsd128Source), &mut wav, &opts);
        assert!(
            matches!(r, Err(dsd2pcm::Error::Unsupported(_))),
            "{filter:?}: {r:?}"
        );
    }
}

/// A 24-bit stereo WAV made by the crate's writer with `frames` frames.
fn wav_i24(frames: usize) -> Vec<u8> {
    let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, 2, 88_200).unwrap();
    let raw: Vec<u8> = (0..frames * 6).map(|i| (i * 13) as u8).collect();
    w.write_raw(&raw).unwrap();
    w.finish().unwrap();
    w.into_inner().into_inner()
}

fn wav_samples(b: Vec<u8>) -> dsd2pcm::Result<Vec<Vec<f64>>> {
    let mut r = WavReader::new(Cursor::new(b))?;
    let mut out = vec![Vec::new(); usize::from(r.info().channels)];
    while r.read_frames(7, &mut out)? > 0 {}
    assert_eq!(out[0].len() as u64, r.info().frames);
    Ok(out)
}

/// Sets the `data` chunk size of a file from [`wav_i24`] (and of RIFF, as such encoders do).
fn set_data_size(mut b: Vec<u8>, size: u32) -> Vec<u8> {
    let at = b.windows(4).position(|w| w == b"data").unwrap();
    b[at + 4..at + 8].copy_from_slice(&size.to_le_bytes());
    b[4..8].copy_from_slice(&size.to_le_bytes());
    b
}

#[test]
fn wav_data_size_zero_as_last_chunk_reads_to_the_end() {
    let good = wav_samples(wav_i24(100)).unwrap();
    assert_eq!(good[0].len(), 100);
    assert_eq!(wav_samples(set_data_size(wav_i24(100), 0)).unwrap(), good);
    // A trailing partial frame is dropped.
    let mut b = set_data_size(wav_i24(100), 0);
    b.extend_from_slice(&[1, 2, 3, 4, 5]);
    assert_eq!(wav_samples(b).unwrap(), good);
}

#[test]
fn wav_data_size_placeholder_reads_to_the_end() {
    let good = wav_samples(wav_i24(100)).unwrap();
    assert_eq!(
        wav_samples(set_data_size(wav_i24(100), u32::MAX)).unwrap(),
        good
    );
}

#[test]
fn wav_data_size_zero_followed_by_a_chunk_is_an_error() {
    for size in [0u32, u32::MAX] {
        let mut b = set_data_size(wav_i24(0), size);
        b.extend_from_slice(b"LIST\x05\0\0\0INFOx\0");
        let r = WavReader::new(Cursor::new(b));
        assert!(matches!(r, Err(Error::Format(_))), "size {size:#x}: {r:?}");
    }
}

#[test]
fn wav_empty_data_chunk_is_zero_frames() {
    // Size 0 with nothing after it is a valid empty file.
    assert!(wav_samples(wav_i24(0)).unwrap()[0].is_empty());
    // A correct size followed by another chunk reads only the declared samples.
    let mut b = wav_i24(10);
    b.extend_from_slice(b"LIST\x04\0\0\0INFO");
    assert_eq!(wav_samples(b).unwrap()[0].len(), 10);
}

/// `wav::decode` gives exactly what `WavReader` reads from the same bytes, for every format.
#[test]
fn wav_decode_matches_the_reader() {
    use dsd2pcm::wav::{decode, WavFormat, WavReader, WavWriter};
    for format in [
        WavFormat::I16,
        WavFormat::I24,
        WavFormat::I32,
        WavFormat::F32,
        WavFormat::F64,
    ] {
        // 600 samples of 2 channels; pseudo-random bytes, so floats include NaN and inf.
        let mut x = 0x9E37_79B9u32;
        let data: Vec<u8> = (0..600 * format.bytes())
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let mut w = WavWriter::new(Cursor::new(Vec::new()), format, 2, 88_200).unwrap();
        w.write_raw(&data).unwrap();
        w.finish().unwrap();
        let mut r = WavReader::new(Cursor::new(w.into_inner().into_inner())).unwrap();
        let mut read = vec![Vec::new(), Vec::new()];
        while r.read_frames(1000, &mut read).unwrap() > 0 {}
        // A trailing partial sample is ignored.
        let mut with_tail = data.clone();
        with_tail.push(0x7f);
        let mut out = Vec::new();
        assert_eq!(decode(format, &with_tail, &mut out), 600);
        assert_eq!(out.len(), 600);
        for (i, v) in out.iter().enumerate() {
            assert_eq!(
                v.to_bits(),
                read[i % 2][i / 2].to_bits(),
                "{format:?} sample {i}"
            );
        }
    }
}

/// `wav::header_len` is where WavWriter's samples start, for every format and channel count.
#[test]
fn wav_header_len_matches_the_writer() {
    use dsd2pcm::wav::{header_len, WavFormat, WavWriter};
    for format in [
        WavFormat::I16,
        WavFormat::I24,
        WavFormat::I32,
        WavFormat::F32,
        WavFormat::F64,
    ] {
        for ch in 1..=6u16 {
            let w = WavWriter::new(Cursor::new(Vec::new()), format, ch, 88_200).unwrap();
            assert_eq!(w.data_offset(), header_len(format, ch), "{format:?} {ch}");
            assert_eq!(
                w.into_inner().into_inner().len() as u64,
                header_len(format, ch)
            );
        }
    }
}

/// A source that declares 2^37 samples (2^32 outputs at 88.2 kHz, a count a 32-bit `usize`
/// truncates to 0) and ends after 1 MiB per channel.
struct HugeShort(bool);

impl DsdRead for HugeShort {
    fn info(&self) -> dsd2pcm::io::DsdInfo {
        dsd2pcm::io::DsdInfo::new(1, 2_822_400, 1 << 37)
    }
    fn read_bytes(&mut self, _max: usize, out: &mut [Vec<u8>]) -> dsd2pcm::Result<usize> {
        if std::mem::replace(&mut self.0, true) {
            return Ok(0);
        }
        out[0].extend(std::iter::repeat_n(0x69, 1 << 20));
        Ok(1 << 20)
    }
}

#[test]
fn output_counts_past_32_bits_do_not_panic() {
    for filter in [
        dsd2pcm::Filter::Precise,
        dsd2pcm::Filter::Fast,
        dsd2pcm::Filter::Pysacd,
    ] {
        let mut wav = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, 1, 88_200).unwrap();
        let mut opts = dsd2pcm::PcmOptions::new(dsd2pcm::PcmRate::Hz88200);
        opts.filter = filter;
        let r = dsd2pcm::convert::dsd64_to_pcm(Box::new(HugeShort(false)), &mut wav, &opts);
        assert!(matches!(r, Err(Error::Format(_))), "{filter:?}: {r:?}");
    }
}

/// A writer that keeps only its length.
struct Discard(u64, u64);

impl std::io::Write for Discard {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0 += b.len() as u64;
        self.1 = self.1.max(self.0);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Seek for Discard {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0 = match p {
            std::io::SeekFrom::Start(n) => n,
            std::io::SeekFrom::End(d) => self.1.checked_add_signed(d).unwrap(),
            std::io::SeekFrom::Current(d) => self.0.checked_add_signed(d).unwrap(),
        };
        Ok(self.0)
    }
}

#[test]
fn wav_writer_refuses_data_past_4_gib() {
    let mut w = WavWriter::new(Discard(0, 0), WavFormat::I24, 6, 176_400).unwrap();
    let block = vec![0u8; 1 << 20];
    let mut written = 0u64;
    let err = loop {
        match w.write_raw(&block) {
            Ok(()) => written += block.len() as u64,
            Err(e) => break e,
        }
    };
    assert!(matches!(err, Error::Unsupported(_)), "{err:?}");
    // The refused block was not written, and the file still finishes within the limit.
    assert_eq!(w.data_offset() + written, w.into_inner().0);
    let mut w = WavWriter::new(Discard(0, 0), WavFormat::I24, 6, 176_400).unwrap();
    for _ in 0..written >> 20 {
        w.write_raw(&block).unwrap();
    }
    w.finish().unwrap();
    assert!(w.into_inner().1 <= u64::from(u32::MAX) + 8);
}

// ---- Reader rules for files that bend the format (each accepted or refused on purpose) ----

/// A `data` size past the end of the file (a file cut short) reads the whole frames there;
/// `declared_frames` keeps what the header declared.
#[test]
fn wav_data_size_past_the_end_reads_what_is_there() {
    let good = wav_samples(wav_i24(100)).unwrap();
    // Declares 100 frames, holds 60 and half of the 61st.
    let mut b = wav_i24(100);
    b.truncate(b.len() - 40 * 6 + 3);
    let r = WavReader::new(Cursor::new(b.clone())).unwrap();
    assert_eq!((r.info().frames, r.declared_frames()), (60, 100));
    let got = wav_samples(b).unwrap();
    assert_eq!(got[0], good[0][..60]);
    assert_eq!(got[1], good[1][..60]);
    // A whole file declares what it holds.
    let r = WavReader::new(Cursor::new(wav_i24(100))).unwrap();
    assert_eq!((r.info().frames, r.declared_frames()), (100, 100));
}

/// A stereo `WAVE_FORMAT_EXTENSIBLE` file of 24-bit containers with the given valid bits
/// and sub-format GUID, holding `samples` (left-justified, as written).
fn wav_extensible(valid: u16, guid: [u8; 16], samples: &[i32]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&48_000u32.to_le_bytes());
    fmt.extend_from_slice(&(48_000u32 * 6).to_le_bytes());
    fmt.extend_from_slice(&6u16.to_le_bytes());
    fmt.extend_from_slice(&24u16.to_le_bytes());
    fmt.extend_from_slice(&22u16.to_le_bytes());
    fmt.extend_from_slice(&valid.to_le_bytes());
    fmt.extend_from_slice(&3u32.to_le_bytes());
    fmt.extend_from_slice(&guid);
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|&v| v.to_le_bytes()[..3].to_vec())
        .collect();
    let mut b = b"WAVEfmt ".to_vec();
    b.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    b.extend_from_slice(&fmt);
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(b.len() as u32).to_le_bytes());
    f.extend_from_slice(&b);
    f
}

/// KSDATAFORMAT_SUBTYPE_PCM as stored.
const PCM_GUID: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// Valid bits: 0 (not given) and fewer than the container (20 in 24, as `flac -d` writes)
/// read at the container's width; more than the container is malformed.
#[test]
fn wav_extensible_valid_bits() {
    // 20-bit values, left-justified in 24 bits.
    let samples: Vec<i32> = (0..20).map(|i| (i * 40_000 - 300_000) << 4).collect();
    let want: Vec<f64> = samples
        .iter()
        .step_by(2)
        .map(|&v| f64::from(v) / 8_388_608.0)
        .collect();
    for valid in [0u16, 20, 24] {
        let got = wav_samples(wav_extensible(valid, PCM_GUID, &samples)).unwrap();
        assert_eq!(got[0], want, "valid bits {valid}");
    }
    let e = WavReader::new(Cursor::new(wav_extensible(25, PCM_GUID, &samples))).unwrap_err();
    assert!(
        matches!(&e, Error::Format(m) if m.kind() == dsd2pcm::FormatErrorKind::Malformed
            && m.message().contains("valid bits 25")),
        "{e}"
    );
}

/// The sub-format GUID must be KSDATAFORMAT's: an Ambisonic B-format GUID, whose first
/// bytes read like PCM's, and a GUID with one byte changed are refused as unsupported.
#[test]
fn wav_extensible_sub_format_guid() {
    let samples = [1i32 << 8; 4];
    assert!(wav_samples(wav_extensible(24, PCM_GUID, &samples)).is_ok());
    let ambisonic = [
        0x01, 0x00, 0x00, 0x00, 0x21, 0x07, 0xD3, 0x11, 0x86, 0x44, 0xC8, 0xC1, 0xCA, 0x00, 0x00,
        0x00,
    ];
    let mut one_off = PCM_GUID;
    one_off[15] ^= 1;
    for (guid, text) in [
        (ambisonic, "00000001-0721-11D3-8644-C8C1CA000000"),
        (one_off, "00000001-0000-0010-8000-00AA00389B70"),
    ] {
        let e = WavReader::new(Cursor::new(wav_extensible(24, guid, &samples))).unwrap_err();
        assert!(
            matches!(&e, Error::Unsupported(m) if m.kind() == dsd2pcm::UnsupportedKind::Encoding
                && m.message().contains(text)),
            "{e}"
        );
    }
}

/// A DSDIFF `DSD ` chunk whose length is not a multiple of the channel count: the partial
/// last turn (fewer bytes than channels) is not read; the rest reads as usual.
#[test]
fn dff_partial_last_turn_is_not_read() {
    let data = pattern(3, 100);
    let mut b = dff_bytes(&data, 2_822_400);
    // Append two bytes of a 101st turn to the 'DSD ' chunk (and its FRM8 size).
    let at = b.windows(4).rposition(|w| w == b"DSD ").unwrap();
    let size = u64::from_be_bytes(b[at + 4..at + 12].try_into().unwrap());
    assert_eq!(size, 300);
    b[at + 4..at + 12].copy_from_slice(&302u64.to_be_bytes());
    b.extend_from_slice(&[0xAA, 0xBB]);
    let form = u64::from_be_bytes(b[4..12].try_into().unwrap());
    b[4..12].copy_from_slice(&(form + 2).to_be_bytes());
    let mut r = DffReader::new(Cursor::new(b)).unwrap();
    assert_eq!(r.info().samples, 800);
    assert_eq!(read_all(&mut r, 7), data);
}

/// A DSF tag larger than `MAX_METADATA` is not loaded: the audio reads, `metadata` is
/// `None` and `skipped_metadata_len` gives its size. One of exactly that size is loaded.
#[test]
#[cfg_attr(
    miri,
    ignore = "16 MiB tags: over 8 GB of Miri state; runs natively at every SIMD level"
)]
fn dsf_oversized_tag_is_skipped_not_fatal() {
    let data = pattern(2, 4096);
    let max = dsd2pcm::dsf::MAX_METADATA as usize;
    for (len, loaded) in [(max, true), (max + 1, false)] {
        let mut tag = b"ID3".to_vec();
        tag.resize(len, 0);
        let b = dsf_bytes(&data, 4096 * 8, 2_822_400, Some(tag.clone()));
        let mut r = DsfReader::new(Cursor::new(b)).unwrap();
        if loaded {
            assert_eq!(r.metadata(), Some(&tag[..]));
            assert_eq!(r.skipped_metadata_len(), None);
        } else {
            assert_eq!(r.metadata(), None);
            assert_eq!(r.skipped_metadata_len(), Some(len as u64));
        }
        assert_eq!(read_all(&mut r, 1000), data);
    }
}
