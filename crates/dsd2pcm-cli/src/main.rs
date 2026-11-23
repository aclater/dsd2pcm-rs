//! `dsd2pcm IN.dsf|IN.dff OUT.wav [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|pysacd] [--single-thread] [-n|--no-clobber]`

use dsd2pcm::{convert_file_to, Cutoff, Filter, OutputOptions, PcmOptions, PcmRate};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

const USAGE: &str = "usage: dsd2pcm IN.dsf|IN.dff OUT.wav [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|pysacd] [--single-thread] [-n|--no-clobber]

Converts DSD64 to 24-bit PCM WAV. Defaults: --rate 176400 --cutoff 40 --gain 0.
Cutoffs: 30 (pass 30 kHz, stop 36 kHz), 40 (pass 40 kHz, stop 48 kHz),
wide (pass 80 kHz; 176.4 and 352.8 kHz only).
Filters: default (scarletbook-pcm 0.7.0's), pysacd (pysacd's filter and dithered
quantiser, bit-exact with its reference; 88200 and 176400 only, fixed response).
--single-thread: convert on the calling thread only (default: all cores).
-n, --no-clobber: fail if OUT exists, and never replace a file that appears at OUT
during the conversion (default: replace OUT).";

fn parse(args: &[String]) -> Result<(PathBuf, PathBuf, PcmOptions, OutputOptions), String> {
    let mut pos = Vec::new();
    let mut opts = PcmOptions::new(PcmRate::Hz176400);
    let mut out_opts = OutputOptions::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--rate" => {
                let v = val("--rate")?;
                let hz: u32 = v.parse().map_err(|_| format!("bad rate {v}"))?;
                opts.rate = PcmRate::from_hz(hz)
                    .ok_or_else(|| format!("rate {hz}: 88200, 176400 or 352800"))?;
            }
            "--cutoff" => {
                opts.cutoff = match val("--cutoff")?.as_str() {
                    "30" => Cutoff::Low,
                    "40" => Cutoff::Standard,
                    "wide" | "80" => Cutoff::Wide,
                    v => return Err(format!("cutoff {v}: 30, 40 or wide")),
                }
            }
            "--gain" => {
                let v = val("--gain")?;
                opts.gain_db = v.parse().map_err(|_| format!("bad gain {v}"))?;
            }
            "--filter" => {
                opts.filter = match val("--filter")?.as_str() {
                    "default" => Filter::Default,
                    "pysacd" => Filter::Pysacd,
                    v => return Err(format!("filter {v}: default or pysacd")),
                }
            }
            "--single-thread" => opts.threads = false,
            "-n" | "--no-clobber" => out_opts.no_clobber = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (inp, out, opts, out_opts) = match parse(&args) {
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
