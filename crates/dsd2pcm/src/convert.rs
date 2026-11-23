//! DSD64 to 24-bit PCM (WAV), sample-identical to scarletbook-pcm 0.7.0.
//!

use crate::decimator::{self, Cutoff, PcmRate, IDLE};
use crate::dff::DffReader;
use crate::dsf::DsfReader;
use crate::error::{Error, Result};
use crate::io::{DsdRead, PcmRead};
use crate::wav::{WavFormat, WavReader, WavWriter};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// DSD to PCM settings.
#[derive(Clone, Copy, Debug)]
pub struct PcmOptions {
    /// Output rate.
    pub rate: PcmRate,
    /// Low-pass filter.
    pub cutoff: Cutoff,
    /// Gain in dB, -120 to +24 (0 = scarletbook-pcm's default).
    pub gain_db: f64,
    /// Convert channels on separate threads.
    pub threads: bool,
}

impl PcmOptions {
    /// Standard cutoff, unity gain.
    pub fn new(rate: PcmRate) -> PcmOptions {
        PcmOptions {
            rate,
            cutoff: Cutoff::Standard,
            gain_db: 0.0,
            threads: true,
        }
    }
}

/// Result of a DSD to PCM conversion.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Input rate (Hz).
    pub in_rate: u32,
    /// Output rate (Hz).
    pub out_rate: u32,
    /// Input duration in seconds.
    pub seconds: f64,
    /// Output samples per channel.
    pub out_samples: u64,
    /// Samples clamped to 24 bits, per channel.
    pub clipped: Vec<u64>,
}

/// An opened input file.
pub enum Input {
    /// DSD input (DSF or DFF).
    Dsd(Box<dyn DsdRead + Send>),
    /// PCM input (WAV).
    Pcm(Box<dyn PcmRead + Send>),
}

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::Dsd(r) => write!(f, "Input::Dsd({:?})", r.info()),
            Input::Pcm(r) => write!(f, "Input::Pcm({:?})", r.info()),
        }
    }
}

/// Opens a DSF, DFF or WAV file, recognised by its first four bytes.
pub fn open_input(path: &Path) -> Result<Input> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    f.seek(SeekFrom::Start(0))?;
    match &magic {
        b"DSD " => Ok(Input::Dsd(Box::new(DsfReader::new(f)?))),
        b"FRM8" => Ok(Input::Dsd(Box::new(DffReader::new(f)?))),
        b"RIFF" => Ok(Input::Pcm(Box::new(WavReader::new(f)?))),
        _ => Err(Error::Format("not a DSF, DFF or WAV file".into())),
    }
}

/// Converts a DSF or DFF file (DSD64) to a 24-bit WAV file.
pub fn convert_file(input: &Path, output: &Path, opts: &PcmOptions) -> Result<Report> {
    let Input::Dsd(r) = open_input(input)? else {
        return Err(Error::Unsupported("input is PCM, not DSD".into()));
    };
    check_options(opts)?;
    if same_file(input, output) {
        return Err(Error::InvalidArgument("output is the input file".into()));
    }
    let info = r.info();
    let data = decimator::output_len(info.samples, opts.rate) * u64::from(info.channels) * 3;
    if 36 + data + (data & 1) > u64::from(u32::MAX) {
        return Err(Error::Unsupported(
            "WAV output over 4 GiB (RF64 is not supported yet)".into(),
        ));
    }
    let out = BufWriter::with_capacity(1 << 20, File::create(output)?);
    let mut w = WavWriter::new(out, WavFormat::I24, r.info().channels, opts.rate.hz())?;
    let rep = dsd64_to_pcm(r, &mut w, opts)?;
    w.finish()?;
    Ok(rep)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Checks the gain range and the rate/cutoff combination.
pub fn check_options(opts: &PcmOptions) -> Result<()> {
    if !opts.gain_db.is_finite() || !(-120.0..=24.0).contains(&opts.gain_db) {
        return Err(Error::InvalidArgument(format!(
            "gain {} dB is outside -120 to +24 dB",
            opts.gain_db
        )));
    }
    decimator::Decimator::new(opts.rate, opts.cutoff).map(|_| ())
}

/// Converts DSD64 to 24-bit PCM, sample-identical to scarletbook-pcm 0.7.0, into `w`.
/// `w` is not finished.
pub fn dsd64_to_pcm<W: Write + Seek>(
    mut r: Box<dyn DsdRead + Send>,
    w: &mut WavWriter<W>,
    opts: &PcmOptions,
) -> Result<Report> {
    check_options(opts)?;
    let gain = decimator::db_to_linear(opts.gain_db);
    let rate = opts.rate;
    let info = r.info();
    if info.sample_rate != decimator::DSD64_RATE {
        return Err(Error::Unsupported(format!(
            "PCM output from {} Hz DSD (only DSD64 so far)",
            info.sample_rate
        )));
    }
    let ch = usize::from(info.channels);
    let n_out = decimator::output_len(info.samples, rate);
    let mut decs = (0..ch)
        .map(|_| decimator::Decimator::new(rate, opts.cutoff))
        .collect::<Result<Vec<_>>>()?;
    let mut clipped = vec![0u64; ch];
    let mut done = 0u64;
    let mut inbuf = vec![Vec::new(); ch];
    let mut eof = false;
    while done < n_out {
        for b in &mut inbuf {
            b.clear();
        }
        let got = if eof {
            0
        } else {
            r.read_bytes(1 << 16, &mut inbuf)?
        };
        if got == 0 {
            eof = true;
            for b in &mut inbuf {
                b.resize(4096, IDLE);
            }
        }
        let want = (n_out - done) as usize;
        let mut outs: Vec<(Vec<i32>, u64)> = vec![(Vec::new(), 0); ch];
        let work = |(d, (inp, o)): ChannelWork<'_>| {
            let mut v = Vec::new();
            d.push(inp, &mut v);
            v.truncate(want);
            o.1 = decimator::quantise(&v, gain, &mut o.0);
        };
        run_channels(
            opts.threads && ch > 1,
            decs.iter_mut().zip(inbuf.iter().zip(outs.iter_mut())),
            work,
        );
        let n = outs[0].0.len();
        let mut bytes = Vec::with_capacity(n * ch * 3);
        for i in 0..n {
            for o in &outs {
                bytes.extend_from_slice(&o.0[i].to_le_bytes()[..3]);
            }
        }
        for (c, o) in outs.iter().enumerate() {
            clipped[c] += o.1;
        }
        w.write_raw(&bytes)?;
        done += n as u64;
    }
    Ok(Report {
        in_rate: info.sample_rate,
        out_rate: rate.hz(),
        seconds: info.samples as f64 / f64::from(info.sample_rate),
        out_samples: n_out,
        clipped,
    })
}

type ChannelWork<'a> = (
    &'a mut decimator::Decimator,
    (&'a Vec<u8>, &'a mut (Vec<i32>, u64)),
);

/// Runs `f` on every item, on scoped threads if `par`.
fn run_channels<I, T, F>(par: bool, items: I, f: F)
where
    I: Iterator<Item = T>,
    T: Send,
    F: Fn(T) + Sync,
{
    if par {
        std::thread::scope(|s| {
            for it in items {
                let f = &f;
                s.spawn(move || f(it));
            }
        });
    } else {
        items.for_each(f);
    }
}
