//! `-n` / `--no-clobber`: an existing output is refused (exit status 1) and left as it was;
//! without it, it is replaced.

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

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dsd2pcm"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

const PRECIOUS: &[u8] = b"an existing output that --no-clobber must not touch";

#[test]
fn no_clobber() {
    let d: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-no-clobber");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    small_dsf(&d.join("in.dsf"));
    for flag in ["-n", "--no-clobber"] {
        std::fs::write(d.join("out.wav"), PRECIOUS).unwrap();
        let o = run(&d, &["in.dsf", "out.wav", flag]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{flag}: {err}");
        assert!(err.contains("no-clobber"), "{flag}: {err}");
        assert_eq!(std::fs::read(d.join("out.wav")).unwrap(), PRECIOUS);
        // A new output is written.
        let o = run(&d, &[flag, "in.dsf", "new.wav"]);
        assert_eq!(o.status.code(), Some(0), "{flag}");
        assert_eq!(&std::fs::read(d.join("new.wav")).unwrap()[..4], b"RIFF");
        std::fs::remove_file(d.join("new.wav")).unwrap();
    }
    // Without the option the output is replaced.
    let o = run(&d, &["in.dsf", "out.wav"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(&std::fs::read(d.join("out.wav")).unwrap()[..4], b"RIFF");
    std::fs::remove_dir_all(&d).unwrap();
}
