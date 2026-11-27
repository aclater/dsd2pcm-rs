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
    assert!(e.contains("unknown option \"-x.dsf\""), "{e}");
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
        (&["in.dsf", "out.wav", "--rate", "x"][..], "bad rate \"x\""),
        (
            &["in.dsf", "out.wav", "--gain", "loud"][..],
            "bad gain \"loud\"",
        ),
        (&["in.dsf", "out.wav", "--frobnicate"][..], "unknown option"),
        // Control characters are escaped, not sent to the terminal.
        (
            &["in.dsf", "out.wav", "--cutoff", "\x1b[2J"][..],
            "cutoff \"\\u{1b}[2J\"",
        ),
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

    for opt in ["--rate", "--cutoff", "--gain", "--filter", "--threads"] {
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

/// `--filter`: no option, `precise` and `default` give the precise filter's bytes, `fast` the
/// library's `Filter::Fast` (scarletbook-pcm's) and other bytes; each is what the library
/// writes with that filter. An unknown name lists the accepted ones.
#[test]
fn filter_names() {
    use dsd2pcm::{convert_file, Filter, PcmOptions, PcmRate};
    let d = dir("filter");
    // Material the filters tell apart: a pseudo-random bitstream, not the idle pattern.
    let inp = d.join("in.dsf");
    small_dsf(&inp);
    let mut f = std::fs::read(&inp).unwrap();
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    for b in &mut f[92..] {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = s as u8;
    }
    std::fs::write(&inp, f).unwrap();
    let cli = |name: &str, extra: &[&str]| {
        let mut a = os(&["in.dsf", name, "--rate", "88200", "--single-thread"]);
        a.extend(os(extra));
        let o = run(&d, &a);
        assert_eq!(o.status.code(), Some(0), "{extra:?}: {}", stderr(&o));
        std::fs::read(d.join(name)).unwrap()
    };
    let lib = |filter: Filter| {
        let mut o = PcmOptions::new(PcmRate::Hz88200);
        o.filter = filter;
        let out = d.join(format!("lib-{filter:?}.wav"));
        convert_file(&inp, &out, &o).unwrap();
        std::fs::read(out).unwrap()
    };
    let none = cli("none.wav", &[]);
    let precise = lib(Filter::Precise);
    let fast = lib(Filter::Fast);
    assert!(
        precise != fast,
        "the test input does not tell the filters apart"
    );
    assert!(none == precise, "no --filter: not the precise filter");
    assert!(cli("p.wav", &["--filter", "precise"]) == precise);
    assert!(cli("d.wav", &["--filter", "default"]) == precise);
    assert!(cli("f.wav", &["--filter", "fast"]) == fast);
    assert!(cli("y.wav", &["--filter", "pysacd"]) == lib(Filter::Pysacd));
    let o = run(&d, &os(&["in.dsf", "x.wav", "--filter", "scarletbook"]));
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("filter \"scarletbook\": precise, fast or pysacd"),
        "{}",
        stderr(&o)
    );
    // The help names the trade-off of `fast`.
    let o = run(&d, &os(&["--help"]));
    let help = String::from_utf8_lossy(&o.stdout);
    assert!(help.contains("--filter precise|fast|pysacd"), "{help}");
    assert!(help.contains("lower CPU, lower quality"), "{help}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// `--threads N`: the same bytes as all cores and `--single-thread` for every N from 1 to the
/// cap; 0, a count above 256 and a non-number are usage errors.
#[test]
fn threads_count() {
    let d = dir("threads");
    // Six channels of eight blocks of a pseudo-random bitstream, so the conversion splits
    // into many pieces.
    let (ch, blocks, block) = (6u64, 8u64, 4096u64);
    let data = blocks * block * ch;
    let mut h = Vec::new();
    h.extend(b"DSD ");
    for v in [28u64, 92 + data, 0] {
        h.extend(v.to_le_bytes());
    }
    h.extend(b"fmt ");
    h.extend(52u64.to_le_bytes());
    for v in [1u32, 0, 7, ch as u32, 2_822_400, 1] {
        h.extend(v.to_le_bytes());
    }
    h.extend((blocks * block * 8).to_le_bytes());
    h.extend((block as u32).to_le_bytes());
    h.extend(0u32.to_le_bytes());
    h.extend(b"data");
    h.extend((12 + data).to_le_bytes());
    let mut s = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..data {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        h.push(s as u8);
    }
    std::fs::write(d.join("in.dsf"), h).unwrap();
    let conv = |name: &str, extra: &[&str]| {
        let mut a = os(&["in.dsf", name]);
        a.extend(os(extra));
        let o = run(&d, &a);
        assert_eq!(o.status.code(), Some(0), "{extra:?}: {}", stderr(&o));
        std::fs::read(d.join(name)).unwrap()
    };
    let all = conv("all.wav", &[]);
    assert!(all == conv("single.wav", &["--single-thread"]));
    for n in ["1", "2", "3", "5", "7", "64", "256"] {
        assert!(all == conv("n.wav", &["--threads", n]), "--threads {n}");
    }
    for bad in ["0", "257", "-1", "two", ""] {
        let o = run(&d, &os(&["in.dsf", "x.wav", "--threads", bad]));
        assert_eq!(o.status.code(), Some(2), "{bad:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("a whole number from 1 to 256"),
            "{bad:?}: {}",
            stderr(&o)
        );
    }
    // The two are alternatives, in either order: a usage error, nothing written.
    for args in [
        ["--single-thread", "--threads", "2"],
        ["--threads", "2", "--single-thread"],
        ["--single-thread", "--threads", "1"],
    ] {
        let mut a = os(&["in.dsf", "both.wav"]);
        a.extend(os(&args));
        let o = run(&d, &a);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains("cannot be used together"), "{args:?}");
        assert!(!d.join("both.wav").exists(), "{args:?}");
    }
    let o = run(&d, &os(&["--help"]));
    assert!(String::from_utf8_lossy(&o.stdout).contains("--threads N"));
    std::fs::remove_dir_all(&d).unwrap();
}
