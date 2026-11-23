// SPDX-License-Identifier: MIT OR Apache-2.0
//! Super Audio CD frame runs to 24-bit PCM, with the interface and the output bytes of
//! scarletbook-pcm 0.7.0's `Converter::convert`.
//!
//! Portions Copyright (c) 2026 The scarletbook-rs authors, MIT OR Apache-2.0 (see
//! PROVENANCE.md): the public types and their semantics (`Converter`, `Run`, `FrameRun`,
//! `Frames`, `PcmOut`, `PcmConfig`, `Gain`, `ConvertStats`, `pack_s24le`) follow
//! scarletbook-pcm 0.7.0 `src/lib.rs`. The conversion is this crate's decimator.
//!
//! The input is DSD64 in Super Audio CD frames: [`FRAME_BYTES_PER_CHANNEL`] bytes per channel
//! per frame (1/75 s), channels interleaved byte by byte (byte `k C + c` is byte `k` of
//! channel `c`), the earliest sample in the most significant bit, 1 = +1. The output is
//! 24-bit PCM, interleaved, as sign-extended `i32` or packed 3-byte little-endian
//! ([`PcmOut`]).
//!
//! Every output sample is a function of the input bits around it, so converting frame `f`
//! with frames `f - 1` and `f + 1` as neighbours gives exactly the bytes of frame `f` in a
//! conversion of the whole area, and so does any run of frames with its two neighbours,
//! whatever the run length or buffer layout. A neighbour of `None` marks the edge of the
//! area: the input beyond it is the idle pattern (0x69). The samples are those of
//! scarletbook-pcm 0.7.0 and of [`crate::convert::dsd64_to_pcm`].
//!
//! ```
//! use dsd2pcm::frames::{Converter, PcmConfig, PcmOut, Run, FRAME_BYTES_PER_CHANNEL};
//! use dsd2pcm::{Cutoff, PcmRate};
//!
//! let mut conv = Converter::new(PcmConfig::new(PcmRate::Hz176400, Cutoff::Standard, 2))?;
//! // Frames 1 and 2 of an area, each in its own buffer, with frames 0 and 3 around them.
//! let area: Vec<Vec<u8>> = (0..4).map(|_| vec![0x69u8; 2 * FRAME_BYTES_PER_CHANNEL]).collect();
//! let frames = [&area[1][..], &area[2][..]];
//! let run = Run::new(&frames[..]).with_before(Some(&area[0])).with_after(Some(&area[3]));
//! let mut wav = vec![0u8; 2 * conv.samples_per_frame() * 2 * 3];
//! let mut clipped = [0u64; 2];
//! let stats = conv.convert(run, PcmOut::S24le(&mut wav), Some(&mut clipped))?;
//! assert_eq!(clipped.iter().sum::<u64>(), stats.clipped_total());
//! assert!(wav.iter().all(|&b| b == 0));
//! # Ok::<(), dsd2pcm::Error>(())
//! ```
//!
//! With [`crate::Filter::Pysacd`] in [`PcmConfig`], the samples are those of pysacd's
//! filter instead, which also depend on the run's position in the area: give it with
//! [`Run::at_frame`]. A run with no frame before it starts the area.
//!
//! A [`Converter`] converts one call at a time on the calling thread (`&mut self`); use one
//! per worker thread. Its buffers hold a few frames of every channel, grow on the first
//! call and are reused: later calls allocate nothing.

use crate::decimator::{self, Cutoff, PcmRate, Plan, Scratch, IDLE};
use crate::error::{Error, Result};
use crate::filters::{pysacd, Filter};

/// Bytes of one channel in one frame: 37 632 one-bit samples.
pub const FRAME_BYTES_PER_CHANNEL: usize = 4704;

/// The highest channel count of an area.
pub const MAX_CHANNELS: u8 = 6;

/// Frames converted per block: the block's bytes of one channel and its intermediate
/// samples stay in the L2 cache, and each block is one pass of the decimator per channel.
const BLOCK_FRAMES: usize = 6;

/// A gain applied before rounding, from -120 to +24 dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gain {
    db: f64,
    linear: f64,
}

impl Gain {
    /// 0 dB.
    pub const UNITY: Gain = Gain {
        db: 0.0,
        linear: 1.0,
    };
    /// The lowest gain, in dB.
    pub const MIN_DB: f64 = -120.0;
    /// The highest gain, in dB.
    pub const MAX_DB: f64 = 24.0;

    /// The gain of `db` decibels (amplitude): [`decimator::db_to_linear`], the same factor
    /// on every platform.
    pub fn from_db(db: f64) -> Result<Gain> {
        if !db.is_finite() || !(Self::MIN_DB..=Self::MAX_DB).contains(&db) {
            return Err(Error::InvalidArgument(format!(
                "gain {db} dB is outside -120 to +24 dB"
            )));
        }
        Ok(Gain {
            db,
            linear: decimator::db_to_linear(db),
        })
    }

    /// The gain in dB, as given.
    pub fn db(self) -> f64 {
        self.db
    }

    /// The linear factor.
    pub fn linear(self) -> f64 {
        self.linear
    }
}

impl Default for Gain {
    fn default() -> Gain {
        Gain::UNITY
    }
}

/// What a [`Converter`] produces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PcmConfig {
    /// The output rate.
    pub rate: PcmRate,
    /// The low-pass filter.
    pub cutoff: Cutoff,
    /// Channels of the input frames, 1 to 6.
    pub channels: u8,
    /// The gain; [`Gain::UNITY`] by default.
    pub gain: Gain,
    /// The filter and quantiser; [`Filter::Default`] (scarletbook-pcm's) by default. With
    /// [`Filter::Pysacd`] the output also depends on each run's position in the area
    /// ([`Run::at_frame`]).
    pub filter: Filter,
}

impl PcmConfig {
    /// A configuration with unity gain.
    pub fn new(rate: PcmRate, cutoff: Cutoff, channels: u8) -> PcmConfig {
        PcmConfig {
            rate,
            cutoff,
            channels,
            gain: Gain::UNITY,
            filter: Filter::Default,
        }
    }

    /// The same configuration with gain `gain`.
    pub fn with_gain(self, gain: Gain) -> PcmConfig {
        PcmConfig { gain, ..self }
    }

    /// The same configuration with filter `filter`.
    pub fn with_filter(self, filter: Filter) -> PcmConfig {
        PcmConfig { filter, ..self }
    }
}

/// Counts from one conversion call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvertStats {
    /// Samples clamped to the 24-bit range, per channel.
    pub clipped: [u64; MAX_CHANNELS as usize],
}

impl ConvertStats {
    /// Clamped samples over all channels.
    pub fn clipped_total(&self) -> u64 {
        self.clipped.iter().sum()
    }
}

/// A run of consecutive frames of one area in one buffer, and the frames around it.
#[derive(Clone, Copy, Debug)]
pub struct FrameRun<'a> {
    /// The frame before the run, or `None` if the run starts the area.
    pub before: Option<&'a [u8]>,
    /// The frames to convert, `C x 4704` bytes each, interleaved.
    pub frames: &'a [u8],
    /// The frame after the run, or `None` if the run ends the area.
    pub after: Option<&'a [u8]>,
}

impl<'a> FrameRun<'a> {
    /// The run `frames`, with no frame before or after it: a whole area.
    pub fn new(frames: &'a [u8]) -> FrameRun<'a> {
        FrameRun {
            before: None,
            frames,
            after: None,
        }
    }

    /// The same run with the frame before it, `None` if the run starts the area.
    pub fn with_before(self, before: Option<&'a [u8]>) -> FrameRun<'a> {
        FrameRun { before, ..self }
    }

    /// The same run with the frame after it, `None` if the run ends the area.
    pub fn with_after(self, after: Option<&'a [u8]>) -> FrameRun<'a> {
        FrameRun { after, ..self }
    }
}

/// The frames of a [`Run`], each `C x 4704` bytes, interleaved.
#[derive(Clone, Copy, Debug)]
pub enum Frames<'a> {
    /// Frames back to back in one buffer (a whole number of frames).
    Contiguous(&'a [u8]),
    /// One buffer per frame, each exactly one frame, read in place.
    Separate(&'a [&'a [u8]]),
}

impl<'a> From<&'a [u8]> for Frames<'a> {
    fn from(frames: &'a [u8]) -> Frames<'a> {
        Frames::Contiguous(frames)
    }
}

impl<'a> From<&'a [&'a [u8]]> for Frames<'a> {
    fn from(frames: &'a [&'a [u8]]) -> Frames<'a> {
        Frames::Separate(frames)
    }
}

/// A run of consecutive frames of one area and the frames around it, for
/// [`Converter::convert`].
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    /// The frame before the run, or `None` if the run starts the area.
    pub before: Option<&'a [u8]>,
    /// The frames to convert.
    pub frames: Frames<'a>,
    /// The frame after the run, or `None` if the run ends the area.
    pub after: Option<&'a [u8]>,
    /// The index in the area of the run's first frame (0 = the area's first frame). Only
    /// [`Filter::Pysacd`] uses it: its dither is seeded per frame from this index.
    pub first_frame: u64,
}

impl<'a> Run<'a> {
    /// The run `frames` (`&[u8]` or `&[&[u8]]`), with no frame before or after it: a whole
    /// area, starting at frame 0.
    pub fn new(frames: impl Into<Frames<'a>>) -> Run<'a> {
        Run {
            before: None,
            frames: frames.into(),
            after: None,
            first_frame: 0,
        }
    }

    /// The same run with its first frame at index `frame` of the area.
    pub fn at_frame(self, frame: u64) -> Run<'a> {
        Run {
            first_frame: frame,
            ..self
        }
    }

    /// The same run with the frame before it, `None` if the run starts the area.
    pub fn with_before(self, before: Option<&'a [u8]>) -> Run<'a> {
        Run { before, ..self }
    }

    /// The same run with the frame after it, `None` if the run ends the area.
    pub fn with_after(self, after: Option<&'a [u8]>) -> Run<'a> {
        Run { after, ..self }
    }

    /// Frame `i` of the run (`i` below the frame count).
    fn frame(&self, i: usize, frame: usize) -> &'a [u8] {
        match self.frames {
            Frames::Contiguous(b) => &b[i * frame..][..frame],
            Frames::Separate(s) => s[i],
        }
    }
}

impl<'a> From<FrameRun<'a>> for Run<'a> {
    fn from(run: FrameRun<'a>) -> Run<'a> {
        Run {
            before: run.before,
            frames: Frames::Contiguous(run.frames),
            after: run.after,
            first_frame: 0,
        }
    }
}

/// The output buffer of [`Converter::convert`] and its sample layout. Either holds
/// `frames x samples_per_frame() x channels` interleaved samples.
#[derive(Debug)]
pub enum PcmOut<'o> {
    /// Sign-extended `i32` samples.
    I32(&'o mut [i32]),
    /// 3-byte little-endian samples (24-bit WAV and FLAC): the bytes of [`pack_s24le`] of
    /// the `I32` output.
    S24le(&'o mut [u8]),
}

/// A converter for one stream: one per worker thread.
#[derive(Debug)]
pub struct Converter {
    config: PcmConfig,
    plan: Plan,
    sc: Scratch,
    /// One block's bytes per channel, `span` each, back to back.
    bytes: Vec<u8>,
    /// One block's samples per channel, `BLOCK_FRAMES x samples_per_frame` each.
    q: Vec<i32>,
    /// The state of [`Filter::Pysacd`].
    pysacd: Option<Box<pysacd::FrameState>>,
}

impl Converter {
    /// A converter for `config`: 1 to 6 channels, a cutoff the rate has (`Wide` is not
    /// available at 88.2 kHz), a gain made by [`Gain::from_db`].
    pub fn new(config: PcmConfig) -> Result<Converter> {
        if !(1..=MAX_CHANNELS).contains(&config.channels) {
            return Err(Error::InvalidArgument(format!(
                "{} channels: 1 to 6 are supported",
                config.channels
            )));
        }
        if !config.gain.linear.is_finite()
            || !(Gain::MIN_DB..=Gain::MAX_DB).contains(&config.gain.db)
        {
            return Err(Error::InvalidArgument(format!(
                "gain {} dB is outside -120 to +24 dB",
                config.gain.db
            )));
        }
        let pysacd = match config.filter {
            Filter::Pysacd => Some(Box::new(pysacd::FrameState::new(
                config.rate,
                config.cutoff,
                config.gain.db,
            )?)),
            _ => None,
        };
        let plan = Plan::new(config.rate, config.cutoff)?;
        Ok(Converter {
            config,
            plan,
            sc: Scratch::default(),
            bytes: Vec::new(),
            q: Vec::new(),
            pysacd,
        })
    }

    /// The configuration.
    pub fn config(&self) -> PcmConfig {
        self.config
    }

    /// Output samples per channel per frame: 1176, 2352 or 4704.
    pub fn samples_per_frame(&self) -> usize {
        FRAME_BYTES_PER_CHANNEL / self.plan.bytes_per_output() as usize
    }

    /// Bytes per channel of each neighbour frame that the output depends on (those nearest
    /// the run); less than one frame.
    pub fn context_bytes(&self) -> usize {
        if let Some(p) = &self.pysacd {
            return p.context_bytes();
        }
        let (w0, wlen) = self.plan.window();
        let before = -w0;
        let after = w0 + wlen as i64 - 2 * self.plan.bytes_per_output();
        before.max(after).max(0) as usize
    }

    /// Converts frame `cur`, with its neighbours `prev` and `next` (`None` at the edges of
    /// the area), into `out`: `samples_per_frame() x channels` interleaved samples.
    pub fn convert_frame(
        &mut self,
        prev: Option<&[u8]>,
        cur: &[u8],
        next: Option<&[u8]>,
        out: &mut [i32],
    ) -> Result<ConvertStats> {
        let frame = FRAME_BYTES_PER_CHANNEL * usize::from(self.config.channels);
        if cur.len() != frame {
            return Err(input_len(cur.len(), "one frame"));
        }
        self.convert_run(FrameRun::new(cur).with_before(prev).with_after(next), out)
    }

    /// Converts a run of frames into `out`: `frames x samples_per_frame() x channels`
    /// interleaved samples. A whole area is one run with no neighbours.
    pub fn convert_run(&mut self, run: FrameRun<'_>, out: &mut [i32]) -> Result<ConvertStats> {
        self.convert(run.into(), PcmOut::I32(out), None)
    }

    /// Converts a run of frames, in one buffer or one buffer per frame, into `out`:
    /// `frames x samples_per_frame() x channels` interleaved samples in either layout.
    ///
    /// With `frame_clipped`, which must have one entry per frame of the run, each entry is
    /// set to the samples clamped in that frame (over all channels); the entries sum to the
    /// returned [`ConvertStats::clipped_total`]. Nothing is written on an error.
    pub fn convert(
        &mut self,
        run: Run<'_>,
        mut out: PcmOut<'_>,
        mut frame_clipped: Option<&mut [u64]>,
    ) -> Result<ConvertStats> {
        let ch = usize::from(self.config.channels);
        let frame = FRAME_BYTES_PER_CHANNEL * ch;
        let frames = match run.frames {
            Frames::Contiguous(b) => {
                if b.len() % frame != 0 {
                    return Err(input_len(b.len(), "a whole number of frames"));
                }
                b.len() / frame
            }
            Frames::Separate(s) => {
                if let Some(f) = s.iter().find(|f| f.len() != frame) {
                    return Err(input_len(f.len(), "one frame"));
                }
                s.len()
            }
        };
        for n in [run.before, run.after].into_iter().flatten() {
            if n.len() != frame {
                return Err(input_len(n.len(), "one frame"));
            }
        }
        let spf = self.samples_per_frame();
        let expected = frames * spf * ch;
        match &out {
            PcmOut::I32(o) if o.len() != expected => {
                return Err(Error::InvalidArgument(format!(
                    "output of {} samples, {expected} expected",
                    o.len()
                )));
            }
            PcmOut::S24le(o) if o.len() != 3 * expected => {
                return Err(Error::InvalidArgument(format!(
                    "output of {} bytes, {} expected",
                    o.len(),
                    3 * expected
                )));
            }
            _ => {}
        }
        if let Some(fc) = &mut frame_clipped {
            if fc.len() != frames {
                return Err(Error::InvalidArgument(format!(
                    "{} per-frame clip counts for {frames} frames",
                    fc.len()
                )));
            }
            fc.fill(0);
        }
        if let Some(p) = &mut self.pysacd {
            return Ok(p.convert(&run, frames, ch, out, frame_clipped));
        }
        let mut stats = ConvertStats::default();
        let gain = self.config.gain.linear;
        let block_out = BLOCK_FRAMES * spf;
        let isa = self.plan.isa();
        let Converter {
            plan, sc, bytes, q, ..
        } = self;
        let mut f0 = 0;
        while f0 < frames {
            let nf = BLOCK_FRAMES.min(frames - f0);
            let n = nf * spf;
            let o0 = (f0 * spf) as i64;
            let (lo, span) = plan.span(o0, n);
            bytes.resize(span * ch, 0);
            q.resize(block_out * ch, 0);
            fill(&run, frames, ch, lo, span, bytes);
            for (c, (src, qc)) in bytes
                .chunks_exact(span)
                .zip(q.chunks_exact_mut(block_out))
                .enumerate()
            {
                let clipped = &mut stats.clipped[c];
                let fc = &mut frame_clipped;
                plan.run_quantised_segs(src, lo, o0, gain, sc, &mut qc[..n], spf, |s, k| {
                    *clipped += k;
                    if let Some(fc) = fc {
                        fc[f0 + s] += k;
                    }
                });
            }
            let at = f0 * spf * ch;
            let per: [&[i32]; MAX_CHANNELS as usize] = std::array::from_fn(|c| {
                if c < ch {
                    &q[c * block_out..][..n]
                } else {
                    &[][..]
                }
            });
            match &mut out {
                PcmOut::S24le(o) => {
                    decimator::pack24(isa, &per[..ch], &mut o[3 * at..][..3 * n * ch])
                }
                PcmOut::I32(o) => interleave(&per[..ch], &mut o[at..][..n * ch]),
            }
            f0 += nf;
        }
        Ok(stats)
    }
}

fn input_len(len: usize, what: &str) -> Error {
    Error::InvalidArgument(format!("input of {len} bytes is not {what}"))
}

/// Interleaves the channels `q` (equal lengths) into `out`.
fn interleave(q: &[&[i32]], out: &mut [i32]) {
    let ch = q.len();
    for (i, d) in out.chunks_exact_mut(ch).enumerate() {
        for (d, q) in d.iter_mut().zip(q) {
            *d = q[i];
        }
    }
}

/// Fills `bytes` (`ch` blocks of `span`) with each channel's bytes `lo .. lo + span` of the
/// run's timeline: byte 0 is the first byte of the run's first frame; the neighbours lie
/// one frame before and after the run, and everything beyond them is the idle pattern.
pub(crate) fn fill(
    run: &Run<'_>,
    frames: usize,
    ch: usize,
    lo: i64,
    span: usize,
    bytes: &mut [u8],
) {
    let fpc = FRAME_BYTES_PER_CHANNEL as i64;
    let frame = FRAME_BYTES_PER_CHANNEL * ch;
    let end = lo + span as i64;
    // Timeline frames `-1 ..= frames`, as far as they overlap `lo .. end`.
    let first = lo.div_euclid(fpc).max(-1);
    let last = (end - 1).div_euclid(fpc).min(frames as i64);
    let mut done = lo;
    for t in first..=last {
        let src = match t {
            -1 => run.before,
            t if t == frames as i64 => run.after,
            t => Some(run.frame(t as usize, frame)),
        };
        let a = (t * fpc).max(lo);
        let b = ((t + 1) * fpc).min(end);
        if a >= b {
            continue;
        }
        idle(bytes, span, (done - lo) as usize, (a - lo) as usize);
        let off = (a - lo) as usize;
        let len = (b - a) as usize;
        match src {
            Some(f) => {
                let s = &f[(a - t * fpc) as usize * ch..][..len * ch];
                deinterleave(s, ch, bytes, span, off);
            }
            None => idle(bytes, span, off, off + len),
        }
        done = b;
    }
    idle(bytes, span, (done - lo) as usize, span);
}

/// Sets bytes `a .. b` of every channel block to the idle pattern.
fn idle(bytes: &mut [u8], span: usize, a: usize, b: usize) {
    if a < b {
        for blk in bytes.chunks_exact_mut(span) {
            blk[a..b].fill(IDLE);
        }
    }
}

/// Writes the interleaved bytes `src` (`ch` channels) to each channel's block of `dst` from
/// `off` on. The channel count is a constant in each arm, so that the loop is unrolled.
fn deinterleave(src: &[u8], ch: usize, dst: &mut [u8], span: usize, off: usize) {
    #[inline(always)]
    fn by<const CH: usize>(src: &[u8], dst: &mut [u8], span: usize, off: usize) {
        let n = src.len() / CH;
        let mut it = dst.chunks_exact_mut(span);
        let d: [&mut [u8]; CH] =
            std::array::from_fn(|_| &mut it.next().expect("a block per channel")[off..][..n]);
        for (i, s) in src.chunks_exact(CH).enumerate() {
            for c in 0..CH {
                d[c][i] = s[c];
            }
        }
    }
    match ch {
        1 => dst[off..][..src.len()].copy_from_slice(src),
        2 => by::<2>(src, dst, span, off),
        3 => by::<3>(src, dst, span, off),
        4 => by::<4>(src, dst, span, off),
        5 => by::<5>(src, dst, span, off),
        6 => by::<6>(src, dst, span, off),
        _ => unreachable!("1 to 6 channels"),
    }
}

/// Packs 24-bit samples into 3-byte little-endian. `dst` must hold `3 x src.len()` bytes;
/// any excess is left untouched, and a short `dst` receives only the samples that fit.
pub fn pack_s24le(src: &[i32], dst: &mut [u8]) {
    for (s, d) in src.iter().zip(dst.chunks_exact_mut(3)) {
        d.copy_from_slice(&s.to_le_bytes()[..3]);
    }
}

const _: () = {
    const fn both<T: Send + Sync>() {}
    both::<Gain>();
    both::<PcmConfig>();
    both::<ConvertStats>();
    both::<FrameRun<'_>>();
    both::<Frames<'_>>();
    both::<Run<'_>>();
    both::<PcmOut<'_>>();
    both::<Converter>();
};
