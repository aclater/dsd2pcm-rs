//! Writes arbitrary audio with the crate's DSF, DFF and WAV writers (in arbitrary write
//! sizes) and checks that the readers return exactly what was written.
#![no_main]

use arbitrary::Arbitrary;
use dsd2pcm::dff::{DffReader, DffWriter};
use dsd2pcm::dsf::{default_channel_type, DsfReader, DsfWriter, MAX_METADATA};
use dsd2pcm::io::{DsdRead, DsdWrite, PcmRead};
use dsd2pcm::wav::{WavFormat, WavReader, WavWriter};
use dsd2pcm_fuzz::read_sizes;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

const RATES: [u32; 10] = [
    2_822_400, 5_644_800, 11_289_600, 22_579_200, 45_158_400, 3_072_000, 6_144_000, 12_288_000,
    24_576_000, 49_152_000,
];

#[derive(Arbitrary, Debug)]
enum Case {
    Dsf {
        channels: u8,
        rate: u8,
        channel_type: Option<u32>,
        trim: u8,
        meta: Option<Vec<u8>>,
        writes: Vec<u16>,
        data: Vec<u8>,
    },
    Dff {
        channels: u8,
        rate: u8,
        writes: Vec<u16>,
        data: Vec<u8>,
    },
    Wav {
        format: u8,
        channels: u8,
        rate: u32,
        data: Vec<u8>,
    },
}

/// Splits `data` into `ch` equal channels.
fn split(data: &[u8], ch: usize) -> Vec<Vec<u8>> {
    let per = data.len() / ch;
    (0..ch)
        .map(|c| data[c * per..(c + 1) * per].to_vec())
        .collect()
}

/// Feeds `chans` to `w` in pieces whose sizes come from `writes`.
fn write_in_pieces(w: &mut dyn DsdWrite, chans: &[Vec<u8>], writes: &[u16]) {
    let len = chans[0].len();
    let mut at = 0;
    let mut i = 0;
    while at < len {
        let step = writes
            .get(i % writes.len().max(1))
            .map_or(len, |&s| usize::from(s).max(1))
            .min(len - at);
        let piece: Vec<Vec<u8>> = chans.iter().map(|c| c[at..at + step].to_vec()).collect();
        w.write_bytes(&piece).unwrap();
        at += step;
        i += 1;
    }
}

fn read_back(r: &mut dyn DsdRead, seed: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![Vec::new(); usize::from(r.info().channels)];
    for max in read_sizes(seed) {
        if r.read_bytes(max, &mut out).unwrap() == 0 {
            break;
        }
    }
    out
}

fn check_wav(format: WavFormat, raw: &[u8], got: &[Vec<f64>]) {
    let ch = got.len();
    let bs = format.bytes();
    for (i, b) in raw.chunks_exact(bs).enumerate() {
        let s = got[i % ch][i / ch];
        let ok = match format {
            WavFormat::I16 => (s * 32768.0) as i16 == i16::from_le_bytes([b[0], b[1]]),
            WavFormat::I24 => {
                (s * 8_388_608.0) as i32 == i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8
            }
            WavFormat::I32 => {
                (s * 2_147_483_648.0) as i64
                    == i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            WavFormat::F32 => {
                let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if v.is_finite() {
                    s == f64::from(v)
                } else {
                    s == 0.0
                }
            }
            WavFormat::F64 => {
                let v = f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                if v.is_finite() {
                    s == v
                } else {
                    s == 0.0
                }
            }
            f => unreachable!("format {f:?}"),
        };
        assert!(ok, "WAV sample {i} differs");
    }
}

fuzz_target!(|case: Case| match case {
    Case::Dsf {
        channels,
        rate,
        channel_type,
        trim,
        meta,
        writes,
        data,
    } => {
        let ch = usize::from(channels % 6 + 1);
        let rate = RATES[usize::from(rate) % RATES.len()];
        let chans = split(&data, ch);
        let bytes = chans[0].len() as u64;
        let samples = (bytes * 8).saturating_sub(u64::from(trim % 8));
        let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch as u16, rate, channel_type).unwrap();
        w.set_metadata(meta.clone());
        write_in_pieces(&mut w, &chans, &writes);
        w.finish(samples).unwrap();
        let file = w.into_inner().into_inner();
        let mut r = DsfReader::new(Cursor::new(&file)).unwrap();
        let info = r.info();
        assert_eq!(
            (usize::from(info.channels), info.sample_rate, info.samples),
            (ch, rate, samples)
        );
        let ct = match channel_type {
            Some(t @ 1..=7) => t,
            _ => default_channel_type(ch as u16),
        };
        assert_eq!(r.channel_type(), Some(ct));
        let want_meta = meta.filter(|m| !m.is_empty() && m.len() as u64 <= MAX_METADATA);
        assert_eq!(r.metadata(), want_meta.as_deref());
        let got = read_back(&mut r, &file);
        let keep = samples.div_ceil(8) as usize;
        // The padding bits past `samples` in the last byte read as 0x69's.
        let pad = (keep as u64 * 8 - samples) as u32;
        let mask = ((1u16 << pad) - 1) as u8;
        for (g, c) in got.iter().zip(&chans) {
            let mut want = c[..keep].to_vec();
            if let Some(last) = want.last_mut() {
                *last = (*last & !mask) | (0x69 & mask);
            }
            assert_eq!(g[..], want[..]);
        }
    }
    Case::Dff {
        channels,
        rate,
        writes,
        data,
    } => {
        let ch = usize::from(channels % 6 + 1);
        let rate = RATES[usize::from(rate) % RATES.len()];
        let chans = split(&data, ch);
        let samples = chans[0].len() as u64 * 8;
        let mut w = DffWriter::new(Cursor::new(Vec::new()), ch as u16, rate).unwrap();
        write_in_pieces(&mut w, &chans, &writes);
        w.finish(samples).unwrap();
        let file = w.into_inner().into_inner();
        let mut r = DffReader::new(Cursor::new(&file)).unwrap();
        let info = r.info();
        assert_eq!(
            (usize::from(info.channels), info.sample_rate, info.samples),
            (ch, rate, samples)
        );
        assert_eq!(read_back(&mut r, &file), chans);
    }
    Case::Wav {
        format,
        channels,
        rate,
        data,
    } => {
        let format = [
            WavFormat::I16,
            WavFormat::I24,
            WavFormat::I32,
            WavFormat::F32,
            WavFormat::F64,
        ][usize::from(format) % 5];
        let ch = usize::from(channels % 6 + 1);
        let frame = ch * format.bytes();
        let raw = &data[..data.len() / frame * frame];
        let Ok(mut w) = WavWriter::new(Cursor::new(Vec::new()), format, ch as u16, rate) else {
            assert!(rate == 0 || u64::from(rate) * frame as u64 > u64::from(u32::MAX));
            return;
        };
        w.write_raw(raw).unwrap();
        w.finish().unwrap();
        let file = w.into_inner().into_inner();
        let mut r = WavReader::new(Cursor::new(&file)).unwrap();
        let info = r.info();
        assert_eq!(r.format(), format);
        assert_eq!((usize::from(info.channels), info.sample_rate), (ch, rate));
        assert_eq!(info.frames, (raw.len() / frame) as u64);
        let mut out = vec![Vec::new(); ch];
        for max in read_sizes(&file) {
            if r.read_frames(max, &mut out).unwrap() == 0 {
                break;
            }
        }
        check_wav(format, raw, &out);
    }
});
