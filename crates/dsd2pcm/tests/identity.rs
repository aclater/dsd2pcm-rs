//! Output samples are identical to scarletbook-pcm 0.7.0 for every rate, cutoff and gain.

use dsd2pcm::convert::dsd64_to_pcm;
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{Cutoff, PcmOptions, PcmRate};
use std::io::Cursor;

const FRAME: usize = 4704;

/// Test material per channel: random bytes, low-level patterns, idle, full-scale runs.
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

fn ours(chans: &[Vec<u8>], opts: &PcmOptions) -> (Vec<u8>, Vec<u64>) {
    let mut w =
        DsfWriter::new(Cursor::new(Vec::new()), chans.len() as u16, 2_822_400, None).unwrap();
    w.write_bytes(chans).unwrap();
    w.finish(chans[0].len() as u64 * 8).unwrap();
    let dsf = w.into_inner().into_inner();
    let r = DsfReader::new(Cursor::new(dsf)).unwrap();
    let mut wav = WavWriter::new(
        Cursor::new(Vec::new()),
        WavFormat::I24,
        chans.len() as u16,
        opts.rate.hz(),
    )
    .unwrap();
    let rep = dsd64_to_pcm(Box::new(r), &mut wav, opts).unwrap();
    wav.finish().unwrap();
    let off = wav.data_offset() as usize;
    let bytes = wav.into_inner().into_inner();
    (bytes[off..].to_vec(), rep.clipped)
}

fn theirs(chans: &[Vec<u8>], rate: PcmRate, cutoff: Cutoff, gain_db: f64) -> (Vec<u8>, Vec<u64>) {
    use scarletbook_pcm as sb;
    let ch = chans.len();
    let rate = sb::PcmRate::from_hz(rate.hz()).unwrap();
    let cutoff = match cutoff {
        Cutoff::Standard => sb::Cutoff::Standard,
        Cutoff::Low => sb::Cutoff::Low,
        Cutoff::Wide => sb::Cutoff::Wide,
    };
    let cfg =
        sb::PcmConfig::new(rate, cutoff, ch as u8).with_gain(sb::Gain::from_db(gain_db).unwrap());
    let mut conv = sb::Converter::new(cfg).unwrap();
    let n = chans[0].len();
    let mut inter = vec![0u8; n * ch];
    for (c, d) in chans.iter().enumerate() {
        for (i, &b) in d.iter().enumerate() {
            inter[i * ch + c] = b;
        }
    }
    let frames = n / FRAME;
    let mut out = vec![0u8; frames * conv.samples_per_frame() * ch * 3];
    let stats = conv
        .convert(sb::Run::new(&inter[..]), sb::PcmOut::S24le(&mut out), None)
        .unwrap();
    (out, stats.clipped[..ch].to_vec())
}

fn rate_div(r: PcmRate) -> usize {
    r.decimation() as usize
}

#[test]
fn identical_to_scarletbook_pcm() {
    let chans = material(2, 7, 0x1234_5678_9abc_def1);
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
        for cutoff in [Cutoff::Low, Cutoff::Standard, Cutoff::Wide] {
            if cutoff == Cutoff::Wide && rate == PcmRate::Hz88200 {
                continue;
            }
            for gain_db in [0.0, -3.0, 6.0] {
                let opts = PcmOptions {
                    rate,
                    cutoff,
                    gain_db,
                    threads: true,
                    filter: dsd2pcm::Filter::Default,
                };
                let (a, ca) = ours(&chans, &opts);
                let (b, cb) = theirs(&chans, rate, cutoff, gain_db);
                assert_eq!(a.len(), b.len(), "{rate:?} {cutoff:?} {gain_db}");
                assert_eq!(a.len(), 7 * FRAME * 8 / rate_div(rate) * 2 * 3);
                if gain_db == 6.0 {
                    assert!(ca.iter().sum::<u64>() > 0, "expected clipping at +6 dB");
                }
                assert!(a == b, "samples differ: {rate:?} {cutoff:?} {gain_db} dB");
                assert_eq!(ca, cb, "clip counts: {rate:?} {cutoff:?} {gain_db} dB");
            }
        }
    }
}

#[test]
fn identical_mono_and_six_channels() {
    for ch in [1usize, 6] {
        let chans = material(ch, 3, 0xfeed_beef ^ ch as u64);
        let opts = PcmOptions {
            rate: PcmRate::Hz176400,
            cutoff: Cutoff::Standard,
            gain_db: 0.0,
            threads: ch > 1,
            filter: dsd2pcm::Filter::Default,
        };
        let (a, _) = ours(&chans, &opts);
        let (b, _) = theirs(&chans, PcmRate::Hz176400, Cutoff::Standard, 0.0);
        assert!(a == b, "{ch} channels differ");
    }
}

const ALL: [(PcmRate, Cutoff); 8] = [
    (PcmRate::Hz88200, Cutoff::Low),
    (PcmRate::Hz88200, Cutoff::Standard),
    (PcmRate::Hz176400, Cutoff::Low),
    (PcmRate::Hz176400, Cutoff::Standard),
    (PcmRate::Hz176400, Cutoff::Wide),
    (PcmRate::Hz352800, Cutoff::Low),
    (PcmRate::Hz352800, Cutoff::Standard),
    (PcmRate::Hz352800, Cutoff::Wide),
];

/// SHA-256 of the output bytes and clip counts for every channel count (1, 2, 6), input
/// (the test material and full scale, all 0xFF) and gain (0, -3 and +7.5 dB, which clips),
/// for one rate, cutoff and filter (the default's output is checked against
/// scarletbook-pcm's as well).
fn digest(rate: PcmRate, cutoff: Cutoff, filter: dsd2pcm::Filter) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for ch in [1usize, 2, 6] {
        let inputs = [
            material(ch, 3, 0x0dd_ba11 ^ ch as u64),
            vec![vec![0xFF; 3 * FRAME]; ch],
        ];
        for chans in &inputs {
            for gain_db in [0.0, -3.0, 7.5] {
                let opts = PcmOptions {
                    rate,
                    cutoff,
                    gain_db,
                    // Both the threaded and the single-thread driver.
                    threads: gain_db != -3.0,
                    filter,
                };
                let (bytes, clipped) = ours(chans, &opts);
                if filter == dsd2pcm::Filter::Default {
                    // The hashed output is scarletbook-pcm's.
                    let (b, cb) = theirs(chans, rate, cutoff, gain_db);
                    assert!(
                        bytes == b && clipped == cb,
                        "{rate:?} {cutoff:?} {gain_db} dB"
                    );
                }
                h.update(&bytes);
                for c in clipped {
                    h.update(c.to_le_bytes());
                }
            }
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The digests of `filter` at every rate and cutoff, at every SIMD level of this CPU (each
/// forced in turn), checked against `golden`.
fn check_golden(filter: dsd2pcm::Filter, golden: [&str; 8]) {
    use dsd2pcm::decimator::{with_simd_level, SimdLevel};
    let levels = SimdLevel::available();
    let got: Vec<Vec<String>> = ALL
        .into_iter()
        .map(|(rate, cutoff)| {
            levels
                .iter()
                .map(|&level| with_simd_level(level, || digest(rate, cutoff, filter)).unwrap())
                .collect()
        })
        .collect();
    for ((rate, cutoff), d) in ALL.into_iter().zip(&got) {
        eprintln!("{filter:?} {rate:?} {cutoff:?}: {d:?}");
    }
    for (((rate, cutoff), d), golden) in ALL.into_iter().zip(&got).zip(golden) {
        for (level, d) in levels.iter().zip(d) {
            assert_eq!(d, golden, "{filter:?} {rate:?} {cutoff:?} at {level:?}");
        }
    }
}

/// Golden output hashes, the same at every SIMD level of this CPU (each forced in turn) and
/// on every platform. They change only with a major version.
#[test]
fn golden_hashes_at_every_simd_level() {
    const GOLDEN: [&str; 8] = [
        "56c126ccdc7ad14f156a408fc41433350ad64799cb029d5c1daf112ea6f2e5e2",
        "fbf0ced5f83210c736a5ac42b4f71bfe7e35f33eb0b65b663ac3c12b0e209348",
        "16887deec03b81bdb74fbde173850d7e20f55c06b6d2b25568b6fc0eb3faa972",
        "f1b7f78e723726b1a0c41f65dc85d2309bf1ff0690189453ebc1d6d23a8607d9",
        "10a6efd0d4a8b4bf68fb6acd15f0bfaa38ce4a6fdb673934c7d41c10702472ee",
        "c2cd259fb52881c6fc74baa11c868fda8cb2bb814ddd54da6cf6717305aa4d51",
        "30de64ebe6a071f889121069306e49b7c8094b9bba4b0c0dd69bbba5214c6be2",
        "14d04a339fd56e168d5566edc241dae92910fde490ef96a92bf89bbf2f2bf40e",
    ];
    check_golden(dsd2pcm::Filter::Default, GOLDEN);
}

/// The precise filter's golden output hashes, as [`golden_hashes_at_every_simd_level`]
/// (computed at the generic, sse2, avx2 and avx2fma levels on x86-64, all equal).
#[test]
fn golden_hashes_precise_at_every_simd_level() {
    const GOLDEN: [&str; 8] = [
        "18acb04ec74f4fe8c1393a9de446e3d29672a2d404d4d1bbd2a707e88ab81bf6",
        "2d0dd8496e57d1ce0215d86c75ad6515fcc8999bef6ab24cfd69d5c583f6aae0",
        "5b98d636b7a8d6adefe8aa0ecf64439e29c8772a0b2ff9a0452cd1f6ff0f4802",
        "cb1691c219858053f0f188ce99f60a1fa6d35f5cdf35a169118ec8e0a1aaa31b",
        "2ff237858fb9f93576afac5f9378adf54bf7f2bb29a1de3189e7841875e1ff65",
        "c33b9582063e2a62009dcf12caf68fc68714d2c4505b82c40a0d25bc29d6d065",
        "bd4a8286a40a6d6ee09c2a16110e47228f694b4bc717efb2dc5c68c2feb7555d",
        "bfeef94a939fc912ffed9819371d4daa3829723dc9957e1207e8b07555e939ba",
    ];
    check_golden(dsd2pcm::Filter::Precise, GOLDEN);
}

/// A real DSD64 file (path in `DSD2PCM_REAL_DSF`), cut to whole 4704-byte frames.
#[test]
#[ignore = "needs DSD2PCM_REAL_DSF"]
fn identical_on_real_file() {
    use dsd2pcm::io::DsdRead;
    let Ok(path) = std::env::var("DSD2PCM_REAL_DSF") else {
        return;
    };
    let mut r =
        DsfReader::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let ch = usize::from(r.info().channels);
    let mut chans = vec![Vec::new(); ch];
    while r.read_bytes(1 << 20, &mut chans).unwrap() > 0 {}
    let frames = chans[0].len() / FRAME;
    for c in &mut chans {
        c.truncate(frames * FRAME);
    }
    for rate in [PcmRate::Hz88200, PcmRate::Hz176400, PcmRate::Hz352800] {
        for cutoff in [Cutoff::Low, Cutoff::Standard, Cutoff::Wide] {
            if cutoff == Cutoff::Wide && rate == PcmRate::Hz88200 {
                continue;
            }
            let opts = PcmOptions {
                rate,
                cutoff,
                gain_db: 0.0,
                threads: true,
                filter: dsd2pcm::Filter::Default,
            };
            let (a, _) = ours(&chans, &opts);
            let (b, _) = theirs(&chans, rate, cutoff, 0.0);
            assert!(a == b, "real file differs: {rate:?} {cutoff:?}");
            eprintln!("{frames} frames x {ch} ch identical: {rate:?} {cutoff:?}");
        }
    }
}
