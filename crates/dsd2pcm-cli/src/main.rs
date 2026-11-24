//! `dsd2pcm [OPTIONS] [--] IN.dsf|IN.dff OUT.wav`

use dsd2pcm::{convert_file_to, Cutoff, Filter, OutputOptions, PcmOptions, PcmRate, Threads};
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

const USAGE: &str = "usage: dsd2pcm [OPTIONS] [--] IN.dsf|IN.dff OUT.wav
options: [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|precise|pysacd] [--single-thread] [-n|--no-clobber]

Converts DSD64 to 24-bit PCM WAV. Defaults: --rate 176400 --cutoff 40 --gain 0.
Cutoffs: 30 (pass 30 kHz, stop 36 kHz), 40 (pass 40 kHz, stop 48 kHz),
wide (pass 80 kHz; 176.4 and 352.8 kHz only).
Filters: default (scarletbook-pcm 0.7.0's), precise (150 dB stopbands, every rate and
cutoff, more CPU), pysacd (pysacd's filter and dithered quantiser, bit-exact with its
reference; 88200 and 176400 only, fixed response).
--single-thread: convert on the calling thread only (default: all cores).
-n, --no-clobber: fail if OUT exists, and never replace a file that appears at OUT
during the conversion (default: replace OUT).
--: every later argument is a file name, even one that starts with '-'.
File names may be in any encoding the system allows.";

/// Parses the arguments (without the program name). File names stay `OsString`s, so any
/// name the operating system allows works; option values must be UTF-8. Messages quote
/// argument text with `{:?}`, so control characters in it reach the terminal escaped.
/// `Err("")` asks for the usage on standard output.
fn parse(args: Vec<OsString>) -> Result<(PathBuf, PathBuf, PcmOptions, OutputOptions), String> {
    let mut pos = Vec::new();
    let mut opts = PcmOptions::new(PcmRate::Hz176400);
    let mut out_opts = OutputOptions::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        // A name that is not UTF-8 is never an option.
        let Some(s) = a.to_str() else {
            pos.push(PathBuf::from(a));
            continue;
        };
        let mut val = |name: &str| -> Result<String, String> {
            let v = it.next().ok_or_else(|| format!("{name} needs a value"))?;
            v.into_string().map_err(|v| {
                format!(
                    "{name}: the value {:?} is not valid UTF-8",
                    v.to_string_lossy()
                )
            })
        };
        match s {
            "--rate" => {
                let v = val("--rate")?;
                let hz: u32 = v.parse().map_err(|_| format!("bad rate {v:?}"))?;
                opts.rate = PcmRate::from_hz(hz)
                    .ok_or_else(|| format!("rate {hz}: 88200, 176400 or 352800"))?;
            }
            "--cutoff" => {
                opts.cutoff = match val("--cutoff")?.as_str() {
                    "30" => Cutoff::Low,
                    "40" => Cutoff::Standard,
                    "wide" | "80" => Cutoff::Wide,
                    v => return Err(format!("cutoff {v:?}: 30, 40 or wide")),
                }
            }
            "--gain" => {
                let v = val("--gain")?;
                opts.gain_db = v.parse().map_err(|_| format!("bad gain {v:?}"))?;
            }
            "--filter" => {
                opts.filter = match val("--filter")?.as_str() {
                    "default" => Filter::Default,
                    "precise" => Filter::Precise,
                    "pysacd" => Filter::Pysacd,
                    v => return Err(format!("filter {v:?}: default, precise or pysacd")),
                }
            }
            "--single-thread" => opts.threads = Threads::Single,
            "-n" | "--no-clobber" => out_opts.no_clobber = true,
            "-h" | "--help" => return Err(String::new()),
            "--" => {
                pos.extend(it.by_ref().map(PathBuf::from));
                break;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!(
                    "unknown option {s:?} (put -- before file names that start with '-')"
                ))
            }
            _ => pos.push(PathBuf::from(a)),
        }
    }
    if pos.len() != 2 {
        return Err("need IN and OUT".into());
    }
    let out = pos.pop().unwrap_or_default();
    let inp = pos.pop().unwrap_or_default();
    Ok((inp, out, opts, out_opts))
}

fn main() -> ExitCode {
    let (inp, out, opts, out_opts) = match parse(std::env::args_os().skip(1).collect()) {
        Ok(v) => v,
        Err(e) if e.is_empty() => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("dsd2pcm: {e}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let signal = watch_signals();
    let t = Instant::now();
    let mut hook = |_| {
        if signal.load(Ordering::Relaxed) == 0 {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    };
    match convert_file_to(&inp, &out, &opts, &out_opts, &mut hook) {
        Ok(rep) => {
            let secs = t.elapsed().as_secs_f64();
            let total: u64 = rep.clipped.iter().sum();
            eprintln!(
                "{} Hz -> {} Hz, {:.2} s of audio in {:.2} s ({:.1}x real time), clipped samples: {} {:?}",
                rep.in_rate,
                rep.out_rate,
                rep.seconds,
                secs,
                rep.seconds / secs.max(1e-9),
                total,
                rep.clipped
            );
            ExitCode::SUCCESS
        }
        Err(dsd2pcm::Error::Cancelled) if signal.load(Ordering::Relaxed) != 0 => {
            eprintln!("dsd2pcm: interrupted; no output written");
            ExitCode::from(exit_status(signal.load(Ordering::Relaxed)))
        }
        Err(e) => {
            eprintln!("dsd2pcm: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The signals that stop a conversion cleanly: SIGINT and SIGTERM (on Windows also Ctrl-Break,
/// as SIGBREAK). The first one sets the returned value to the signal number, and the progress
/// hook then cancels the conversion, which removes the temporary output file. A second one
/// ends the process at once with the same exit status. Registration failures are ignored:
/// the default action (ending the process) then applies.
fn watch_signals() -> Arc<AtomicUsize> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    let got = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(windows)]
    let signals = [SIGINT, SIGTERM, signal_hook::consts::SIGBREAK];
    #[cfg(not(windows))]
    let signals = [SIGINT, SIGTERM];
    for sig in signals {
        let status = i32::from(exit_status(sig as usize));
        // Registered first, so it sees the flag the first signal sets only from the second
        // signal on.
        let _ = signal_hook::flag::register_conditional_shutdown(sig, status, first.clone());
        let _ = signal_hook::flag::register(sig, first.clone());
        let _ = signal_hook::flag::register_usize(sig, got.clone(), sig as usize);
    }
    got
}

/// The shell's status for a process ended by a signal: 128 + the signal number (130 for
/// SIGINT, 143 for SIGTERM).
fn exit_status(sig: usize) -> u8 {
    u8::try_from(128 + sig).unwrap_or(128)
}
