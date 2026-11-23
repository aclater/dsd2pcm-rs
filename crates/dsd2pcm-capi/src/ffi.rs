//! The exported functions. This module holds all of the crate's `unsafe` code: raw-pointer
//! reads and writes, each with its `SAFETY` reasoning, and `#[no_mangle]`. Each entry point
//! checks its pointers for NULL and its lengths, then runs inside `guard`, which catches
//! panics.
#![deny(clippy::undocumented_unsafe_blocks)]

use crate::*;
use dsd::frames::{Frames, PcmOut, Run};
use std::ffi::CStr;
use std::mem::size_of;
use std::ops::ControlFlow;
use std::path::Path;

/// The library version, for example "0.1.0": a static NUL-terminated string.
#[no_mangle]
pub extern "C" fn dsd2pcm_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// The message of the most recent failed call on this thread (UTF-8, NUL-terminated), or
/// "" if none failed. Successful calls leave it unchanged. The string belongs to the
/// library and stays valid until the next failed call on this thread.
#[no_mangle]
pub extern "C" fn dsd2pcm_last_error() -> *const c_char {
    last_error_ptr()
}

/// A short static name for a status code, such as "invalid argument".
#[no_mangle]
pub extern "C" fn dsd2pcm_status_str(status: c_int) -> *const c_char {
    status_str(status).as_ptr().cast()
}

/// A `&str` from a NUL-terminated UTF-8 string.
///
/// # Safety
/// `p` is NULL or points to a NUL-terminated string that outlives `'a`.
unsafe fn utf8<'a>(p: *const c_char, name: &str) -> Result<&'a str, Fail> {
    if p.is_null() {
        return Err(null(name));
    }
    // SAFETY: `p` is not NULL, and the caller guarantees a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str()
        .map_err(|_| invalid(format!("{name} is not valid UTF-8")))
}

/// Checks a buffer pointer and length: `None` for NULL with length 0, an error for NULL with
/// a length, for a length over `isize::MAX` bytes or for a misaligned pointer.
fn buffer<T>(p: *const T, len: usize, name: &str) -> Result<Option<(usize, usize)>, Fail> {
    if p.is_null() {
        return if len == 0 { Ok(None) } else { Err(null(name)) };
    }
    let bytes = len
        .checked_mul(size_of::<T>())
        .filter(|&b| b <= isize::MAX as usize)
        .ok_or_else(|| invalid(format!("{name}: length {len} is too large")))?;
    if (p as usize) % std::mem::align_of::<T>() != 0 {
        return Err(invalid(format!(
            "{name} is not aligned to {} bytes",
            std::mem::align_of::<T>()
        )));
    }
    Ok(Some((p as usize, bytes)))
}

/// A shared slice from a pointer checked by [`buffer`].
///
/// # Safety
/// If `p` is not NULL, it points to `len` initialised `T`s, valid for reads and not
/// written by anyone during `'a`.
unsafe fn slice<'a, T>(p: *const T, len: usize) -> &'a [T] {
    if p.is_null() {
        &[]
    } else {
        // SAFETY: non-NULL, aligned and at most isize::MAX bytes (checked by `buffer`);
        // readable for `len` elements by the caller's contract.
        unsafe { std::slice::from_raw_parts(p, len) }
    }
}

/// A mutable slice from a pointer checked by [`buffer`].
///
/// # Safety
/// If `p` is not NULL, it points to `len` `T`s valid for reads and writes, accessed by
/// nothing else during `'a` (in particular, no overlap with any other slice in use).
unsafe fn slice_mut<'a, T>(p: *mut T, len: usize) -> &'a mut [T] {
    if p.is_null() {
        &mut []
    } else {
        // SAFETY: non-NULL, aligned and at most isize::MAX bytes (checked by `buffer`);
        // exclusive and writable by the caller's contract, overlap checked by the caller.
        unsafe { std::slice::from_raw_parts_mut(p, len) }
    }
}

/// Converts a DSF or DFF file (DSD64) to a 24-bit WAV file.
///
/// `input_path` and `output_path` are NUL-terminated UTF-8 paths (on every platform). An
/// existing output file is replaced; on any error it may be left incomplete. `report` may
/// be NULL; it is written on success only. The progress callback in `options`, if any, is
/// called on this thread.
///
/// Returns `DSD2PCM_OK` or a `DSD2PCM_E_*` code; `dsd2pcm_last_error` gives the message.
///
/// # Safety
/// `input_path` and `output_path` are NULL or NUL-terminated strings; `options` is NULL or
/// points to a `dsd2pcm_file_options`; `report` is NULL or points to writable memory for a
/// `dsd2pcm_file_report`. The callback must not unwind.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_convert_file(
    input_path: *const c_char,
    output_path: *const c_char,
    options: *const dsd2pcm_file_options,
    report: *mut dsd2pcm_file_report,
) -> c_int {
    guard(|| {
        // SAFETY: NUL-terminated or NULL by the contract; NULL is checked.
        let input = unsafe { utf8(input_path, "input_path") }?;
        // SAFETY: as above.
        let output = unsafe { utf8(output_path, "output_path") }?;
        if options.is_null() {
            return Err(null("options"));
        }
        // SAFETY: not NULL; points to a `dsd2pcm_file_options` by the contract. It is
        // copied, so later changes by the caller do not matter.
        let o = unsafe { options.read_unaligned() };
        let opts = file_options(&o)?;
        let (cb, user_data) = (o.progress, o.user_data);
        let mut hook = |p: dsd::Progress| match cb {
            // SAFETY: a C function pointer supplied by the caller, called with the
            // documented arguments; it must not unwind (contract).
            Some(f) if unsafe { f(p.done, p.total, user_data) } != 0 => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        };
        let rep = dsd::convert_file_with(Path::new(input), Path::new(output), &opts, &mut hook)?;
        if !report.is_null() {
            // SAFETY: not NULL; writable for one `dsd2pcm_file_report` by the contract.
            unsafe { report.write_unaligned(file_report(&rep)) };
        }
        Ok(())
    })
}

/// Creates a frame-run converter for `config` and stores it in `*out` (NULL on failure).
/// Free it with `dsd2pcm_frames_free`.
///
/// # Safety
/// `config` is NULL or points to a `dsd2pcm_frames_config`; `out` is NULL or points to
/// writable memory for a pointer.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_new(
    config: *const dsd2pcm_frames_config,
    out: *mut *mut dsd2pcm_frames,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return Err(null("out"));
        }
        // SAFETY: not NULL; writable for a pointer by the contract.
        unsafe { out.write_unaligned(std::ptr::null_mut()) };
        if config.is_null() {
            return Err(null("config"));
        }
        // SAFETY: not NULL; points to a `dsd2pcm_frames_config` by the contract.
        let c = unsafe { config.read_unaligned() };
        let conv = dsd::frames::Converter::new(frames_config(&c)?)?;
        let handle = Box::into_raw(Box::new(dsd2pcm_frames::new(conv)));
        // SAFETY: as above.
        unsafe { out.write_unaligned(handle) };
        Ok(())
    })
}

/// Frees a converter. NULL is allowed and does nothing.
///
/// # Safety
/// `conv` is NULL or a handle from `dsd2pcm_frames_new` not yet freed, and no other thread
/// is using it.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_free(conv: *mut dsd2pcm_frames) {
    guard_value((), || {
        if !conv.is_null() {
            // SAFETY: a live handle from `Box::into_raw` in `dsd2pcm_frames_new`, freed once
            // and not in use (contract).
            drop(unsafe { Box::from_raw(conv) });
        }
    })
}

/// A shared reference to a live handle, or `None` for NULL.
///
/// # Safety
/// `conv` is NULL or a live handle from `dsd2pcm_frames_new`.
unsafe fn handle<'a>(conv: *const dsd2pcm_frames) -> Option<&'a dsd2pcm_frames> {
    // SAFETY: NULL or a live, properly aligned handle (contract); only shared references
    // are made, and the converter itself is behind a mutex.
    unsafe { conv.as_ref() }
}

/// A query of a handle: `f` of it, or 0 for NULL or a panic.
///
/// # Safety
/// `conv` is NULL or a live handle.
unsafe fn query(conv: *const dsd2pcm_frames, f: impl FnOnce(&dsd2pcm_frames) -> usize) -> usize {
    guard_value(0, || {
        // SAFETY: the contract of this function.
        unsafe { handle(conv) }.map_or(0, f)
    })
}

/// Bytes of one frame (all channels): channels x 4704. 0 if `conv` is NULL.
///
/// # Safety
/// `conv` is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_frame_bytes(conv: *const dsd2pcm_frames) -> usize {
    // SAFETY: the contract of this function.
    unsafe { query(conv, |h| h.frame_bytes()) }
}

/// Output samples per channel per frame: 1176, 2352 or 4704. 0 if `conv` is NULL.
///
/// # Safety
/// `conv` is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_samples_per_frame(conv: *const dsd2pcm_frames) -> usize {
    // SAFETY: the contract of this function.
    unsafe { query(conv, |h| h.samples_per_frame) }
}

/// Bytes per channel of each neighbour frame that the output depends on (those nearest the
/// run); less than one frame. 0 if `conv` is NULL.
///
/// # Safety
/// `conv` is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_context_bytes(conv: *const dsd2pcm_frames) -> usize {
    // SAFETY: the contract of this function.
    unsafe { query(conv, |h| h.context_bytes) }
}

/// Bytes of output for `frames` frames in `format` (`DSD2PCM_FORMAT_*`): the `out_bytes`
/// that `dsd2pcm_frames_convert` expects. 0 if `conv` is NULL, the format is unknown or the
/// size overflows.
///
/// # Safety
/// `conv` is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_output_bytes(
    conv: *const dsd2pcm_frames,
    frames: usize,
    format: u32,
) -> usize {
    // SAFETY: the contract of this function.
    unsafe { query(conv, |h| h.output_bytes(frames, format).unwrap_or(0)) }
}

/// Frames per call into the converter when the run is one buffer per frame: the slice of
/// frame references lives on the stack, so nothing is allocated.
const GROUP: usize = 32;

/// Converts a run of frames into `out`: `frames x samples_per_frame x channels` interleaved
/// samples in `format` (`DSD2PCM_FORMAT_S24LE`: 3 bytes each; `DSD2PCM_FORMAT_I32`:
/// `int32_t`, `out` aligned to 4 bytes). `out_bytes` must equal
/// `dsd2pcm_frames_output_bytes(conv, frames, format)`.
///
/// `frame_clipped` is NULL (with `frame_clipped_len` 0) or one entry per frame of the run;
/// each entry is set to the samples clamped in that frame over all channels. `stats` may be
/// NULL. On an error nothing is written. The output equals the run's part of a conversion
/// of the whole area; after the first call the converter allocates nothing.
///
/// Output buffers must not overlap each other or any input buffer.
///
/// # Safety
/// `conv` is NULL or a live handle; `run` is NULL or points to a `dsd2pcm_run` whose
/// non-NULL pointers are valid for reads of their lengths (each `frame_ptrs` entry for one
/// frame); `out` is NULL or valid for writes of `out_bytes` bytes; `frame_clipped` is NULL
/// or valid for writes of `frame_clipped_len` entries; `stats` is NULL or writable for one
/// `dsd2pcm_stats`. No other thread writes these buffers during the call.
#[no_mangle]
pub unsafe extern "C" fn dsd2pcm_frames_convert(
    conv: *mut dsd2pcm_frames,
    run: *const dsd2pcm_run,
    format: u32,
    out: *mut c_void,
    out_bytes: usize,
    frame_clipped: *mut u64,
    frame_clipped_len: usize,
    stats: *mut dsd2pcm_stats,
) -> c_int {
    guard(|| {
        // SAFETY: contract of this function.
        let h = unsafe { handle(conv) }.ok_or_else(|| null("conv"))?;
        if run.is_null() {
            return Err(null("run"));
        }
        // SAFETY: not NULL; points to a `dsd2pcm_run` by the contract; copied.
        let r = unsafe { run.read_unaligned() };
        let fb = h.frame_bytes();

        // Every buffer's address range, before any reference is made.
        let before = buffer(r.before, r.before_len, "run.before")?;
        let after = buffer(r.after, r.after_len, "run.after")?;
        let contiguous = buffer(r.frames, r.frames_len, "run.frames")?;
        let ptrs = buffer(r.frame_ptrs, r.frame_count, "run.frame_ptrs")?;
        if contiguous.is_some() && ptrs.is_some() {
            return Err(invalid("run has both frames and frame_ptrs"));
        }
        let frames = if ptrs.is_some() {
            r.frame_count
        } else if r.frames_len % fb != 0 {
            return Err(invalid(format!(
                "run.frames_len {} is not a whole number of {fb}-byte frames",
                r.frames_len
            )));
        } else {
            r.frames_len / fb
        };
        let expected = h
            .output_bytes(frames, format)
            .ok_or_else(|| invalid(format!("format {format}: 0 (S24LE) or 1 (I32)")))?;
        if out_bytes != expected {
            return Err(invalid(format!(
                "out_bytes {out_bytes}, {expected} expected"
            )));
        }
        let outr = match format {
            DSD2PCM_FORMAT_I32 => buffer(out.cast::<i32>(), out_bytes / 4, "out")?,
            _ => buffer(out.cast::<u8>(), out_bytes, "out")?,
        };
        let fcr = buffer(frame_clipped, frame_clipped_len, "frame_clipped")?;
        if fcr.is_some() && frame_clipped_len != frames {
            return Err(invalid(format!(
                "{frame_clipped_len} per-frame clip counts for {frames} frames"
            )));
        }
        let statr = buffer(stats, usize::from(!stats.is_null()), "stats")?;

        for (n, len) in [(before, r.before_len), (after, r.after_len)] {
            if n.is_some() && len != fb {
                return Err(invalid(format!(
                    "neighbour frame of {len} bytes, {fb} expected"
                )));
            }
        }
        let inputs = [before, after, contiguous, ptrs];
        let outputs = [outr, fcr, statr];
        for (i, m) in outputs.iter().enumerate() {
            let Some(m) = *m else { continue };
            let clash = inputs.iter().flatten().any(|&x| overlaps(m, x))
                || outputs[i + 1..].iter().flatten().any(|&o| overlaps(m, o));
            if clash {
                return Err(invalid("an output buffer overlaps another buffer"));
            }
        }
        // SAFETY: checked by `buffer`; readable for `frame_count` pointers (contract), and
        // overlapping no output buffer (checked above).
        let frame_ptrs: &[*const u8] = unsafe { slice(r.frame_ptrs, r.frame_count) };
        for (i, &p) in frame_ptrs.iter().enumerate() {
            if p.is_null() {
                return Err(null(&format!("run.frame_ptrs[{i}]")));
            }
            let range = buffer(p, fb, "run.frame_ptrs entry")?.unwrap_or((0, 0));
            if outputs.iter().flatten().any(|&m| overlaps(m, range)) {
                return Err(invalid(format!(
                    "run.frame_ptrs[{i}] overlaps an output buffer"
                )));
            }
        }

        // SAFETY: each pointer below passed `buffer` (aligned, length in range) and is
        // readable for its length (contract); none overlaps an output buffer.
        let before = before.map(|_| unsafe { slice(r.before, r.before_len) });
        // SAFETY: as above.
        let after = after.map(|_| unsafe { slice(r.after, r.after_len) });
        // SAFETY: as above.
        let contiguous: &[u8] = unsafe { slice(r.frames, r.frames_len) };
        // SAFETY: as above, for one frame each; every entry was checked non-NULL.
        let frame_at = |i: usize| -> &[u8] { unsafe { slice(frame_ptrs[i], fb) } };

        let mut conv = match h.conv.try_lock() {
            Ok(c) => c,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(Fail(
                    DSD2PCM_E_BUSY,
                    "the converter is in use by another thread".into(),
                ))
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(Fail(
                    DSD2PCM_E_INTERNAL,
                    "the converter is unusable after an internal error".into(),
                ))
            }
        };

        // One view of `out`, in the caller's format.
        let (out_s24, out_i32): (&mut [u8], &mut [i32]) = if format == DSD2PCM_FORMAT_I32 {
            // SAFETY: valid for writes of `out_bytes` bytes (contract), aligned to 4 bytes
            // and at most isize::MAX bytes (checked by `buffer`), overlapping no other
            // buffer in use (checked above).
            (&mut [], unsafe {
                slice_mut(out.cast::<i32>(), out_bytes / 4)
            })
        } else {
            // SAFETY: as above, as bytes.
            (unsafe { slice_mut(out.cast::<u8>(), out_bytes) }, &mut [])
        };
        // SAFETY: as above, `frame_clipped_len` entries.
        let fc: &mut [u64] = unsafe { slice_mut(frame_clipped, frame_clipped_len) };
        let mut fc = fcr.map(|_| fc);
        let per_frame = h.samples_per_frame * h.channels;

        let mut total = dsd::frames::ConvertStats::default();
        let mut add = |s: dsd::frames::ConvertStats| {
            for (t, c) in total.clipped.iter_mut().zip(s.clipped) {
                *t += c;
            }
        };
        if ptrs.is_none() {
            let run = Run::new(contiguous).with_before(before).with_after(after);
            let o = if format == DSD2PCM_FORMAT_I32 {
                PcmOut::I32(out_i32)
            } else {
                PcmOut::S24le(out_s24)
            };
            add(conv.convert(run, o, fc)?);
        } else {
            // In groups of GROUP frames, each with its real neighbours: the output of a run
            // with its neighbours is that of the whole area, so the groups give the same
            // bytes as one call.
            let mut refs: [&[u8]; GROUP] = [&[]; GROUP];
            let mut g = 0;
            while g < frames {
                let n = GROUP.min(frames - g);
                for (k, slot) in refs[..n].iter_mut().enumerate() {
                    *slot = frame_at(g + k);
                }
                let b = if g == 0 {
                    before
                } else {
                    Some(frame_at(g - 1))
                };
                let a = if g + n == frames {
                    after
                } else {
                    Some(frame_at(g + n))
                };
                let run = Run::new(Frames::Separate(&refs[..n]))
                    .with_before(b)
                    .with_after(a);
                let (s, e) = (g * per_frame, (g + n) * per_frame);
                let o = if format == DSD2PCM_FORMAT_I32 {
                    PcmOut::I32(&mut out_i32[s..e])
                } else {
                    PcmOut::S24le(&mut out_s24[3 * s..3 * e])
                };
                let fcg = fc.as_deref_mut().map(|f| &mut f[g..g + n]);
                add(conv.convert(run, o, fcg)?);
                g += n;
            }
        }
        if !stats.is_null() {
            let st = dsd2pcm_stats {
                clipped: total.clipped,
                clipped_total: total.clipped_total(),
            };
            // SAFETY: not NULL; writable for one `dsd2pcm_stats` (contract), overlapping no
            // other buffer (checked).
            unsafe { stats.write_unaligned(st) };
        }
        Ok(())
    })
}
