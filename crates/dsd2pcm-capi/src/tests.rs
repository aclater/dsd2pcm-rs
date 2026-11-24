//! Tests of the exported functions: NULL pointers, bad lengths and values, overlapping
//! buffers, cancelling from the callback, a busy or poisoned handle, and output bytes
//! identical to the Rust API.

use crate::ffi::*;
use crate::*;
use dsd::dsf::DsfWriter;
use dsd::frames::{Converter, PcmOut, Run};
use dsd::io::DsdWrite;
use std::ffi::{CStr, CString};
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

const FPC: usize = DSD2PCM_FRAME_BYTES_PER_CHANNEL;

fn last_error() -> String {
    // SAFETY (test): the library returns a valid NUL-terminated string.
    unsafe { CStr::from_ptr(dsd2pcm_last_error()) }
        .to_str()
        .unwrap()
        .to_owned()
}

/// Interleaved test frames: noise, idle and full-scale stretches (which clip at +6 dB).
fn area(ch: usize, frames: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    (0..frames * FPC * ch)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            match (i / ch / 2000) % 4 {
                0 | 1 => s as u8,
                2 => 0x69,
                _ => 0xFF,
            }
        })
        .collect()
}

/// A temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!(
            "dsd2pcm-capi-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes a DSF file of `frames` frames of `area` data.
fn write_dsf(path: &std::path::Path, ch: usize, frames: usize) {
    write_dsf_at(path, ch, frames, 2_822_400);
}

/// [`write_dsf`] at DSD sample rate `rate`.
fn write_dsf_at(path: &std::path::Path, ch: usize, frames: usize, rate: u32) {
    let a = area(ch, frames, 0x5eed);
    let chans: Vec<Vec<u8>> = (0..ch)
        .map(|c| a.iter().skip(c).step_by(ch).copied().collect())
        .collect();
    let f = std::fs::File::create(path).unwrap();
    let mut w = DsfWriter::new(f, ch as u16, rate, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish((frames * FPC * 8) as u64).unwrap();
}

fn report_init() -> dsd2pcm_file_report {
    dsd2pcm_file_report {
        struct_size: size_of::<dsd2pcm_file_report>() as u32,
        ..Default::default()
    }
}

fn stats_init() -> dsd2pcm_stats {
    dsd2pcm_stats {
        struct_size: size_of::<dsd2pcm_stats>() as u32,
        ..Default::default()
    }
}

fn cstr(p: &std::path::Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn file_opts(rate_hz: u32) -> dsd2pcm_file_options {
    dsd2pcm_file_options {
        struct_size: size_of::<dsd2pcm_file_options>() as u32,
        rate_hz,
        cutoff: 0,
        filter: 0,
        threads: 0,
        gain_db: 0.0,
        progress: None,
        user_data: null_mut(),
        no_clobber: 0,
    }
}

#[test]
fn version_and_status_strings() {
    // SAFETY (test): static NUL-terminated strings.
    let v = unsafe { CStr::from_ptr(dsd2pcm_version()) };
    assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    for code in 0..=9 {
        // SAFETY (test): as above.
        let s = unsafe { CStr::from_ptr(dsd2pcm_status_str(code)) };
        assert!(!s.to_bytes().is_empty());
    }
    // SAFETY (test): as above.
    let s = unsafe { CStr::from_ptr(dsd2pcm_status_str(DSD2PCM_E_CANCELLED)) };
    assert_eq!(s.to_str().unwrap(), "cancelled");
}

#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn convert_file_matches_rust_and_reports() {
    let d = TempDir::new("file");
    let (inp, ours, theirs) = (d.path("in.dsf"), d.path("c.wav"), d.path("rust.wav"));
    write_dsf(&inp, 2, 40);
    for (rate, cutoff, gain, threads, filter) in [
        (
            176_400,
            DSD2PCM_CUTOFF_STANDARD,
            6.0,
            0,
            DSD2PCM_FILTER_DEFAULT,
        ),
        (88_200, DSD2PCM_CUTOFF_LOW, -3.0, 1, DSD2PCM_FILTER_DEFAULT),
        (352_800, DSD2PCM_CUTOFF_WIDE, 0.0, 0, DSD2PCM_FILTER_DEFAULT),
        (88_200, DSD2PCM_CUTOFF_LOW, 0.0, 1, DSD2PCM_FILTER_PRECISE),
        (
            352_800,
            DSD2PCM_CUTOFF_WIDE,
            -3.0,
            0,
            DSD2PCM_FILTER_PRECISE,
        ),
        (
            176_400,
            DSD2PCM_CUTOFF_STANDARD,
            6.0,
            0,
            DSD2PCM_FILTER_PYSACD,
        ),
        (
            88_200,
            DSD2PCM_CUTOFF_STANDARD,
            -3.0,
            1,
            DSD2PCM_FILTER_PYSACD,
        ),
    ] {
        let mut o = file_opts(rate);
        o.filter = filter;
        o.cutoff = cutoff;
        o.gain_db = gain;
        o.threads = threads;
        let mut rep = report_init();
        // SAFETY (test): valid strings, options and report.
        let rc = unsafe {
            dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&ours).as_ptr(), &o, &mut rep)
        };
        assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
        let (opts, _) = file_options(&o).ok().unwrap();
        let want = dsd::convert_file(&inp, &theirs, &opts).unwrap();
        assert_eq!(
            std::fs::read(&ours).unwrap(),
            std::fs::read(&theirs).unwrap()
        );
        assert_eq!(rep.in_rate, 2_822_400);
        assert_eq!(rep.out_rate, rate);
        assert_eq!(rep.channels, 2);
        assert_eq!(rep.out_samples, want.out_samples);
        assert_eq!(&rep.clipped[..2], &want.clipped[..]);
        assert_eq!(rep.clipped[2..], [0; 4]);
        assert_eq!(rep.clipped_total, want.clipped.iter().sum::<u64>());
        if gain > 0.0 {
            assert!(rep.clipped_total > 0);
        }
    }
    // A NULL report is allowed.
    let o = file_opts(176_400);
    // SAFETY (test): valid strings and options; NULL report.
    let rc =
        unsafe { dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&ours).as_ptr(), &o, null_mut()) };
    assert_eq!(rc, DSD2PCM_OK);
}

#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn convert_file_rejects_nulls_and_bad_values() {
    let d = TempDir::new("bad");
    let inp = d.path("in.dsf");
    write_dsf(&inp, 1, 2);
    let (i, o) = (cstr(&inp), cstr(&d.path("out.wav")));
    let good = file_opts(176_400);
    let call = |i: *const c_char, o: *const c_char, opts: *const dsd2pcm_file_options| {
        // SAFETY (test): every pointer is NULL or valid.
        unsafe { dsd2pcm_convert_file(i, o, opts, null_mut()) }
    };
    assert_eq!(call(null(), o.as_ptr(), &good), DSD2PCM_E_NULL);
    assert_eq!(last_error(), "input_path is NULL");
    assert_eq!(call(i.as_ptr(), null(), &good), DSD2PCM_E_NULL);
    assert_eq!(call(i.as_ptr(), o.as_ptr(), null()), DSD2PCM_E_NULL);
    assert_eq!(last_error(), "options is NULL");
    // Not UTF-8: on Unix a valid path of raw bytes (here, a missing file); elsewhere refused.
    let bad_utf8 = b"\xff\xfe.dsf\0";
    let rc = call(bad_utf8.as_ptr().cast(), o.as_ptr(), &good);
    if cfg!(unix) {
        assert_eq!(rc, DSD2PCM_E_IO);
    } else {
        assert_eq!(rc, DSD2PCM_E_INVALID_ARGUMENT);
        assert!(last_error().contains("UTF-8"));
    }
    assert_eq!(
        call(c"".as_ptr(), o.as_ptr(), &good),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    let mutate = |f: &dyn Fn(&mut dsd2pcm_file_options)| {
        let mut x = good;
        f(&mut x);
        call(i.as_ptr(), o.as_ptr(), &x)
    };
    assert_eq!(mutate(&|x| x.rate_hz = 44_100), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(mutate(&|x| x.rate_hz = 0), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(mutate(&|x| x.cutoff = 3), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(mutate(&|x| x.filter = 3), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(mutate(&|x| x.filter = DSD2PCM_FILTER_PRECISE), DSD2PCM_OK);
    // pysacd's filter: 88.2 and 176.4 kHz, standard cutoff only.
    assert_eq!(mutate(&|x| x.filter = DSD2PCM_FILTER_PYSACD), DSD2PCM_OK);
    assert_ne!(
        mutate(&|x| {
            x.filter = DSD2PCM_FILTER_PYSACD;
            x.rate_hz = 352_800
        }),
        DSD2PCM_OK
    );
    assert_ne!(
        mutate(&|x| {
            x.filter = DSD2PCM_FILTER_PYSACD;
            x.cutoff = DSD2PCM_CUTOFF_LOW
        }),
        DSD2PCM_OK
    );
    // Any thread count gives the same bytes (counts above 256 run on 256 threads).
    let threads_out = |t: u32| {
        assert_eq!(mutate(&|x| x.threads = t), DSD2PCM_OK, "{}", last_error());
        std::fs::read(d.path("out.wav")).unwrap()
    };
    let one = threads_out(1);
    for t in [0, 2, 3, u32::MAX] {
        assert!(threads_out(t) == one, "threads {t}");
    }
    assert_eq!(mutate(&|x| x.gain_db = 25.0), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(
        mutate(&|x| x.gain_db = f64::NAN),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert_eq!(
        mutate(&|x| {
            x.rate_hz = 88_200;
            x.cutoff = DSD2PCM_CUTOFF_WIDE
        }),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    let missing = cstr(&d.path("missing.dsf"));
    assert_eq!(call(missing.as_ptr(), o.as_ptr(), &good), DSD2PCM_E_IO);
    let junk = d.path("junk.dsf");
    std::fs::write(&junk, b"not a DSD file at all").unwrap();
    assert_eq!(
        call(cstr(&junk).as_ptr(), o.as_ptr(), &good),
        DSD2PCM_E_FORMAT
    );
    assert_eq!(
        call(i.as_ptr(), i.as_ptr(), &good),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    // A success leaves the last error alone.
    assert_eq!(call(i.as_ptr(), o.as_ptr(), &good), DSD2PCM_OK);
    assert!(last_error().contains("same") || last_error().contains("input"));
}

struct Calls {
    seen: Vec<(u64, u64)>,
    stop_after: usize,
}

unsafe extern "C" fn record(done: u64, total: u64, user: *mut c_void) -> c_int {
    // SAFETY (test): `user` is the `Calls` passed in the options, alive during the call.
    let c = unsafe { &mut *user.cast::<Calls>() };
    c.seen.push((done, total));
    c_int::from(c.seen.len() > c.stop_after)
}

#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn progress_and_cancel_from_the_callback() {
    let d = TempDir::new("progress");
    let (inp, out) = (d.path("in.dsf"), d.path("out.wav"));
    // About 1.5 MiB per channel: several blocks.
    write_dsf(&inp, 2, 340);
    for threads in [0, 1] {
        let mut calls = Calls {
            seen: Vec::new(),
            stop_after: usize::MAX,
        };
        let mut o = file_opts(176_400);
        o.threads = threads;
        o.progress = Some(record);
        o.user_data = (&mut calls as *mut Calls).cast();
        // SAFETY (test): valid pointers; `calls` outlives the call.
        let rc = unsafe {
            dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&out).as_ptr(), &o, null_mut())
        };
        assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
        let total = (340 * FPC * 8 / 16) as u64;
        assert!(calls.seen.len() >= 3, "{:?}", calls.seen);
        assert_eq!(calls.seen[0], (0, total));
        assert_eq!(*calls.seen.last().unwrap(), (total, total));
        assert!(calls
            .seen
            .windows(2)
            .all(|w| w[0].0 < w[1].0 && w[1].1 == total));

        for stop_after in [0, 1] {
            let mut calls = Calls {
                seen: Vec::new(),
                stop_after,
            };
            o.user_data = (&mut calls as *mut Calls).cast();
            // SAFETY (test): as above.
            let rc = unsafe {
                dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&out).as_ptr(), &o, null_mut())
            };
            assert_eq!(rc, DSD2PCM_E_CANCELLED);
            assert_eq!(last_error(), "conversion cancelled");
            assert_eq!(calls.seen.len(), stop_after + 1);
        }
    }
}

fn frames_cfg(rate_hz: u32, channels: u32) -> dsd2pcm_frames_config {
    dsd2pcm_frames_config {
        struct_size: size_of::<dsd2pcm_frames_config>() as u32,
        rate_hz,
        cutoff: 0,
        filter: 0,
        channels,
        gain_db: 0.0,
    }
}

fn new_handle(cfg: &dsd2pcm_frames_config) -> *mut dsd2pcm_frames {
    let mut h = null_mut();
    // SAFETY (test): valid pointers.
    let rc = unsafe { dsd2pcm_frames_new(cfg, &mut h) };
    assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
    assert!(!h.is_null());
    h
}

fn contiguous(before: Option<&[u8]>, frames: &[u8], after: Option<&[u8]>) -> dsd2pcm_run {
    let (b, bl) = before.map_or((null(), 0), |b| (b.as_ptr(), b.len()));
    let (a, al) = after.map_or((null(), 0), |a| (a.as_ptr(), a.len()));
    dsd2pcm_run {
        struct_size: size_of::<dsd2pcm_run>() as u32,
        before: b,
        before_len: bl,
        frames: if frames.is_empty() {
            null()
        } else {
            frames.as_ptr()
        },
        frames_len: frames.len(),
        frame_ptrs: null(),
        frame_count: 0,
        after: a,
        after_len: al,
        first_frame: 0,
    }
}

#[test]
fn frames_new_rejects_bad_configs() {
    let mut h: *mut dsd2pcm_frames = std::ptr::NonNull::dangling().as_ptr();
    // SAFETY (test): `out` valid, config NULL.
    assert_eq!(
        unsafe { dsd2pcm_frames_new(null(), &mut h) },
        DSD2PCM_E_NULL
    );
    assert!(h.is_null(), "out is set to NULL on failure");
    // SAFETY (test): out NULL.
    assert_eq!(
        unsafe { dsd2pcm_frames_new(&frames_cfg(176_400, 2), null_mut()) },
        DSD2PCM_E_NULL
    );
    for (cfg, code) in [
        (frames_cfg(176_400, 0), DSD2PCM_E_INVALID_ARGUMENT),
        (frames_cfg(176_400, 7), DSD2PCM_E_INVALID_ARGUMENT),
        (frames_cfg(176_400, 256 + 2), DSD2PCM_E_INVALID_ARGUMENT),
        (frames_cfg(48_000, 2), DSD2PCM_E_INVALID_ARGUMENT),
        (
            dsd2pcm_frames_config {
                cutoff: 9,
                ..frames_cfg(176_400, 2)
            },
            DSD2PCM_E_INVALID_ARGUMENT,
        ),
        (
            dsd2pcm_frames_config {
                cutoff: 2,
                ..frames_cfg(88_200, 2)
            },
            DSD2PCM_E_INVALID_ARGUMENT,
        ),
        (
            dsd2pcm_frames_config {
                filter: 1,
                ..frames_cfg(352_800, 2)
            },
            DSD2PCM_E_INVALID_ARGUMENT,
        ),
        (
            dsd2pcm_frames_config {
                gain_db: -121.0,
                ..frames_cfg(176_400, 2)
            },
            DSD2PCM_E_INVALID_ARGUMENT,
        ),
    ] {
        // SAFETY (test): valid pointers.
        assert_eq!(unsafe { dsd2pcm_frames_new(&cfg, &mut h) }, code, "{cfg:?}");
        assert!(h.is_null());
    }
    // NULL handles: queries give 0, free does nothing.
    // SAFETY (test): NULL is allowed.
    unsafe {
        assert_eq!(dsd2pcm_frames_frame_bytes(null()), 0);
        assert_eq!(dsd2pcm_frames_samples_per_frame(null()), 0);
        assert_eq!(dsd2pcm_frames_context_bytes(null()), 0);
        assert_eq!(dsd2pcm_frames_output_bytes(null(), 1, 0), 0);
        dsd2pcm_frames_free(null_mut());
    }
}

#[test]
fn frames_queries() {
    let h = new_handle(&frames_cfg(88_200, 6));
    let r = Converter::new(frames_config(&frames_cfg(88_200, 6)).ok().unwrap()).unwrap();
    // SAFETY (test): live handle.
    unsafe {
        assert_eq!(dsd2pcm_frames_frame_bytes(h), 6 * FPC);
        assert_eq!(dsd2pcm_frames_samples_per_frame(h), 1176);
        assert_eq!(dsd2pcm_frames_context_bytes(h), r.context_bytes());
        assert_eq!(
            dsd2pcm_frames_output_bytes(h, 5, DSD2PCM_FORMAT_S24LE),
            5 * 1176 * 6 * 3
        );
        assert_eq!(
            dsd2pcm_frames_output_bytes(h, 5, DSD2PCM_FORMAT_I32),
            5 * 1176 * 6 * 4
        );
        assert_eq!(dsd2pcm_frames_output_bytes(h, 5, 2), 0);
        assert_eq!(dsd2pcm_frames_output_bytes(h, usize::MAX / 2, 0), 0);
        dsd2pcm_frames_free(h);
    }
}

/// The C converter gives the Rust converter's bytes, per-frame and per-channel clip counts,
/// for contiguous and per-frame buffers (over several groups), in both formats, with both
/// filters, at the start of an area and inside it (pysacd's dither depends on the position).
#[test]
#[cfg_attr(miri, ignore = "too slow: 70-frame conversions, covered natively")]
fn frames_convert_matches_rust() {
    let cases = [
        (176_400, 2u32, 6.0, DSD2PCM_FILTER_DEFAULT, 0u64, true),
        (88_200, 1, 0.0, DSD2PCM_FILTER_DEFAULT, 5, false),
        (352_800, 6, 3.0, DSD2PCM_FILTER_DEFAULT, 0, true),
        (352_800, 2, 3.0, DSD2PCM_FILTER_PRECISE, 0, true),
        (176_400, 1, 0.0, DSD2PCM_FILTER_PRECISE, 7, false),
        (176_400, 2, 6.0, DSD2PCM_FILTER_PYSACD, 1234, true),
        (88_200, 6, 0.0, DSD2PCM_FILTER_PYSACD, 0, false),
        (176_400, 1, -1.5, DSD2PCM_FILTER_PYSACD, u64::MAX - 40, true),
    ];
    for (rate, ch, gain, filter, first, has_before) in cases {
        let cfg = dsd2pcm_frames_config {
            gain_db: gain,
            filter,
            ..frames_cfg(rate, ch)
        };
        let ch = ch as usize;
        let n = 70; // 2 groups of 32 and one of 6
        let a = area(ch, n + 2, rate as u64 ^ first);
        let fb = FPC * ch;
        let before = has_before.then(|| &a[..fb]);
        let (run, after) = (&a[fb..(n + 1) * fb], &a[(n + 1) * fb..]);
        let rust_run = || {
            Run::new(run)
                .with_before(before)
                .with_after(Some(after))
                .at_frame(first)
        };
        let mut rust = Converter::new(frames_config(&cfg).ok().unwrap()).unwrap();
        let spf = rust.samples_per_frame();
        let mut want = vec![0u8; n * spf * ch * 3];
        let mut want_fc = vec![0u64; n];
        let want_st = rust
            .convert(rust_run(), PcmOut::S24le(&mut want), Some(&mut want_fc))
            .unwrap();
        let mut want_i32 = vec![0i32; n * spf * ch];
        rust.convert(rust_run(), PcmOut::I32(&mut want_i32), None)
            .unwrap();
        if gain > 0.0 {
            assert!(want_st.clipped_total() > 0);
        }
        if filter == DSD2PCM_FILTER_PYSACD {
            // The position matters: another first frame gives other bytes.
            let mut other = vec![0u8; want.len()];
            rust.convert(
                rust_run().at_frame(first.wrapping_add(1)),
                PcmOut::S24le(&mut other),
                None,
            )
            .unwrap();
            assert!(
                other != want,
                "pysacd output does not depend on first_frame"
            );
        }

        let h = new_handle(&cfg);
        let ptrs: Vec<*const u8> = run.chunks_exact(fb).map(<[u8]>::as_ptr).collect();
        let mut cont = contiguous(before, run, Some(after));
        cont.first_frame = first;
        let mut separate = contiguous(before, &[], Some(after));
        separate.frame_ptrs = ptrs.as_ptr();
        separate.frame_count = n;
        separate.first_frame = first;
        for r in [cont, separate] {
            let mut got = vec![0u8; want.len()];
            let mut fc = vec![u64::MAX; n];
            let mut st = stats_init();
            // SAFETY (test): every buffer valid for its length.
            let rc = unsafe {
                dsd2pcm_frames_convert(
                    h,
                    &r,
                    DSD2PCM_FORMAT_S24LE,
                    got.as_mut_ptr().cast(),
                    got.len(),
                    fc.as_mut_ptr(),
                    n,
                    &mut st,
                )
            };
            assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
            assert!(got == want, "S24LE bytes differ, filter {filter}");
            assert_eq!(fc, want_fc);
            assert_eq!(st.clipped, want_st.clipped);
            assert_eq!(st.clipped_total, want_st.clipped_total());

            let mut got = vec![0i32; want_i32.len()];
            // SAFETY (test): as above; no per-frame counts or stats.
            let rc = unsafe {
                dsd2pcm_frames_convert(
                    h,
                    &r,
                    DSD2PCM_FORMAT_I32,
                    got.as_mut_ptr().cast(),
                    got.len() * 4,
                    null_mut(),
                    0,
                    null_mut(),
                )
            };
            assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
            assert!(got == want_i32, "I32 samples differ, filter {filter}");
        }
        // SAFETY (test): live handle, freed once.
        unsafe { dsd2pcm_frames_free(h) };
    }
}

/// A whole area (no neighbours) and an empty run.
#[test]
fn frames_edges_and_empty_run() {
    let h = new_handle(&frames_cfg(176_400, 2));
    let a = area(2, 3, 9);
    let mut rust = Converter::new(frames_config(&frames_cfg(176_400, 2)).ok().unwrap()).unwrap();
    let mut want = vec![0u8; 3 * 2352 * 2 * 3];
    rust.convert(Run::new(&a[..]), PcmOut::S24le(&mut want), None)
        .unwrap();
    let mut got = vec![0u8; want.len()];
    // SAFETY (test): valid buffers.
    let rc = unsafe {
        dsd2pcm_frames_convert(
            h,
            &contiguous(None, &a, None),
            0,
            got.as_mut_ptr().cast(),
            got.len(),
            null_mut(),
            0,
            null_mut(),
        )
    };
    assert_eq!(rc, DSD2PCM_OK);
    assert_eq!(got, want);
    let mut st = dsd2pcm_stats {
        clipped_total: 7,
        ..stats_init()
    };
    // SAFETY (test): an empty run with NULL buffers.
    let rc = unsafe {
        dsd2pcm_frames_convert(
            h,
            &contiguous(None, &[], None),
            0,
            null_mut(),
            0,
            null_mut(),
            0,
            &mut st,
        )
    };
    assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
    assert_eq!(st.clipped_total, 0);
    // SAFETY (test): live handle.
    unsafe { dsd2pcm_frames_free(h) };
}

#[test]
fn frames_convert_rejects_bad_arguments() {
    let h = new_handle(&frames_cfg(176_400, 2));
    let fb = 2 * FPC;
    let a = area(2, 4, 3);
    let (before, run, after) = (&a[..fb], &a[fb..3 * fb], &a[3 * fb..]);
    let out_len = 2 * 2352 * 2 * 3;
    let mut out = vec![0u8; out_len + 8];
    let mut fc = [0u64; 2];
    let good = contiguous(Some(before), run, Some(after));
    let call = |r: *const dsd2pcm_run, fmt: u32, o: *mut u8, ol: usize, f: *mut u64, fl: usize| {
        // SAFETY (test): every non-NULL pointer is valid for its length, or the call is
        // refused before it is read.
        unsafe { dsd2pcm_frames_convert(h, r, fmt, o.cast(), ol, f, fl, null_mut()) }
    };
    let o = out.as_mut_ptr();
    let f = fc.as_mut_ptr();
    assert_eq!(call(&good, 0, o, out_len, f, 2), DSD2PCM_OK);
    // SAFETY (test): NULL handle.
    let rc = unsafe {
        dsd2pcm_frames_convert(
            null_mut(),
            &good,
            0,
            o.cast(),
            out_len,
            null_mut(),
            0,
            null_mut(),
        )
    };
    assert_eq!(rc, DSD2PCM_E_NULL);
    assert_eq!(last_error(), "conv is NULL");
    assert_eq!(call(null(), 0, o, out_len, f, 2), DSD2PCM_E_NULL);
    assert_eq!(call(&good, 0, null_mut(), out_len, f, 2), DSD2PCM_E_NULL);
    assert_eq!(call(&good, 0, o, out_len, null_mut(), 2), DSD2PCM_E_NULL);
    assert_eq!(
        call(&good, 0, o, out_len - 1, f, 2),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(last_error().contains("out_bytes"));
    assert_eq!(
        call(&good, 0, o, out_len + 3, f, 2),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert_eq!(call(&good, 0, o, out_len, f, 1), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(call(&good, 7, o, out_len, f, 2), DSD2PCM_E_INVALID_ARGUMENT);
    // Misaligned int32 output.
    let i32_len = 2 * 2352 * 2 * 4;
    let mut buf = vec![0u32; i32_len / 4 + 1];
    let mis = buf.as_mut_ptr().cast::<u8>().wrapping_add(1);
    assert_eq!(
        call(&good, 1, mis, i32_len, f, 2),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(last_error().contains("aligned"));
    assert_eq!(
        call(&good, 1, buf.as_mut_ptr().cast(), i32_len, f, 2),
        DSD2PCM_OK
    );
    // Bad run lengths and pointers.
    let bad = [
        dsd2pcm_run {
            frames_len: run.len() - 1,
            ..good
        },
        dsd2pcm_run {
            before_len: fb - 1,
            ..good
        },
        dsd2pcm_run {
            after_len: fb + 1,
            ..good
        },
        dsd2pcm_run {
            before_len: 0,
            ..good
        },
    ];
    for r in bad {
        assert_eq!(
            call(&r, 0, o, out_len, f, 2),
            DSD2PCM_E_INVALID_ARGUMENT,
            "{r:?}"
        );
    }
    for r in [
        dsd2pcm_run {
            frames: null(),
            ..good
        },
        dsd2pcm_run {
            before: null(),
            ..good
        },
        dsd2pcm_run {
            after: null(),
            ..good
        },
    ] {
        assert_eq!(call(&r, 0, o, out_len, f, 2), DSD2PCM_E_NULL, "{r:?}");
    }
    let ptrs = [run.as_ptr(), null()];
    let sep = dsd2pcm_run {
        frames: null(),
        frames_len: 0,
        frame_ptrs: ptrs.as_ptr(),
        frame_count: 2,
        ..good
    };
    assert_eq!(call(&sep, 0, o, out_len, f, 2), DSD2PCM_E_NULL);
    assert!(last_error().contains("frame_ptrs[1]"));
    let both = dsd2pcm_run {
        frame_ptrs: ptrs.as_ptr(),
        frame_count: 2,
        ..good
    };
    assert_eq!(call(&both, 0, o, out_len, f, 2), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(
        call(
            &dsd2pcm_run {
                frame_ptrs: null(),
                ..sep
            },
            0,
            o,
            out_len,
            f,
            2
        ),
        DSD2PCM_E_NULL
    );
    // Nothing is written on an error.
    out[..out_len].fill(0xAB);
    fc = [9, 9];
    let f = fc.as_mut_ptr();
    assert_eq!(
        call(&bad[2], 0, o, out_len, f, 2),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(out[..out_len].iter().all(|&b| b == 0xAB));
    assert_eq!(fc, [9, 9]);
    // SAFETY (test): live handle.
    unsafe { dsd2pcm_frames_free(h) };
}

#[test]
fn frames_convert_rejects_overlapping_buffers() {
    let h = new_handle(&frames_cfg(352_800, 1));
    let out_len = 4704 * 3;
    // Input and output in one allocation.
    let mut buf = area(1, 1, 5);
    buf.resize(FPC + out_len, 0);
    let p = buf.as_mut_ptr();
    let mut fc = [0u64; 1];
    let call = |r: &dsd2pcm_run, o: *mut u8, f: *mut u64, st: *mut dsd2pcm_stats| {
        // SAFETY (test): every pointer valid for its length; aliasing is what is tested,
        // and the call refuses it before making references.
        unsafe {
            dsd2pcm_frames_convert(h, r, 0, o.cast(), out_len, f, usize::from(!f.is_null()), st)
        }
    };
    let r = dsd2pcm_run {
        frames: p,
        frames_len: FPC,
        ..contiguous(None, &[], None)
    };
    // Disjoint: fine.
    assert_eq!(
        call(&r, p.wrapping_add(FPC), fc.as_mut_ptr(), null_mut()),
        DSD2PCM_OK
    );
    // The output overlaps the frames by one byte.
    assert_eq!(
        call(&r, p.wrapping_add(FPC - 1), null_mut(), null_mut()),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(last_error().contains("overlaps"));
    // The per-frame counts inside the output.
    let mut big = vec![0u64; out_len / 8 + 2];
    let bp = big.as_mut_ptr();
    assert_eq!(
        call(&r, bp.cast(), bp.wrapping_add(3), null_mut()),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    // The stats inside the frames.
    let mut words = vec![0u64; FPC / 8];
    let wp = words.as_mut_ptr();
    let r2 = dsd2pcm_run {
        frames: wp.cast(),
        ..r
    };
    assert_eq!(
        call(
            &r2,
            p.wrapping_add(FPC),
            null_mut(),
            wp.wrapping_add(4).cast()
        ),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    // A per-frame pointer inside the output.
    let ptrs = [p.wrapping_add(FPC).cast_const()];
    let sep = dsd2pcm_run {
        frames: null(),
        frames_len: 0,
        frame_ptrs: ptrs.as_ptr(),
        frame_count: 1,
        ..r
    };
    assert_eq!(
        call(&sep, p.wrapping_add(FPC), null_mut(), null_mut()),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    // The pointer array inside the output.
    let mut arr = vec![0u64; out_len / 8 + 2];
    let ap = arr.as_mut_ptr();
    // SAFETY (test): `arr` is 8-byte aligned and holds more than one pointer.
    unsafe { ap.cast::<*const u8>().write(p.cast_const()) };
    let sep2 = dsd2pcm_run {
        frame_ptrs: ap.cast(),
        ..sep
    };
    assert_eq!(
        call(&sep2, ap.cast(), null_mut(), null_mut()),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    // The same, disjoint, is fine.
    assert_eq!(
        call(&sep2, p.wrapping_add(FPC), null_mut(), null_mut()),
        DSD2PCM_OK
    );
    drop((words, big, arr));
    // SAFETY (test): live handle.
    unsafe { dsd2pcm_frames_free(h) };
}

#[test]
fn busy_and_poisoned_handles() {
    let h = new_handle(&frames_cfg(176_400, 1));
    let a = area(1, 1, 1);
    let mut out = vec![0u8; 2352 * 3];
    let r = contiguous(None, &a, None);
    let call = |out: &mut [u8]| {
        // SAFETY (test): valid buffers.
        unsafe {
            dsd2pcm_frames_convert(
                h,
                &r,
                0,
                out.as_mut_ptr().cast(),
                out.len(),
                null_mut(),
                0,
                null_mut(),
            )
        }
    };
    // SAFETY (test): `h` is NULL or a live handle; only a shared reference.
    let handle = unsafe { h.as_ref() }.expect("a live handle");
    {
        let _held = handle.conv.lock().unwrap();
        assert_eq!(call(&mut out), DSD2PCM_E_BUSY);
    }
    assert_eq!(call(&mut out), DSD2PCM_OK);
    let poison = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = handle.conv.lock().unwrap();
        panic!("poison the handle");
    }));
    assert!(poison.is_err());
    assert_eq!(call(&mut out), DSD2PCM_E_INTERNAL);
    assert!(last_error().contains("unusable"));
    // SAFETY (test): live handle, not in use.
    unsafe { dsd2pcm_frames_free(h) };
}

#[test]
fn panics_become_internal_errors() {
    let rc = guard(|| panic!("boom"));
    assert_eq!(rc, DSD2PCM_E_INTERNAL);
    assert_eq!(last_error(), "internal error: boom");
    assert_eq!(guard_value(5usize, || panic!("boom")), 5);
}

#[test]
fn last_error_is_per_thread() {
    // SAFETY (test): NULL arguments are refused.
    assert_eq!(
        unsafe { dsd2pcm_frames_new(null(), null_mut()) },
        DSD2PCM_E_NULL
    );
    assert_eq!(last_error(), "out is NULL");
    std::thread::spawn(|| assert_eq!(last_error(), ""))
        .join()
        .unwrap();
    assert_eq!(last_error(), "out is NULL");
}

/// Counts allocations per thread, so that tests running in parallel do not interfere.
struct Counting;

thread_local! {
    static ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count() {
    let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
}

// SAFETY (test): forwards to the system allocator.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        count();
        // SAFETY (test): the caller's contract, forwarded.
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        // SAFETY (test): as above.
        unsafe { std::alloc::System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        count();
        // SAFETY (test): as above.
        unsafe { std::alloc::System.realloc(p, l, n) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// After the first call, `dsd2pcm_frames_convert` allocates nothing, in either buffer
/// layout and format.
#[test]
#[cfg_attr(miri, ignore = "too slow: 40-frame conversions, covered natively")]
fn frames_convert_allocates_nothing_after_warm_up() {
    for filter in [
        DSD2PCM_FILTER_DEFAULT,
        DSD2PCM_FILTER_PRECISE,
        DSD2PCM_FILTER_PYSACD,
    ] {
        alloc_case(filter);
    }
}

fn alloc_case(filter: u32) {
    let h = new_handle(&dsd2pcm_frames_config {
        filter,
        ..frames_cfg(176_400, 6)
    });
    let fb = 6 * FPC;
    let n = 40;
    let a = area(6, n + 2, 11);
    let (before, run, after) = (&a[..fb], &a[fb..(n + 1) * fb], &a[(n + 1) * fb..]);
    let ptrs: Vec<*const u8> = run.chunks_exact(fb).map(<[u8]>::as_ptr).collect();
    let mut sep = contiguous(Some(before), &[], Some(after));
    sep.frame_ptrs = ptrs.as_ptr();
    sep.frame_count = n;
    let cont = contiguous(Some(before), run, Some(after));
    let mut out = vec![0i32; n * 2352 * 6];
    let mut fc = vec![0u64; n];
    let mut st = stats_init();
    let mut go = |r: &dsd2pcm_run, fmt: u32| {
        let len = if fmt == 0 {
            n * 2352 * 6 * 3
        } else {
            n * 2352 * 6 * 4
        };
        // SAFETY (test): valid buffers of the given lengths.
        unsafe {
            dsd2pcm_frames_convert(
                h,
                r,
                fmt,
                out.as_mut_ptr().cast(),
                len,
                fc.as_mut_ptr(),
                n,
                &mut st,
            )
        }
    };
    assert_eq!(go(&cont, 0), DSD2PCM_OK);
    let before_count = ALLOCS.with(|a| a.get());
    for (r, fmt) in [(&cont, 0), (&sep, 0), (&cont, 1), (&sep, 1)] {
        assert_eq!(go(r, fmt), DSD2PCM_OK);
    }
    assert_eq!(ALLOCS.with(|a| a.get()), before_count, "filter {filter}");
    // SAFETY (test): live handle.
    unsafe { dsd2pcm_frames_free(h) };
}

#[test]
fn abi_version() {
    assert_eq!(dsd2pcm_abi_version(), DSD2PCM_ABI_VERSION);
    assert_eq!(DSD2PCM_ABI_VERSION, 1);
}

/// Paths are raw bytes on Unix: a file whose name is not UTF-8 converts.
// macOS file systems (APFS, HFS+) refuse names that are not UTF-8, so the test cannot
// create its input there.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn non_utf8_paths_on_unix() {
    use std::os::unix::ffi::OsStrExt;
    let d = TempDir::new("bytes");
    let inp = d.0.join(std::ffi::OsStr::from_bytes(b"in-\xff\xfe.dsf"));
    let out = d.0.join(std::ffi::OsStr::from_bytes(b"out-\xe9.wav"));
    write_dsf(&inp, 1, 2);
    let c = |p: &std::path::Path| CString::new(p.as_os_str().as_bytes()).unwrap();
    // SAFETY (test): valid strings and options.
    let rc = unsafe {
        dsd2pcm_convert_file(
            c(&inp).as_ptr(),
            c(&out).as_ptr(),
            &file_opts(88_200),
            null_mut(),
        )
    };
    assert_eq!(rc, DSD2PCM_OK, "{}", last_error());
    assert!(out.exists());
}

/// DSD128 input with pysacd's filter is refused before the filter is chosen.
#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn dsd128_with_pysacd_is_unsupported() {
    let d = TempDir::new("dsd128");
    let (inp, out) = (d.path("in.dsf"), d.path("out.wav"));
    write_dsf_at(&inp, 2, 2, 5_644_800);
    for filter in [DSD2PCM_FILTER_PYSACD, DSD2PCM_FILTER_DEFAULT] {
        let o = dsd2pcm_file_options {
            filter,
            ..file_opts(88_200)
        };
        // SAFETY (test): valid strings and options.
        let rc = unsafe {
            dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&out).as_ptr(), &o, null_mut())
        };
        assert_eq!(rc, DSD2PCM_E_UNSUPPORTED, "filter {filter}");
        assert!(last_error().contains("only DSD64"), "{}", last_error());
    }
}

/// A caller's struct with a field this version does not know.
#[repr(C)]
#[derive(Clone, Copy)]
struct Newer<T> {
    known: T,
    extra: u64,
}

fn newer<T: Versioned>(mut known: T, extra: u64) -> Newer<T> {
    known.set_struct_size(size_of::<Newer<T>>() as u32);
    Newer { known, extra }
}

/// `struct_size` below the 0.1.0 size or above `MAX_STRUCT_SIZE` is refused for every
/// struct, before anything is read or written; a larger one (a newer header) is accepted
/// when the input struct's bytes past ours are 0, and only our fields are read or written.
#[test]
fn struct_size_is_checked() {
    let cfg = frames_cfg(176_400, 1);
    let h = new_handle(&cfg);
    let a = area(1, 1, 3);
    let mut out = vec![0u8; 2352 * 3];
    let run = contiguous(None, &a, None);
    let convert = |r: *const dsd2pcm_run, st: *mut dsd2pcm_stats, out: &mut [u8]| {
        // SAFETY (test): valid buffers; the structs are at least their struct_size.
        unsafe {
            dsd2pcm_frames_convert(
                h,
                r,
                0,
                out.as_mut_ptr().cast(),
                out.len(),
                null_mut(),
                0,
                st,
            )
        }
    };
    fn too_small<T: Versioned>(k: usize) -> u32 {
        [
            0,
            4,
            T::V1_SIZE as u32 - 1,
            MAX_STRUCT_SIZE as u32 + 1,
            u32::MAX,
        ][k]
    }
    for k in 0..5 {
        let r = dsd2pcm_run {
            struct_size: too_small::<dsd2pcm_run>(k),
            ..run
        };
        assert_eq!(
            convert(&r, null_mut(), &mut out),
            DSD2PCM_E_INVALID_ARGUMENT
        );
        assert!(last_error().contains("struct_size"), "{}", last_error());
        let mut st = dsd2pcm_stats {
            struct_size: too_small::<dsd2pcm_stats>(k),
            clipped_total: 77,
            ..stats_init()
        };
        assert_eq!(convert(&run, &mut st, &mut out), DSD2PCM_E_INVALID_ARGUMENT);
        assert_eq!(st.clipped_total, 77, "nothing written");
        let mut hh = null_mut();
        let c = dsd2pcm_frames_config {
            struct_size: too_small::<dsd2pcm_frames_config>(k),
            ..cfg
        };
        // SAFETY (test): valid pointers.
        assert_eq!(
            unsafe { dsd2pcm_frames_new(&c, &mut hh) },
            DSD2PCM_E_INVALID_ARGUMENT
        );
        assert!(hh.is_null());
    }
    // Newer, larger structs: an input struct's unknown bytes must be 0.
    let bad = newer(run, 1 << 40);
    let mut st = newer(stats_init(), 0xDEAD_BEEF);
    assert_eq!(
        convert(&bad.known, &mut st.known, &mut out),
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(last_error().contains("is not 0"), "{}", last_error());
    assert_eq!(st.known.channels, 0, "nothing written");
    let r = newer(run, 0);
    assert_eq!(
        convert(&r.known, &mut st.known, &mut out),
        DSD2PCM_OK,
        "{}",
        last_error()
    );
    assert_eq!(st.extra, 0xDEAD_BEEF, "only our fields are written");
    assert_eq!(
        st.known.struct_size as usize,
        size_of::<Newer<dsd2pcm_stats>>()
    );
    assert_eq!(st.known.channels, 1);
    let mut hh = null_mut();
    let c = newer(cfg, 1);
    // SAFETY (test): valid pointers.
    assert_eq!(
        unsafe { dsd2pcm_frames_new(&c.known, &mut hh) },
        DSD2PCM_E_INVALID_ARGUMENT
    );
    assert!(hh.is_null());
    let c = newer(cfg, 0);
    // SAFETY (test): valid pointers.
    assert_eq!(unsafe { dsd2pcm_frames_new(&c.known, &mut hh) }, DSD2PCM_OK);
    // SAFETY (test): live handles.
    unsafe {
        dsd2pcm_frames_free(hh);
        dsd2pcm_frames_free(h);
    }
}

#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn struct_size_is_checked_for_files() {
    let d = TempDir::new("size");
    let (inp, out) = (d.path("in.dsf"), d.path("out.wav"));
    write_dsf(&inp, 1, 2);
    let call = |o: *const dsd2pcm_file_options, rep: *mut dsd2pcm_file_report| {
        // SAFETY (test): valid strings; the structs are at least their struct_size.
        unsafe { dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&out).as_ptr(), o, rep) }
    };
    let good = file_opts(88_200);
    let small = dsd2pcm_file_options {
        struct_size: 20,
        ..good
    };
    assert_eq!(call(&small, null_mut()), DSD2PCM_E_INVALID_ARGUMENT);
    let mut rep = dsd2pcm_file_report {
        struct_size: 8,
        out_samples: 5,
        ..report_init()
    };
    assert_eq!(call(&good, &mut rep), DSD2PCM_E_INVALID_ARGUMENT);
    assert_eq!(rep.out_samples, 5, "nothing written");
    assert!(!out.exists(), "refused before converting");
    let bad = newer(good, 1);
    let mut rep = newer(report_init(), 0xDEAD_BEEF);
    assert_eq!(call(&bad.known, &mut rep.known), DSD2PCM_E_INVALID_ARGUMENT);
    assert!(!out.exists(), "refused before converting");
    let o = newer(good, 0);
    assert_eq!(
        call(&o.known, &mut rep.known),
        DSD2PCM_OK,
        "{}",
        last_error()
    );
    assert_eq!(rep.extra, 0xDEAD_BEEF);
    assert_eq!(rep.known.out_samples, (2 * FPC * 8 / 32) as u64);
}

/// The 0.1.0 sizes are pinned per target layout, whatever fields are appended later.
#[test]
fn v1_sizes_are_pinned() {
    let v1 = [
        dsd2pcm_file_options::V1_SIZE,
        dsd2pcm_file_report::V1_SIZE,
        dsd2pcm_frames_config::V1_SIZE,
        dsd2pcm_run::V1_SIZE,
        dsd2pcm_stats::V1_SIZE,
    ];
    let want = match (size_of::<usize>(), std::mem::align_of::<u64>()) {
        (8, _) => [48, 88, 32, 80, 64],
        (4, 4) => [36, 88, 28, 44, 64],
        _ => [40, 88, 32, 48, 64],
    };
    assert_eq!(v1, want);
}

/// A later version of a struct: 0.1.0's fields, then an appended one.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct LaterStruct {
    struct_size: u32,
    a: u32,
    b: u64,
    appended: u64,
}

impl Versioned for LaterStruct {
    const NAME: &'static str = "LaterStruct";
    const V1_SIZE: usize = 16;
    fn set_struct_size(&mut self, size: u32) {
        self.struct_size = size;
    }
}

/// A caller built against the 0.1.0 header passes the shorter struct: only its bytes are
/// read (the buffer is exactly that long, so an over-read shows under Miri and ASan) and
/// the appended field reads as 0; written back, only its bytes are written. A caller with
/// a newer, longer struct gets our fields read and written and the rest left alone.
#[test]
fn versioned_structs_old_and_new_callers() {
    // Old caller: 16 bytes.
    let mut old = vec![0u8; 16].into_boxed_slice();
    old[..4].copy_from_slice(&16u32.to_ne_bytes());
    old[4..8].copy_from_slice(&7u32.to_ne_bytes());
    old[8..16].copy_from_slice(&9u64.to_ne_bytes());
    let p = old.as_mut_ptr().cast::<LaterStruct>();
    // SAFETY (test): `p` is readable for its struct_size (16) bytes.
    let v = unsafe { read_struct(p.cast_const(), "old") }.ok().unwrap();
    assert_eq!(
        v,
        LaterStruct {
            struct_size: 16,
            a: 7,
            b: 9,
            appended: 0
        }
    );
    // SAFETY (test): `p` starts with a readable u32.
    let n = unsafe { out_struct(p, "old") }.ok().unwrap().unwrap();
    assert_eq!(n, 16);
    let w = LaterStruct {
        struct_size: 0,
        a: 1,
        b: 2,
        appended: 3,
    };
    // SAFETY (test): `p` is writable for 16 bytes and no reference to it is live.
    unsafe { write_struct(p, w, n) };
    assert_eq!(&old[..4], &16u32.to_ne_bytes());
    assert_eq!(&old[4..8], &1u32.to_ne_bytes());
    assert_eq!(&old[8..16], &2u64.to_ne_bytes());
    // Too short.
    old[..4].copy_from_slice(&15u32.to_ne_bytes());
    // SAFETY (test): as above.
    assert!(unsafe { read_struct(old.as_ptr().cast::<LaterStruct>(), "old") }.is_err());

    // Newer caller: our 24 bytes and 8 more.
    #[repr(C)]
    struct Newest {
        known: LaterStruct,
        extra: u64,
    }
    let mut new = Newest {
        known: LaterStruct {
            struct_size: size_of::<Newest>() as u32,
            a: 4,
            b: 5,
            appended: 6,
        },
        extra: 0,
    };
    let p = (&mut new as *mut Newest).cast::<LaterStruct>();
    // SAFETY (test): `p` is readable for its struct_size bytes.
    let v = unsafe { read_struct(p.cast_const(), "new") }.ok().unwrap();
    assert_eq!((v.a, v.b, v.appended), (4, 5, 6));
    // A field we do not know, set: refused, whichever of its bytes is set.
    for byte in 0..8 {
        new.extra = 1 << (8 * byte);
        // SAFETY (test): as above.
        let e = unsafe { read_struct(p.cast_const(), "new") }.err().unwrap();
        assert_eq!(e.0, DSD2PCM_E_INVALID_ARGUMENT);
        assert!(
            e.1.contains(&format!("byte {} is not 0", 24 + byte)),
            "{}",
            e.1
        );
    }
    // An output struct's unknown bytes are the caller's: left alone.
    new.extra = 0xDEAD_BEEF;
    // SAFETY (test): as above.
    let n = unsafe { out_struct(p, "new") }.ok().unwrap().unwrap();
    assert_eq!(n, size_of::<LaterStruct>());
    // SAFETY (test): writable for n bytes; no reference to `new` is live.
    unsafe { write_struct(p, w, n) };
    assert_eq!(new.known.struct_size as usize, size_of::<Newest>());
    assert_eq!((new.known.a, new.known.appended), (1, 3));
    assert_eq!(new.extra, 0xDEAD_BEEF);
}

/// `no_clobber` 1: an existing output is refused with `DSD2PCM_E_EXISTS` before converting
/// and left as it was; a new one is written. Other values are invalid. A caller built
/// against the 0.1.0 header (its struct ends before `no_clobber`) gets the old behaviour:
/// only its bytes are read, so whatever follows them is not taken as `no_clobber`.
#[test]
#[cfg_attr(miri, ignore = "file I/O")]
fn no_clobber() {
    let d = TempDir::new("no-clobber");
    let (inp, out) = (d.path("in.dsf"), d.path("out.wav"));
    write_dsf(&inp, 2, 2);
    let call = |o: *const dsd2pcm_file_options| {
        // SAFETY (test): valid strings; `o` is readable for its struct_size bytes.
        unsafe { dsd2pcm_convert_file(cstr(&inp).as_ptr(), cstr(&out).as_ptr(), o, null_mut()) }
    };
    let keep = dsd2pcm_file_options {
        no_clobber: 1,
        ..file_opts(88_200)
    };
    assert_eq!(call(&keep), DSD2PCM_OK, "{}", last_error());
    let first = std::fs::read(&out).unwrap();
    std::fs::write(&out, b"precious").unwrap();
    assert_eq!(call(&keep), DSD2PCM_E_EXISTS);
    assert!(last_error().contains("no-clobber"), "{}", last_error());
    assert_eq!(std::fs::read(&out).unwrap(), b"precious");
    // SAFETY (test): a static string.
    let name = unsafe { CStr::from_ptr(dsd2pcm_status_str(DSD2PCM_E_EXISTS)) };
    assert_eq!(name.to_str().unwrap(), "output exists");
    let bad = dsd2pcm_file_options {
        no_clobber: 2,
        ..keep
    };
    assert_eq!(call(&bad), DSD2PCM_E_INVALID_ARGUMENT);
    assert!(last_error().contains("no_clobber 2"), "{}", last_error());
    // An old caller: the 0.1.0 bytes with struct_size 0.1.0, then (where a newer caller's
    // no_clobber would be) a 1 that must not be read. In a u64 buffer for alignment.
    let n = dsd2pcm_file_options::V1_SIZE;
    let mut buf = vec![0u64; n / 8 + 1];
    let p = buf.as_mut_ptr().cast::<u8>();
    // SAFETY (test): `keep` is a live struct of more than `n` bytes; `buf` holds at least
    // n + 4 bytes and does not overlap it.
    unsafe {
        std::ptr::copy_nonoverlapping((&keep as *const dsd2pcm_file_options).cast::<u8>(), p, n);
        std::ptr::copy_nonoverlapping((n as u32).to_ne_bytes().as_ptr(), p, 4);
        std::ptr::copy_nonoverlapping(1u32.to_ne_bytes().as_ptr(), p.add(n), 4);
    }
    assert_eq!(
        call(p.cast::<dsd2pcm_file_options>()),
        DSD2PCM_OK,
        "{}",
        last_error()
    );
    assert_eq!(std::fs::read(&out).unwrap(), first, "replaced, as in 0.1.0");
}

/// The sizes with the fields appended since 0.1.0.
#[test]
fn current_sizes() {
    let want = match (size_of::<usize>(), std::mem::align_of::<u64>()) {
        (8, _) => 56,
        (4, 4) => 40,
        _ => 48,
    };
    assert_eq!(size_of::<dsd2pcm_file_options>(), want);
}
