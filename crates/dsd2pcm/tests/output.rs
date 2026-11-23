//! `convert_file`: the output is written to a temporary file and renamed into place, so a
//! failed or cancelled conversion leaves an existing output as it was, and an output that
//! is the input (also through a hard link) is refused.

use dsd2pcm::dsf::DsfWriter;
use dsd2pcm::io::DsdWrite;
use dsd2pcm::{convert_file, convert_file_with, Error, PcmOptions, PcmRate};
use std::io::Cursor;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

/// A stereo DSF of `bytes` bytes per channel at `rate`.
fn dsf(bytes: usize, rate: u32) -> Vec<u8> {
    let chans: Vec<Vec<u8>> = (0..2)
        .map(|c| (0..bytes).map(|i| (i * 7 + c * 3) as u8 ^ 0x69).collect())
        .collect();
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), 2, rate, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish(bytes as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

/// A fresh directory for one test.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dsd2pcm-output-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The names in `d`, sorted.
fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

const PRECIOUS: &[u8] = b"an existing file that a failed conversion must not touch";

#[test]
fn failed_conversion_leaves_existing_output() {
    let d = dir("fail");
    let (dsd128, out) = (d.join("in128.dsf"), d.join("out.wav"));
    std::fs::write(&dsd128, dsf(10_000, 5_644_800)).unwrap();
    std::fs::write(&out, PRECIOUS).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz176400);
    let r = convert_file(&dsd128, &out, &opts);
    assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    assert_eq!(std::fs::read(&out).unwrap(), PRECIOUS);
    // Bad options and a missing input also fail before the output is touched.
    let inp = d.join("in.dsf");
    std::fs::write(&inp, dsf(10_000, 2_822_400)).unwrap();
    let mut bad = opts;
    bad.gain_db = 99.0;
    assert!(convert_file(&inp, &out, &bad).is_err());
    assert!(convert_file(&d.join("missing.dsf"), &out, &opts).is_err());
    assert_eq!(std::fs::read(&out).unwrap(), PRECIOUS);
    assert_eq!(names(&d), ["in.dsf", "in128.dsf", "out.wav"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn cancelled_conversion_leaves_existing_output() {
    let d = dir("cancel");
    let (inp, out) = (d.join("in.dsf"), d.join("out.wav"));
    std::fs::write(&inp, dsf(200_000, 2_822_400)).unwrap();
    std::fs::write(&out, PRECIOUS).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz176400);
    // Cancel at once, and after some blocks have been written to the temporary file.
    for after in [0, 1] {
        let mut calls = 0;
        let r = convert_file_with(&inp, &out, &opts, &mut |_| {
            calls += 1;
            if calls > after {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        assert_eq!(std::fs::read(&out).unwrap(), PRECIOUS);
        assert_eq!(names(&d), ["in.dsf", "out.wav"]);
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn success_replaces_existing_output() {
    let d = dir("ok");
    let (inp, out, fresh) = (d.join("in.dsf"), d.join("out.wav"), d.join("fresh.wav"));
    std::fs::write(&inp, dsf(50_000, 2_822_400)).unwrap();
    std::fs::write(&out, PRECIOUS).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz88200);
    convert_file(&inp, &out, &opts).unwrap();
    convert_file(&inp, &fresh, &opts).unwrap();
    let got = std::fs::read(&out).unwrap();
    assert_eq!(&got[..4], b"RIFF");
    assert_eq!(got, std::fs::read(&fresh).unwrap());
    assert_eq!(names(&d), ["fresh.wav", "in.dsf", "out.wav"]);
    // A relative output path with no directory part.
    let cwd = std::env::current_dir().unwrap();
    let rel = format!("dsd2pcm-output-rel-{}.wav", std::process::id());
    let rel_out = cwd.join(&rel);
    convert_file(&inp, Path::new(&rel), &opts).unwrap();
    assert_eq!(std::fs::read(&rel_out).unwrap(), got);
    std::fs::remove_file(&rel_out).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn output_that_is_the_input_is_refused() {
    let d = dir("same");
    let inp = d.join("in.dsf");
    let bytes = dsf(10_000, 2_822_400);
    std::fs::write(&inp, &bytes).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz176400);
    let mut outs = vec![inp.clone(), d.join(".").join("in.dsf")];
    #[cfg(unix)]
    {
        std::fs::hard_link(&inp, d.join("hard.wav")).unwrap();
        std::os::unix::fs::symlink(&inp, d.join("sym.wav")).unwrap();
        outs.push(d.join("hard.wav"));
        outs.push(d.join("sym.wav"));
    }
    for out in &outs {
        let r = convert_file(&inp, out, &opts);
        assert!(
            matches!(r, Err(Error::InvalidArgument(_))),
            "{out:?}: {r:?}"
        );
        assert_eq!(std::fs::read(&inp).unwrap(), bytes, "{out:?}");
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// A conversion stopped part-way never leaves a partial WAV behind: no output, no temporary
/// file.
#[test]
fn cancelled_conversion_leaves_no_partial_output() {
    let d = dir("partial");
    let (inp, out) = (d.join("in.dsf"), d.join("out.wav"));
    std::fs::write(&inp, dsf(400_000, 2_822_400)).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz352800);
    let mut blocks = 0;
    let r = convert_file_with(&inp, &out, &opts, &mut |p| {
        if p.done > 0 {
            blocks += 1;
        }
        if blocks >= 2 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(blocks >= 2, "cancelled after some blocks were written");
    assert!(!out.exists());
    assert_eq!(names(&d), ["in.dsf"]);
    std::fs::remove_dir_all(&d).unwrap();
}

/// The temporary file is `.dsd2pcm-<pid>-<n>.tmp` in the output's directory, whatever the
/// output's name, so a 255-byte output name works.
#[test]
fn temporary_name_is_short_and_fixed() {
    let d = dir("names");
    let inp = d.join("in.dsf");
    std::fs::write(&inp, dsf(20_000, 2_822_400)).unwrap();
    let long = format!("{}.wav", "x".repeat(251));
    assert_eq!(long.len(), 255);
    let out = d.join(&long);
    let opts = PcmOptions::new(PcmRate::Hz176400);
    let mut seen = Vec::new();
    convert_file_with(&inp, &out, &opts, &mut |p| {
        if p.done > 0 && seen.is_empty() {
            seen = names(&d);
        }
        ControlFlow::Continue(())
    })
    .unwrap();
    let tmp: Vec<&String> = seen.iter().filter(|n| n.ends_with(".tmp")).collect();
    assert_eq!(tmp.len(), 1, "{seen:?}");
    let want = format!(".dsd2pcm-{}-", std::process::id());
    assert!(tmp[0].starts_with(&want), "{}", tmp[0]);
    assert_eq!(names(&d), ["in.dsf", long.as_str()]);
    std::fs::remove_dir_all(&d).unwrap();
}

/// A directory, a path ending in a separator, and (on Unix) a FIFO and a device are refused
/// before anything is written.
#[test]
fn non_file_outputs_are_refused() {
    let d = dir("nonfile");
    let inp = d.join("in.dsf");
    std::fs::write(&inp, dsf(10_000, 2_822_400)).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz176400);
    std::fs::create_dir(d.join("sub")).unwrap();
    let mut outs = vec![
        d.join("sub"),
        PathBuf::from(format!("{}/", d.join("sub").display())),
        PathBuf::from(format!("{}/", d.join("new.wav").display())),
    ];
    #[cfg(unix)]
    {
        let fifo = d.join("fifo.wav");
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        outs.push(fifo);
        outs.push(PathBuf::from("/dev/null"));
    }
    for out in &outs {
        let r = convert_file(&inp, out, &opts);
        assert!(
            matches!(r, Err(Error::InvalidArgument(_))),
            "{out:?}: {r:?}"
        );
    }
    assert!(d.join("sub").is_dir());
    assert_eq!(std::fs::read_dir(d.join("sub")).unwrap().count(), 0);
    let mut want = vec!["in.dsf", "sub"];
    if cfg!(unix) {
        want.insert(0, "fifo.wav");
    }
    assert_eq!(names(&d), want);
    std::fs::remove_dir_all(&d).unwrap();
}

/// A replaced file keeps its permissions; a file that is not writable is refused and kept.
#[cfg(unix)]
#[test]
fn permissions_of_a_replaced_file() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("perms");
    let (inp, out) = (d.join("in.dsf"), d.join("out.wav"));
    std::fs::write(&inp, dsf(10_000, 2_822_400)).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz176400);
    for mode in [0o640, 0o600, 0o664] {
        std::fs::write(&out, PRECIOUS).unwrap();
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode)).unwrap();
        convert_file(&inp, &out, &opts).unwrap();
        let got = std::fs::metadata(&out).unwrap().permissions().mode() & 0o7777;
        assert_eq!(got, mode);
        assert_eq!(&std::fs::read(&out).unwrap()[..4], b"RIFF");
    }
    std::fs::write(&out, PRECIOUS).unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o444)).unwrap();
    // Root may write anyway; the refusal is only checked where the file is really read-only.
    if std::fs::OpenOptions::new().write(true).open(&out).is_err() {
        let r = convert_file(&inp, &out, &opts);
        assert!(matches!(r, Err(Error::Io(_))), "{r:?}");
        assert!(r.unwrap_err().to_string().contains("not writable"));
        assert_eq!(std::fs::read(&out).unwrap(), PRECIOUS);
        assert_eq!(names(&d), ["in.dsf", "out.wav"]);
    }
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}

/// An output in a directory that cannot be written is refused before converting.
#[cfg(unix)]
#[test]
fn unwritable_directory_is_refused_before_converting() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("rodir");
    let inp = d.join("in.dsf");
    std::fs::write(&inp, dsf(10_000, 2_822_400)).unwrap();
    let ro = d.join("ro");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(ro.join("probe")).is_err() {
        let mut calls = 0;
        let r = convert_file_with(
            &inp,
            &ro.join("out.wav"),
            &PcmOptions::new(PcmRate::Hz176400),
            &mut |_| {
                calls += 1;
                ControlFlow::Continue(())
            },
        );
        assert!(matches!(r, Err(Error::Io(_))), "{r:?}");
        assert_eq!(calls, 0, "refused before the conversion started");
    }
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}
