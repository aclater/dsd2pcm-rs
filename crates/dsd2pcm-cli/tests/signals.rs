//! The command-line tool stopped by SIGINT or SIGTERM part-way through a conversion: it
//! exits with 130 or 143, leaves no temporary file, and leaves an existing output as it was.
#![cfg(unix)]

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A 6-channel DSD64 DSF of 300 s whose audio is a hole (sparse, all zero bytes): about
/// 600 MB on paper, nothing on disk, and seconds to convert on one thread.
fn long_dsf(path: &Path) {
    let (ch, secs, block) = (6u64, 300u64, 4096u64);
    let bytes = 352_800 * secs;
    let data = bytes.div_ceil(block) * block * ch;
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
    h.extend((bytes * 8).to_le_bytes());
    h.extend((block as u32).to_le_bytes());
    h.extend(0u32.to_le_bytes());
    h.extend(b"data");
    h.extend((12 + data).to_le_bytes());
    let mut f = File::create(path).unwrap();
    f.write_all(&h).unwrap();
    f.set_len(92 + data).unwrap();
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

const PRECIOUS: &[u8] = b"an existing output that an interrupted conversion must not touch";

/// Starts a conversion, waits until its temporary file has data, sends `signal`, and
/// returns the exit status and the file names left in the directory.
fn interrupt(dir: &Path, signal: &str) -> (Option<i32>, Vec<String>) {
    let (inp, out) = (dir.join("in.dsf"), dir.join("out.wav"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_dsd2pcm"))
        .arg(&inp)
        .arg(&out)
        .args(["--rate", "352800", "--cutoff", "30", "--single-thread"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    loop {
        let started = std::fs::read_dir(dir).unwrap().any(|e| {
            let e = e.unwrap();
            e.file_name().to_string_lossy().ends_with(".tmp")
                && e.metadata().map(|m| m.len() > 1 << 20).unwrap_or(false)
        });
        if started {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the conversion ended before it could be interrupted"
        );
        assert!(t.elapsed() < Duration::from_secs(60), "no temporary file");
        std::thread::sleep(Duration::from_millis(5));
    }
    let ok = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap()
        .success();
    assert!(ok, "kill {signal}");
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("interrupted"), "{err}");
    (out.status.code(), names(dir))
}

#[test]
fn sigint_and_sigterm_leave_no_partial_output() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("dsd2pcm-cli-signals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    long_dsf(&dir.join("in.dsf"));
    std::fs::write(dir.join("out.wav"), PRECIOUS).unwrap();
    for (signal, code) in [("-INT", 130), ("-TERM", 143)] {
        let (status, left) = interrupt(&dir, signal);
        assert_eq!(status, Some(code), "{signal}");
        assert_eq!(left, ["in.dsf", "out.wav"], "{signal}");
        assert_eq!(
            std::fs::read(dir.join("out.wav")).unwrap(),
            PRECIOUS,
            "{signal}"
        );
    }
    // Without an existing output, nothing is left either.
    std::fs::remove_file(dir.join("out.wav")).unwrap();
    let (status, left) = interrupt(&dir, "-INT");
    assert_eq!(status, Some(130));
    assert_eq!(left, ["in.dsf"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
