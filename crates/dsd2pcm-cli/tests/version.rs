//! `--version`: the version and the SIMD level the CLI selected, at every level this CPU has.

use dsd2pcm::decimator::SimdLevel;
use std::process::{Command, Output};

fn run(args: &[&str], simd: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_dsd2pcm"));
    c.args(args);
    match simd {
        Some(v) => c.env("DSD2PCM_SIMD", v),
        None => c.env_remove("DSD2PCM_SIMD"),
    };
    c.output().unwrap()
}

fn lines(o: &Output) -> Vec<String> {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(o.stderr.is_empty());
    String::from_utf8(o.stdout.clone())
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

fn level_of(o: &Output) -> String {
    let l = lines(o);
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[0], format!("dsd2pcm {}", env!("CARGO_PKG_VERSION")));
    l[1].strip_prefix("simd: ").expect("simd: line").to_string()
}

#[test]
fn version_and_short_option_agree() {
    let a = run(&["--version"], None);
    let b = run(&["-V"], None);
    assert_eq!(a.stdout, b.stdout);
    let name = level_of(&a);
    // What the library says in this process (the test runs without the variable too).
    let best = SimdLevel::available().last().copied().unwrap();
    assert_eq!(
        name,
        format!("{best:?}").to_ascii_lowercase(),
        "unforced: the best level this CPU has"
    );
}

#[test]
fn reports_each_forced_level() {
    for l in SimdLevel::available() {
        let want = format!("{l:?}").to_ascii_lowercase();
        let o = run(&["--version"], Some(&want));
        assert_eq!(level_of(&o), want);
    }
}

#[test]
fn a_cap_above_this_cpu_or_unknown_changes_nothing() {
    let best = level_of(&run(&["--version"], None));
    // Unknown words are ignored by the library, as they always were.
    assert_eq!(level_of(&run(&["--version"], Some("bogus"))), best);
    assert_eq!(level_of(&run(&["--version"], Some("avx2fma"))), best);
}

#[test]
fn needs_no_files_and_double_dash_makes_it_a_name() {
    // Extra arguments do not stop it, as with --help.
    assert!(!level_of(&run(&["--version", "a.dsf", "b.wav"], None)).is_empty());
    // After `--` it is a file name (missing: exit 1, nothing on stdout).
    let o = run(&["--", "--version", "out.wav"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
}

#[test]
fn usage_names_it() {
    let o = run(&["--help"], None);
    assert!(String::from_utf8_lossy(&o.stdout).contains("-V|--version"));
}
