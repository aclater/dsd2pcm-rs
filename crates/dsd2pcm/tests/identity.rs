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
    let bytes = wav.into_inner().into_inner();
    (bytes[44..].to_vec(), rep.clipped)
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
        };
        let (a, _) = ours(&chans, &opts);
        let (b, _) = theirs(&chans, PcmRate::Hz176400, Cutoff::Standard, 0.0);
        assert!(a == b, "{ch} channels differ");
    }
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
            };
            let (a, _) = ours(&chans, &opts);
            let (b, _) = theirs(&chans, rate, cutoff, 0.0);
            assert!(a == b, "real file differs: {rate:?} {cutoff:?}");
            eprintln!("{frames} frames x {ch} ch identical: {rate:?} {cutoff:?}");
        }
    }
}
