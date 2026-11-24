//! Golden hashes of whole output files: `convert_file` writes each input at every rate and
//! cutoff (and with pysacd's filter at its rates), and the SHA-256 of every WAV file, header
//! and samples, is pinned per input. A change to the header (chunk layout, format tag,
//! channel mask) or to the samples fails here; output bytes change only with a major
//! version (AGENTS.md). The hashes were computed on 0.1.2 (updated for 0.2's channel mask
//! and f64 pysacd filter, see [`GOLDEN`]) and are the same on every SIMD level and platform.
//!
//! Inputs: the material of the sample golden tests (`identity.rs`) for 1, 2 and 6 channels,
//! full scale (all 0xFF), DSF files of channel types 5, 6 and 7, and DSDIFF files whose
//! `CHNL` chunk names speakers (four channels with LFE) and numbers them (three channels).
//! FLAC output is not written by this crate, so only WAV is covered.

use dsd2pcm::dsf::DsfWriter;
use dsd2pcm::io::DsdWrite;
use dsd2pcm::{convert_file, Cutoff, Filter, PcmOptions, PcmRate};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};

const FRAME: usize = 4704;

/// The material of `identity.rs`: random bytes, low-level patterns, idle, full-scale runs.
fn material(ch: usize, frames: usize, seed: u64) -> Vec<Vec<u8>> {
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    (0..ch)
        .map(|_| {
            (0..frames * FRAME)
                .map(|i| match (i / 3000) % 6 {
                    0 | 1 => rnd() as u8,
                    2 => 0x69,
                    3 => [0x55, 0x69, 0x96, 0xAA][(rnd() % 4) as usize],
                    4 => {
                        if (i / 700) % 2 == 0 {
                            0xFF
                        } else {
                            0x00
                        }
                    }
                    _ => (rnd() as u8) | 0x11,
                })
                .collect()
        })
        .collect()
}

fn dsf(chans: &[Vec<u8>], channel_type: Option<u32>) -> Vec<u8> {
    let mut w = DsfWriter::new(
        Cursor::new(Vec::new()),
        chans.len() as u16,
        2_822_400,
        channel_type,
    )
    .unwrap();
    w.write_bytes(chans).unwrap();
    w.finish(chans[0].len() as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

/// A DSDIFF file of `chans` whose `CHNL` chunk lists `ids`.
fn dff(chans: &[Vec<u8>], ids: &[&[u8; 4]]) -> Vec<u8> {
    assert_eq!(chans.len(), ids.len());
    let mut chnl = (ids.len() as u16).to_be_bytes().to_vec();
    for id in ids {
        chnl.extend_from_slice(*id);
    }
    let mut prop = b"SND ".to_vec();
    for (id, body) in [
        (b"FS  ", 2_822_400u32.to_be_bytes().to_vec()),
        (b"CHNL", chnl),
        (b"CMPR", b"DSD \x0enot compressed\x00".to_vec()),
    ] {
        prop.extend_from_slice(id);
        prop.extend_from_slice(&(body.len() as u64).to_be_bytes());
        prop.extend_from_slice(&body);
    }
    let mut data = Vec::new();
    for i in 0..chans[0].len() {
        for c in chans {
            data.push(c[i]);
        }
    }
    let mut body = b"DSD ".to_vec();
    body.extend_from_slice(b"FVER");
    body.extend_from_slice(&4u64.to_be_bytes());
    body.extend_from_slice(&0x0105_0000u32.to_be_bytes());
    body.extend_from_slice(b"PROP");
    body.extend_from_slice(&(prop.len() as u64).to_be_bytes());
    body.extend_from_slice(&prop);
    body.extend_from_slice(b"DSD ");
    body.extend_from_slice(&(data.len() as u64).to_be_bytes());
    body.extend_from_slice(&data);
    let mut f = b"FRM8".to_vec();
    f.extend_from_slice(&(body.len() as u64).to_be_bytes());
    f.extend_from_slice(&body);
    f
}

/// Every setting a file is converted with: the default filter at each rate and cutoff,
/// then pysacd's at 88.2 and 176.4 kHz.
fn settings() -> Vec<PcmOptions> {
    let mut v = Vec::new();
    for (rate, cutoff) in [
        (PcmRate::Hz88200, Cutoff::Low),
        (PcmRate::Hz88200, Cutoff::Standard),
        (PcmRate::Hz176400, Cutoff::Low),
        (PcmRate::Hz176400, Cutoff::Standard),
        (PcmRate::Hz176400, Cutoff::Wide),
        (PcmRate::Hz352800, Cutoff::Low),
        (PcmRate::Hz352800, Cutoff::Standard),
        (PcmRate::Hz352800, Cutoff::Wide),
    ] {
        let mut o = PcmOptions::new(rate);
        o.cutoff = cutoff;
        v.push(o);
    }
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400] {
        let mut o = PcmOptions::new(rate);
        o.filter = Filter::Pysacd;
        v.push(o);
    }
    v
}

/// SHA-256 over the whole WAV files `convert_file` writes from `input` with every setting,
/// in order, each preceded by its length.
fn digest(dir: &Path, name: &str, input: &[u8]) -> String {
    let (inp, out) = (dir.join(name), dir.join(format!("{name}.wav")));
    std::fs::write(&inp, input).unwrap();
    let mut h = Sha256::new();
    for opts in settings() {
        convert_file(&inp, &out, &opts).unwrap();
        let wav = std::fs::read(&out).unwrap();
        h.update((wav.len() as u64).to_le_bytes());
        h.update(&wav);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Inputs, by name: (name, file bytes).
fn inputs() -> Vec<(&'static str, Vec<u8>)> {
    let m = |ch: usize| material(ch, 3, 0x0dd_ba11 ^ ch as u64);
    let full = |ch: usize| vec![vec![0xFF; 3 * FRAME]; ch];
    vec![
        ("mono.dsf", dsf(&m(1), None)),
        ("stereo.dsf", dsf(&m(2), None)),
        ("six.dsf", dsf(&m(6), None)),
        ("full-scale-mono.dsf", dsf(&full(1), None)),
        ("full-scale-stereo.dsf", dsf(&full(2), None)),
        ("full-scale-six.dsf", dsf(&full(6), None)),
        // FL FR C LFE: its own mask differs from the four-channel default (quad).
        ("type5.dsf", dsf(&m(4), Some(5))),
        ("type6.dsf", dsf(&m(5), Some(6))),
        ("type7.dsf", dsf(&m(6), Some(7))),
        (
            "speakers.dff",
            dff(&m(4), &[b"MLFT", b"MRGT", b"C   ", b"LFE "]),
        ),
        ("numbered.dff", dff(&m(3), &[b"C000", b"C001", b"C002"])),
    ]
}

/// The whole-file hashes, per input in the order of [`inputs`], computed on 0.1.2 and
/// updated twice for 0.2: type 5 and the DSDIFF speakers' WAV header carries the input's
/// mask, FL FR C LFE (0x0F), where 0.1.2 wrote the four-channel default, quad (0x33); and
/// pysacd's filter is computed in `f64`, so its samples changed (its C reference, in
/// tests/pysacd-ref, pins them). Type 7 and six channels give the same files (the same samples,
/// and 7 is the type of six channels); so do type 5 and the DSDIFF speakers.
const GOLDEN: [&str; 11] = [
    "70d10fa4cd370c99a88f33170f5e88cf8076baafef461368b4956f828b8ee8db",
    "4be0ecccd9a74b4e58c6637d1138e32921f68a4d477258673fa6dd2f88ccaf46",
    "9d7a64f6d7f9fd030397717fe5becf8e95e6ec86ae9f4ab2ab9d6acec81cf2a8",
    "e05d39e07cddb00dbb859af666e6246ac5dbae35ce5b22f9168c5653df9b4514",
    "98a0223e3b4eccdee500ab149c85cc84d96658615347c41b4bbdf1a6551b6c63",
    "6d8c4a247a7cb77067bac0c2ac8b229442c8ac1b6479c5c79023590bead7a335",
    "dd6367837babfaabfe4b65a1259b46ae09de8d9ae5046e0ddf1e64e3dfa60f3b",
    "f13fbbbd087bfabff028665f0c21fade826d70b73845e4e63995c18958354af8",
    "9d7a64f6d7f9fd030397717fe5becf8e95e6ec86ae9f4ab2ab9d6acec81cf2a8",
    "dd6367837babfaabfe4b65a1259b46ae09de8d9ae5046e0ddf1e64e3dfa60f3b",
    "fbc957c1a6ad3e0e0bacd652e6db5fc301884b9b456574f4bd54b22db103bcd2",
];

#[test]
fn golden_whole_files() {
    let dir: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-files");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let got: Vec<(&str, String)> = inputs()
        .into_iter()
        .map(|(name, bytes)| (name, digest(&dir, name, &bytes)))
        .collect();
    for (name, d) in &got {
        eprintln!("{name}: \"{d}\",");
    }
    for ((name, d), want) in got.iter().zip(GOLDEN) {
        assert_eq!(d, want, "{name}: whole output files changed");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
