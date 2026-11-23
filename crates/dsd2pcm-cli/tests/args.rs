//! Command-line arguments: file names in any encoding the system allows, `--` before file
//! names that start with '-', and clear errors (never a panic) for bad option values.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A stereo DSD64 DSF of one 4096-byte block per channel of idle pattern.
fn small_dsf(path: &Path) {
    let (ch, block) = (2u64, 4096u64);
    let data = block * ch;
    let mut h = Vec::new();
    h.extend(b"DSD ");
    for v in [28u64, 92 + data, 0] {
        h.extend(v.to_le_bytes());
    }
    h.extend(b"fmt ");
    h.extend(52u64.to_le_bytes());
    for v in [1u32, 0, 2, ch as u32, 2_822_400, 1] {
        h.extend(v.to_le_bytes());
    }
    h.extend((block * 8).to_le_bytes());
    h.extend((block as u32).to_le_bytes());
    h.extend(0u32.to_le_bytes());
    h.extend(b"data");
    h.extend((12 + data).to_le_bytes());
    h.resize(h.len() + data as usize, 0x69);
    std::fs::write(path, h).unwrap();
}

/// A new empty directory for one test.
fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-args-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dsd2pcm"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn os(v: &[&str]) -> Vec<OsString> {
    v.iter().map(OsString::from).collect()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn double_dash_ends_options() {
    let d = dir("double-dash");
    small_dsf(&d.join("-x.dsf"));
    let o = run(&d, &os(&["--single-thread", "--", "-x.dsf", "-y.wav"]));
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(d.join("-y.wav").is_file());
    // After `--`, even `--help` is a file name (here a missing one).
    let o = run(&d, &os(&["--", "--help", "out.wav"]));
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(o.stdout.is_empty());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn dash_names_without_double_dash_are_options() {
    let d = dir("dash-name");
    small_dsf(&d.join("-x.dsf"));
    let o = run(&d, &os(&["-x.dsf", "out.wav"]));
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    let e = stderr(&o);
    assert!(e.contains("unknown option -x.dsf"), "{e}");
    assert!(e.contains("put -- before"), "{e}");
    assert!(!d.join("out.wav").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn usage_errors_exit_2() {
    let d = dir("usage");
    for (args, want) in [
        (&["in.dsf"][..], "need IN and OUT"),
        (&["in.dsf", "out.wav", "--rate"][..], "--rate needs a value"),
        (&["in.dsf", "out.wav", "--rate", "x"][..], "bad rate x"),
        (
            &["in.dsf", "out.wav", "--gain", "loud"][..],
            "bad gain loud",
        ),
        (&["in.dsf", "out.wav", "--frobnicate"][..], "unknown option"),
    ] {
        let o = run(&d, &os(args));
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(want), "{args:?}: {}", stderr(&o));
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// File names that are not UTF-8 (Linux file names are bytes) convert; option values that
/// are not UTF-8 give a usage error, never a panic (exit status 101).
#[cfg(unix)]
#[test]
fn non_utf8_arguments() {
    use std::os::unix::ffi::OsStringExt;
    let d = dir("non-utf8");
    let inp = OsString::from_vec(b"in-\xff\xfe.dsf".to_vec());
    let out = OsString::from_vec(b"out-\xe9t\xe9.wav".to_vec());
    if std::fs::write(d.join(&inp), b"").is_err() {
        // A file system that only takes UTF-8 names (as some macOS ones do): nothing to test.
        eprintln!("skipped: the file system refuses non-UTF-8 names");
        return;
    }
    small_dsf(&d.join(&inp));
    let o = run(
        &d,
        &[inp.clone(), out.clone(), OsString::from("--single-thread")],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let wav = std::fs::read(d.join(&out)).unwrap();
    assert_eq!(&wav[..4], b"RIFF");

    for opt in ["--rate", "--cutoff", "--gain", "--filter"] {
        let bad = OsString::from_vec(b"17\xff".to_vec());
        let o = run(&d, &[inp.clone(), out.clone(), OsString::from(opt), bad]);
        assert_eq!(o.status.code(), Some(2), "{opt}: {}", stderr(&o));
        let e = stderr(&o);
        assert!(e.contains(&format!("{opt}: the value")), "{opt}: {e}");
        assert!(e.contains("not valid UTF-8"), "{opt}: {e}");
    }
    // A missing input with a non-UTF-8 name: a conversion error, not a panic.
    let missing = OsString::from_vec(b"missing-\xff.dsf".to_vec());
    let o = run(&d, &[missing, out]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    std::fs::remove_dir_all(&d).unwrap();
}
