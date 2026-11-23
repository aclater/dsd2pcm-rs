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
            for max in [1, 5000, 1 << 20] {
                let mut r2 =
                    DsfReader::new(Cursor::new(dsf_bytes(&data, samples, rate, None))).unwrap();
                assert_eq!(read_all(&mut r2, max), data);
            }
            assert_eq!(read_all(&mut r, 4096), data);
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
        Err(Error::Unsupported(m)) => assert!(m.contains("DST")),
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
        dsd2pcm::io::DsdInfo {
            channels: 2,
            sample_rate: 2_822_400,
            samples: 2_822_400 * 3600,
        }
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
