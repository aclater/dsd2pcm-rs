//! DSD64 to 24-bit PCM (WAV), sample-identical to scarletbook-pcm 0.7.0.
//!

use crate::decimator::{self, Cutoff, PcmRate, Plan, Scratch};
use crate::dff::DffReader;
use crate::dsf::DsfReader;
use crate::error::{Error, Result};
use crate::io::{DsdRead, PcmRead};
use crate::wav::{WavFormat, WavReader, WavWriter};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

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
    Plan::new(opts.rate, opts.cutoff).map(|_| ())
}

/// Converts DSD64 to 24-bit PCM, sample-identical to scarletbook-pcm 0.7.0, into `w`.
/// `w` is not finished.
///
/// Every output sample depends only on a fixed window of input bytes, so each block of
/// input is cut into pieces (per channel and per range of outputs) that run on all cores;
/// the output does not depend on how the work is split.
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
    let plan = Plan::new(rate, opts.cutoff)?;
    let ch = usize::from(info.channels);
    let n_out = decimator::output_len(info.samples, rate);
    let workers = if opts.threads {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        1
    };
    let chunk = if workers > 1 { CHUNK } else { CHUNK_1 };
    let mut scratch: Vec<Scratch> = (0..workers).map(|_| Scratch::default()).collect();
    let mut clipped = vec![0u64; ch];
    // Each channel's bytes from `byte_start` on.
    let mut bufs = vec![Vec::new(); ch];
    let mut byte_start = 0i64;
    let mut done = 0u64;
    let mut eof = false;
    // The 24-bit samples of a block, channel by channel, and interleaved as bytes; reused.
    let mut q: Vec<i32> = Vec::new();
    let mut bytes = Vec::new();
    while done < n_out {
        let got = if eof {
            0
        } else {
            r.read_bytes(chunk, &mut bufs)?
        };
        eof |= got == 0;
        let end = byte_start + bufs[0].len() as i64;
        let o0 = done as i64;
        // At the end of the input, the rest is idle: every remaining output is ready.
        let ready = if eof {
            ((n_out - done) as usize).next_multiple_of(2)
        } else {
            plan.ready(o0, end)
        };
        if ready == 0 {
            continue;
        }
        let keep = ((n_out - done) as usize).min(ready);
        // Pieces: an even number of outputs each, about PIECE bytes of input, each writing
        // its samples straight into its part of `q` (channel `c` at `c keep ..`).
        let piece = ((PIECE / plan_k(rate)) as usize).max(2);
        q.resize(keep * ch, 0);
        let pieces: Vec<(usize, i64, Mutex<&mut [i32]>)> = q
            .chunks_exact_mut(keep)
            .enumerate()
            .flat_map(|(c, qc)| {
                qc.chunks_mut(piece)
                    .enumerate()
                    .map(move |(i, part)| (c, o0 + (i * piece) as i64, Mutex::new(part)))
            })
            .collect();
        let results = run_pieces(&pieces, &mut scratch, |(c, a, part), sc| {
            let mut part = part.lock().unwrap_or_else(|e| e.into_inner());
            plan.run_quantised(&bufs[*c], byte_start, *a, gain, sc, &mut part)
        });
        for (&(c, _, _), clip) in pieces.iter().zip(results) {
            clipped[c] += clip;
        }
        drop(pieces);
        let per_ch: Vec<&[i32]> = q.chunks_exact(keep).collect();
        bytes.resize(keep * ch * 3, 0);
        decimator::pack24(plan.isa(), &per_ch, &mut bytes);
        w.write_raw(&bytes)?;
        done += keep as u64;
        // Drop the bytes no later output needs.
        let first = plan.first_byte(done as i64);
        let drop = ((first - byte_start).max(0) as usize).min(bufs[0].len());
        for b in &mut bufs {
            b.drain(..drop);
        }
        byte_start += drop as i64;
    }
    Ok(Report {
        in_rate: info.sample_rate,
        out_rate: rate.hz(),
        seconds: info.samples as f64 / f64::from(info.sample_rate),
        out_samples: n_out,
        clipped,
    })
}

/// Input bytes per channel read at a time.
const CHUNK: usize = 1 << 20;
/// Input bytes per channel read at a time on one thread: the block stays in the cache.
const CHUNK_1: usize = 64 << 10;
/// Input bytes per piece of work.
const PIECE: i64 = 64 << 10;

fn plan_k(rate: PcmRate) -> i64 {
    (rate.decimation() / 8) as i64
}

/// Runs `f` on every item, on up to `scratch.len()` threads (one scratch each), and
/// returns the results in item order.
fn run_pieces<T: Sync, R: Send>(
    items: &[T],
    scratch: &mut [Scratch],
    f: impl Fn(&T, &mut Scratch) -> R + Sync,
) -> Vec<R> {
    let workers = scratch.len().min(items.len());
    if workers <= 1 {
        return items.iter().map(|it| f(it, &mut scratch[0])).collect();
    }
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let handles: Vec<_> = scratch[..workers]
            .iter_mut()
            .map(|sc| {
                let (next, f) = (&next, &f);
                s.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(it) = items.get(i) else { break };
                        mine.push((i, f(it, sc)));
                    }
                    mine
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker panicked"))
            .collect()
    });
    for (i, r) in parts.into_iter().flatten() {
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}
