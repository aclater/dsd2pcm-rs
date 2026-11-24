//! DSD64 to 24-bit PCM (WAV) with the precise filter by default ([`Filter::Precise`]),
//! sample-identical to scarletbook-pcm 0.7.0 with [`Filter::Fast`], or with pysacd's filter
//! ([`Filter::Pysacd`]).
//!

use crate::decimator::{self, Cutoff, PcmRate, Plan, Scratch};
use crate::dff::DffReader;
use crate::dsf::DsfReader;
use crate::error::{Error, Result};
use crate::filters::{pysacd, Filter};
use crate::io::{DsdRead, PcmRead};
use crate::wav::{WavFormat, WavReader, WavWriter};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
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
    /// The filter and quantiser: [`Filter::Precise`] by default; [`Filter::Fast`] is
    /// scarletbook-pcm's.
    pub filter: Filter,
}

impl PcmOptions {
    /// Standard cutoff, unity gain, the default filter ([`Filter::Precise`]).
    pub fn new(rate: PcmRate) -> PcmOptions {
        PcmOptions {
            rate,
            cutoff: Cutoff::Standard,
            gain_db: 0.0,
            threads: true,
            filter: Filter::default(),
        }
    }
}

/// Settings for the output file of [`convert_file_to`]. Start from
/// [`OutputOptions::default`] (replace an existing output, as [`convert_file`] does) and set
/// fields; new fields may be added in minor versions, with defaults that keep the old
/// behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputOptions {
    /// Refuse an existing output instead of replacing it: the conversion fails with
    /// [`Error::Io`] of kind [`std::io::ErrorKind::AlreadyExists`] if anything (a file, a
    /// symbolic link, even a dangling one) is at the output path, before converting or when
    /// the result is put in place, and that file is left as it was. Putting the result in
    /// place never replaces a file that appeared during the conversion (see
    /// [`convert_file_to`]).
    pub no_clobber: bool,
}

impl OutputOptions {
    /// [`OutputOptions::no_clobber`] set to `on`.
    pub fn with_no_clobber(mut self, on: bool) -> OutputOptions {
        self.no_clobber = on;
        self
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

/// Progress of a conversion, passed to the hook of [`dsd64_to_pcm_with`] and
/// [`convert_file_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    /// Output samples per channel written so far.
    pub done: u64,
    /// Output samples per channel in all.
    pub total: u64,
}

/// A progress hook: called on the calling thread at block boundaries (once before the first
/// block, then after each block written); [`ControlFlow::Break`] stops the conversion with
/// [`Error::Cancelled`].
pub type ProgressHook<'a> = &'a mut dyn FnMut(Progress) -> ControlFlow<()>;

/// An opened input file.
pub enum Input {
    /// DSD input (DSF or DFF).
    Dsd(Box<dyn DsdRead + Send>),
    /// PCM input (WAV or FLAC).
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

/// Opens a DSF, DFF, WAV or FLAC file, recognised by its first four bytes (FLAC also
/// after a leading ID3v2 tag).
pub fn open_input(path: &Path) -> Result<Input> {
    let mut f = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    f.seek(SeekFrom::Start(0))?;
    match &magic {
        b"DSD " => Ok(Input::Dsd(Box::new(DsfReader::new(f)?))),
        b"FRM8" => Ok(Input::Dsd(Box::new(DffReader::new(f)?))),
        b"RIFF" => Ok(Input::Pcm(Box::new(WavReader::new(f)?))),
        #[cfg(feature = "flac")]
        b"fLaC" => Ok(Input::Pcm(Box::new(crate::flac::FlacReader::new(f)?))),
        #[cfg(feature = "flac")]
        [b'I', b'D', b'3', _] => Ok(Input::Pcm(Box::new(crate::flac::FlacReader::new(f)?))),
        #[cfg(not(feature = "flac"))]
        b"fLaC" => Err(Error::Unsupported(
            "FLAC input (built without the `flac` feature)".into(),
        )),
        _ => Err(Error::Format("not a DSF, DFF, WAV or FLAC file".into())),
    }
}

/// Converts a DSF or DFF file (DSD64) to a 24-bit WAV file. The output is written to a
/// temporary file and renamed into place on success; see [`convert_file_with`].
pub fn convert_file(input: &Path, output: &Path, opts: &PcmOptions) -> Result<Report> {
    convert_file_with(input, output, opts, &mut |_| ControlFlow::Continue(()))
}

/// [`convert_file`] with a progress hook (see [`ProgressHook`]).
///
/// Before anything is written, the input, its rate and channels, the options and the output
/// size are checked, and so is `output`: a directory (or a path ending in a separator), a
/// device, FIFO or socket, an existing file that is not writable, and the input file itself
/// (by path, symbolic link or hard link) are refused. The WAV is then written to a new
/// temporary file `.dsd2pcm-<pid>-<n>.tmp` in `output`'s directory (created exclusively;
/// creating it also checks that the directory is writable), synced, given the permissions
/// of the file it replaces, and renamed over `output` only when the conversion succeeds; the
/// directory is then synced too (Unix, best effort). On any error, including
/// [`Error::Cancelled`], the temporary file is removed and an existing `output` is left as
/// it was. A symbolic link at `output` is replaced by the new file, not followed. The owner,
/// ACLs and extended attributes of a replaced file are not kept.
pub fn convert_file_with(
    input: &Path,
    output: &Path,
    opts: &PcmOptions,
    progress: ProgressHook<'_>,
) -> Result<Report> {
    convert_file_to(input, output, opts, &OutputOptions::default(), progress)
}

/// [`convert_file_with`] with settings for the output file ([`OutputOptions`]).
///
/// With [`OutputOptions::no_clobber`], an existing output is refused before converting, and
/// the result is put in place without ever replacing a file: it is hard-linked to the
/// output path, which fails if anything is there by then, and the temporary name is
/// removed. Where the file system has no hard links (FAT and exFAT, some network and FUSE
/// file systems), the output path is first created empty with an exclusive create, which
/// fails the same way, and the result is renamed over that empty file. That fallback is
/// best effort: a crash at that moment can leave an empty output, and a file another
/// process puts there in that moment is replaced. A refusal is [`Error::Io`] of kind
/// [`std::io::ErrorKind::AlreadyExists`]; the temporary file is removed and the existing
/// file is left as it was.
pub fn convert_file_to(
    input: &Path,
    output: &Path,
    opts: &PcmOptions,
    out_opts: &OutputOptions,
    progress: ProgressHook<'_>,
) -> Result<Report> {
    let Input::Dsd(r) = open_input(input)? else {
        return Err(Error::Unsupported("input is PCM, not DSD".into()));
    };
    check_options(opts)?;
    let info = r.info();
    check_dsd64(info.sample_rate)?;
    let replaced = check_output(output, out_opts.no_clobber)?;
    if same_file(input, output) {
        return Err(Error::InvalidArgument("output is the input file".into()));
    }
    let data = decimator::output_len(info.samples, opts.rate) * u64::from(info.channels) * 3;
    // The RIFF size: everything after its own 8 bytes.
    let header = crate::wav::header_len(WavFormat::I24, info.channels);
    if header - 8 + data + (data & 1) > u64::from(u32::MAX) {
        return Err(Error::Unsupported(
            "WAV output over 4 GiB (RF64 is not supported yet)".into(),
        ));
    }
    let mut tmp = TempFile::create(output, replaced.is_some())?;
    let file = tmp.file.take().expect("a new temporary file is open");
    let out = BufWriter::with_capacity(1 << 20, file);
    // The speakers the input declares (DSF channel type, DSDIFF channel IDs), else those
    // of the channel count.
    let mask = r
        .channel_mask()
        .unwrap_or_else(|| crate::wav::channel_mask(info.channels));
    let mut w =
        WavWriter::with_channel_mask(out, WavFormat::I24, info.channels, opts.rate.hz(), mask)?;
    let rep = dsd64_to_pcm_with(r, &mut w, opts, progress)?;
    w.finish()?;
    let file = w.into_inner().into_inner().map_err(|e| e.into_error())?;
    if let Some(perms) = replaced {
        file.set_permissions(perms)?;
    }
    file.sync_all()?;
    drop(file);
    if out_opts.no_clobber {
        tmp.persist_new(output, &|a, b| std::fs::hard_link(a, b))?;
    } else {
        tmp.persist(output)?;
    }
    Ok(rep)
}

/// The error for an output that exists when it must not be replaced.
fn exists_error() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "the output exists; not replacing it (no-clobber)",
    ))
}

/// Checks the output path before converting. Returns the permissions of the regular file it
/// would replace, or `None` when there is none (or it is a symbolic link, which is replaced).
/// With `no_clobber`, anything at the path is refused.
fn check_output(output: &Path, no_clobber: bool) -> Result<Option<std::fs::Permissions>> {
    let ends_in_separator = output
        .as_os_str()
        .as_encoded_bytes()
        .last()
        .is_some_and(|&b| std::path::is_separator(char::from(b)));
    if ends_in_separator || output.file_name().is_none() {
        return Err(Error::InvalidArgument(
            "output path names a directory, not a file".into(),
        ));
    }
    let meta = match std::fs::symlink_metadata(output) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let kind = meta.file_type();
    if kind.is_dir() {
        return Err(Error::InvalidArgument("output is a directory".into()));
    }
    if no_clobber {
        return Err(exists_error());
    }
    if kind.is_symlink() {
        return Ok(None);
    }
    if !kind.is_file() {
        return Err(Error::InvalidArgument(
            "output exists and is not a regular file (a device, FIFO or socket); \
             refusing to replace it"
                .into(),
        ));
    }
    // Opening for writing (without truncating) changes nothing; it fails as writing would.
    if let Err(e) = OpenOptions::new().write(true).open(output) {
        return Err(Error::Io(std::io::Error::new(
            e.kind(),
            format!("the existing output is not writable: {e}"),
        )));
    }
    Ok(Some(meta.permissions()))
}

/// A temporary output file next to its target, removed on drop unless persisted.
struct TempFile {
    path: Option<PathBuf>,
    file: Option<File>,
}

impl TempFile {
    /// Creates `.dsd2pcm-<pid>-<n>.tmp` in `target`'s directory, failing if it exists. With
    /// `private`, it is created readable and writable by the owner only (Unix), until it is
    /// given the permissions of the file it replaces.
    fn create(target: &Path, private: bool) -> Result<TempFile> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = match target.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        let mut last = None;
        for _ in 0..100 {
            let name = format!(
                ".dsd2pcm-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = dir.join(name);
            let mut o = OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            if private {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            #[cfg(not(unix))]
            let _ = private;
            match o.open(&path) {
                Ok(file) => {
                    return Ok(TempFile {
                        path: Some(path),
                        file: Some(file),
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e.into()),
            }
        }
        Err(last
            .unwrap_or_else(|| std::io::ErrorKind::AlreadyExists.into())
            .into())
    }

    /// Renames the file over `target`, then syncs the directory (Unix, best effort: some
    /// file systems do not support it).
    fn persist(mut self, target: &Path) -> Result<()> {
        let path = self.path.take().expect("not yet persisted");
        if let Err(e) = std::fs::rename(&path, target) {
            let _ = std::fs::remove_file(&path);
            return Err(e.into());
        }
        #[cfg(unix)]
        if let Some(dir) = path.parent() {
            let _ = File::open(dir).and_then(|d| d.sync_all());
        }
        Ok(())
    }
}

impl TempFile {
    /// Puts the file at `target` only if nothing is there, atomically: `link` (a hard link,
    /// failing with `AlreadyExists` if `target` exists) and then removing the temporary
    /// name. If `link` fails otherwise (no hard links on this file system), `target` is
    /// created empty and exclusively, failing the same way, and the file is renamed over
    /// it. Then the directory is synced (Unix, best effort).
    fn persist_new(
        mut self,
        target: &Path,
        link: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<()> {
        let path = self.path.take().expect("not yet persisted");
        // From here on, `path` is removed on every return.
        let done = (|| match link(&path, target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(exists_error()),
            Err(_) => {
                match OpenOptions::new().write(true).create_new(true).open(target) {
                    Ok(f) => drop(f),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        return Err(exists_error())
                    }
                    Err(e) => return Err(e.into()),
                }
                std::fs::rename(&path, target).map_err(|e| {
                    // Remove the empty file this call created, unless it has been
                    // written to since.
                    if std::fs::symlink_metadata(target).is_ok_and(|m| m.is_file() && m.len() == 0)
                    {
                        let _ = std::fs::remove_file(target);
                    }
                    e.into()
                })
            }
        })();
        let _ = std::fs::remove_file(&path);
        done?;
        #[cfg(unix)]
        if let Some(dir) = path.parent() {
            let _ = File::open(dir).and_then(|d| d.sync_all());
        }
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        drop(self.file.take());
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Whether `a` and `b` name the same file. On Unix this compares device and inode numbers,
/// so hard links are caught as well; elsewhere it compares canonical paths (hard links are
/// then not caught, but the output is still written to a new file and renamed into place,
/// which leaves the input as it was).
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn check_dsd64(rate: u32) -> Result<()> {
    if rate != decimator::DSD64_RATE {
        return Err(Error::Unsupported(format!(
            "PCM output from {rate} Hz DSD (only DSD64 so far)"
        )));
    }
    Ok(())
}

/// Checks the gain range and the rate, cutoff and filter combination.
pub fn check_options(opts: &PcmOptions) -> Result<()> {
    if !opts.gain_db.is_finite() || !(-120.0..=24.0).contains(&opts.gain_db) {
        return Err(Error::InvalidArgument(format!(
            "gain {} dB is outside -120 to +24 dB",
            opts.gain_db
        )));
    }
    match opts.filter {
        Filter::Pysacd => pysacd::Pysacd::new(opts.rate, opts.cutoff).map(|_| ()),
        f => Plan::with_design(opts.rate, opts.cutoff, f.design()).map(|_| ()),
    }
}

/// Converts DSD64 to 24-bit PCM with `opts.filter` into `w` (sample-identical to
/// scarletbook-pcm 0.7.0 with [`Filter::Fast`], to pysacd's reference with
/// [`Filter::Pysacd`]). `w` is not finished.
///
/// Every output sample depends only on a fixed window of input bytes, so each block of
/// input is cut into pieces (per channel and per range of outputs) that run on all cores;
/// the output does not depend on how the work is split.
pub fn dsd64_to_pcm<W: Write + Seek>(
    r: Box<dyn DsdRead + Send>,
    w: &mut WavWriter<W>,
    opts: &PcmOptions,
) -> Result<Report> {
    dsd64_to_pcm_with(r, w, opts, &mut |_| ControlFlow::Continue(()))
}

/// [`dsd64_to_pcm`] with a progress hook (see [`ProgressHook`]). The output is the same;
/// after a [`Error::Cancelled`], `w` holds the blocks written so far.
pub fn dsd64_to_pcm_with<W: Write + Seek>(
    mut r: Box<dyn DsdRead + Send>,
    w: &mut WavWriter<W>,
    opts: &PcmOptions,
    progress: ProgressHook<'_>,
) -> Result<Report> {
    check_options(opts)?;
    let info = r.info();
    check_dsd64(info.sample_rate)?;
    if opts.filter == Filter::Pysacd {
        return pysacd::dsd64_to_pcm(r, w, opts, progress);
    }
    let gain = decimator::db_to_linear(opts.gain_db);
    let rate = opts.rate;
    let plan = Plan::with_design(rate, opts.cutoff, opts.filter.design())?;
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
    report(progress, 0, n_out)?;
    while done < n_out {
        let got = if eof {
            0
        } else {
            r.read_bytes(chunk, &mut bufs)?
        };
        let end = byte_start + bufs[0].len() as i64;
        if got == 0 && !eof && (end as u64).saturating_mul(8) < info.samples {
            // A source that ends before the samples it declared: refuse, instead of
            // converting the missing part as idle in one unbounded batch.
            return Err(Error::Format(format!(
                "input ended after {end} of {} bytes per channel",
                info.samples.div_ceil(8)
            )));
        }
        eof |= got == 0;
        let o0 = done as i64;
        // At the end of the input, the rest is idle: every remaining output is ready.
        let ready = if eof {
            left(n_out, done).next_multiple_of(2)
        } else {
            plan.ready(o0, end)
        };
        if ready == 0 {
            continue;
        }
        let keep = left(n_out, done).min(ready);
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
        report(progress, done, n_out)?;
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

/// Outputs still to write, saturated to an even `usize` (on 32-bit targets a plain cast
/// could truncate the count to 0).
fn left(n_out: u64, done: u64) -> usize {
    usize::try_from(n_out - done).unwrap_or(usize::MAX - 1)
}

/// Calls the hook at a block boundary of a conversion driver: [`Error::Cancelled`] on
/// `Break`.
pub(crate) fn report(progress: ProgressHook<'_>, done: u64, total: u64) -> Result<()> {
    match progress(Progress { done, total }) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(Error::Cancelled),
    }
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

/// Runs `f` on every item, on up to `scratch.len()` threads (one scratch each, the calling
/// thread being one of them), and returns the results in item order.
///
/// A worker thread that cannot be started (the system's thread or memory limit) is not an
/// error: the items are shared out as they are taken, so the threads that run, at least the
/// calling one, take the rest, and the results are the same.
pub(crate) fn run_pieces<T: Sync, R: Send, S: Send>(
    items: &[T],
    scratch: &mut [S],
    f: impl Fn(&T, &mut S) -> R + Sync,
) -> Vec<R> {
    run_pieces_with(items, scratch, f, &|_| Ok(std::thread::Builder::new()))
}

/// [`run_pieces`] with the thread builder for worker `i` (1, 2, ...) from `builder`; an
/// error from it, or from its `spawn_scoped`, ends the spawning (tests inject one).
fn run_pieces_with<T: Sync, R: Send, S: Send>(
    items: &[T],
    scratch: &mut [S],
    f: impl Fn(&T, &mut S) -> R + Sync,
    builder: &dyn Fn(usize) -> std::io::Result<std::thread::Builder>,
) -> Vec<R> {
    let workers = scratch.len().min(items.len());
    if workers <= 1 {
        return items.iter().map(|it| f(it, &mut scratch[0])).collect();
    }
    let next = AtomicUsize::new(0);
    let work = |sc: &mut S| {
        let mut mine = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(it) = items.get(i) else { break };
            mine.push((i, f(it, sc)));
        }
        mine
    };
    let (first, rest) = scratch[..workers]
        .split_first_mut()
        .expect("at least two workers");
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let work = &work;
        let mut handles = Vec::with_capacity(rest.len());
        for (i, sc) in rest.iter_mut().enumerate() {
            match builder(i + 1).and_then(|b| b.spawn_scoped(s, move || work(sc))) {
                Ok(h) => handles.push(h),
                // Out of threads: the ones running (and this one) take the rest.
                Err(_) => break,
            }
        }
        let mut parts = vec![work(first)];
        parts.extend(
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))),
        );
        parts
    });
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    for (i, r) in parts.into_iter().flatten() {
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};
    use std::thread::{Builder, ThreadId};

    /// Runs 1000 items on 8 scratch slots while `builder` refuses workers from `fail_from` on;
    /// returns the results and the threads that ran items.
    fn run(fail_from: usize) -> (Vec<u64>, Vec<ThreadId>) {
        let items: Vec<u64> = (0..1000).collect();
        let mut scratch = vec![0u64; 8];
        let threads = Mutex::new(Vec::new());
        let builder = |i: usize| {
            if i >= fail_from {
                Err(IoError::new(ErrorKind::WouldBlock, "no more threads"))
            } else {
                Ok(Builder::new())
            }
        };
        let out = run_pieces_with(
            &items,
            &mut scratch,
            |&x, sc| {
                *sc += 1;
                let mut t = threads.lock().unwrap();
                let id = std::thread::current().id();
                if !t.contains(&id) {
                    t.push(id);
                }
                x * x + 1
            },
            &builder,
        );
        // Every item ran once, on some scratch slot.
        assert_eq!(scratch.iter().sum::<u64>(), 1000);
        (out, threads.into_inner().unwrap())
    }

    #[test]
    fn spawn_failure_falls_back_to_fewer_threads() {
        let want: Vec<u64> = (0..1000u64).map(|x| x * x + 1).collect();
        // No worker starts: everything runs on the calling thread.
        let (out, threads) = run(1);
        assert_eq!(out, want);
        assert_eq!(threads, [std::thread::current().id()]);
        // Two of the seven workers start, then spawning fails: same results.
        let (out, threads) = run(3);
        assert_eq!(out, want);
        assert!(threads.len() <= 3);
        // All start.
        let (out, _) = run(usize::MAX);
        assert_eq!(out, want);
    }

    fn dir(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/unit-tmp")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn temp(target: &Path, body: &[u8]) -> TempFile {
        let mut t = TempFile::create(target, false).unwrap();
        t.file.take().unwrap().write_all(body).unwrap();
        t
    }

    fn names(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// A file system without hard links: the exclusive-create fallback puts a new file in
    /// place and refuses an existing one, leaving no temporary file either way.
    #[test]
    #[cfg_attr(miri, ignore = "file I/O")]
    fn persist_new_without_hard_links() {
        let d = dir("persist-new-fallback");
        let out = d.join("out.wav");
        let no_links = |_: &Path, _: &Path| -> std::io::Result<()> {
            Err(IoError::new(ErrorKind::Unsupported, "no hard links"))
        };
        temp(&out, b"new").persist_new(&out, &no_links).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert_eq!(names(&d), ["out.wav"]);
        let e = temp(&out, b"newer")
            .persist_new(&out, &no_links)
            .unwrap_err();
        assert!(
            matches!(&e, Error::Io(e) if e.kind() == ErrorKind::AlreadyExists),
            "{e}"
        );
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert_eq!(names(&d), ["out.wav"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// With hard links: a new file goes in place, an existing one (here a dangling symbolic
    /// link on Unix, or a file) is refused and left as it was.
    #[test]
    #[cfg_attr(miri, ignore = "file I/O")]
    fn persist_new_with_hard_links() {
        let d = dir("persist-new-link");
        let out = d.join("out.wav");
        let link = |a: &Path, b: &Path| std::fs::hard_link(a, b);
        temp(&out, b"new").persist_new(&out, &link).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert_eq!(names(&d), ["out.wav"]);
        let e = temp(&out, b"newer").persist_new(&out, &link).unwrap_err();
        assert!(
            matches!(&e, Error::Io(e) if e.kind() == ErrorKind::AlreadyExists),
            "{e}"
        );
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        #[cfg(unix)]
        {
            let dangling = d.join("dangling.wav");
            std::os::unix::fs::symlink(d.join("nowhere"), &dangling).unwrap();
            let e = temp(&dangling, b"x")
                .persist_new(&dangling, &link)
                .unwrap_err();
            assert!(
                matches!(&e, Error::Io(e) if e.kind() == ErrorKind::AlreadyExists),
                "{e}"
            );
            assert!(!d.join("nowhere").exists(), "the link is not followed");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
