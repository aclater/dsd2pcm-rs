//! The progress hook of `dsd64_to_pcm_with` and `convert_file_with`: called before the
//! first block and after each block with the samples done, the output does not change, and
//! `Break` stops the conversion with `Error::Cancelled`.

use dsd2pcm::convert::{dsd64_to_pcm, dsd64_to_pcm_with};
use dsd2pcm::dsf::{DsfReader, DsfWriter};
use dsd2pcm::io::DsdWrite;
use dsd2pcm::wav::{WavFormat, WavWriter};
use dsd2pcm::{
    convert_file, convert_file_with, Error, Filter, PcmOptions, PcmRate, Progress, Threads,
};
use std::io::Cursor;
use std::ops::ControlFlow;

/// A stereo DSF of `bytes` bytes per channel, in memory.
fn dsf(bytes: usize) -> Vec<u8> {
    let mut s = 0x1234_5678u64;
    let chans: Vec<Vec<u8>> = (0..2)
        .map(|_| {
            (0..bytes)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    s as u8
                })
                .collect()
        })
        .collect();
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), 2, 2_822_400, None).unwrap();
    w.write_bytes(&chans).unwrap();
    w.finish(bytes as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

fn run(
    file: &[u8],
    opts: &PcmOptions,
    hook: Option<&mut dyn FnMut(Progress) -> ControlFlow<()>>,
) -> dsd2pcm::Result<Vec<u8>> {
    let r = DsfReader::new(Cursor::new(file.to_vec())).unwrap();
    let mut w = WavWriter::new(Cursor::new(Vec::new()), WavFormat::I24, 2, opts.rate.hz())?;
    match hook {
        Some(h) => dsd64_to_pcm_with(Box::new(r), &mut w, opts, h)?,
        None => dsd64_to_pcm(Box::new(r), &mut w, opts)?,
    };
    w.finish()?;
    Ok(w.into_inner().into_inner())
}

#[test]
fn progress_is_reported_and_output_unchanged() {
    let file = dsf(300_000);
    for (threads, filter) in [
        (false, Filter::Precise),
        (true, Filter::Precise),
        (false, Filter::Fast),
        (true, Filter::Fast),
        (false, Filter::Pysacd),
        (true, Filter::Pysacd),
    ] {
        let mut opts = PcmOptions::new(PcmRate::Hz176400);
        opts.threads = if threads {
            Threads::All
        } else {
            Threads::Single
        };
        opts.filter = filter;
        let plain = run(&file, &opts, None).unwrap();
        let mut seen = Vec::new();
        let mut hook = |p: Progress| {
            seen.push((p.done, p.total));
            ControlFlow::Continue(())
        };
        let hooked = run(&file, &opts, Some(&mut hook)).unwrap();
        assert!(plain == hooked);
        let total = 300_000 * 8 / 16;
        assert_eq!(seen[0], (0, total));
        assert_eq!(*seen.last().unwrap(), (total, total));
        assert!(seen.len() >= 3, "{filter:?} {seen:?}");
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0 && w[1].1 == total));
    }
}

/// Every thread setting gives the same bytes, with every filter: all cores, one thread,
/// counts of 2, 3 and 7, and a count above `Threads::MAX` (capped).
#[test]
fn thread_counts_give_the_same_bytes() {
    use std::num::NonZeroUsize;
    let file = dsf(300_000);
    for filter in [Filter::Fast, Filter::Precise, Filter::Pysacd] {
        let opts = PcmOptions::new(PcmRate::Hz176400).with_filter(filter);
        let one = run(&file, &opts.with_threads(Threads::Single), None).unwrap();
        for t in [
            Threads::All,
            Threads::Count(NonZeroUsize::MIN),
            Threads::Count(NonZeroUsize::new(2).unwrap()),
            Threads::Count(NonZeroUsize::new(3).unwrap()),
            Threads::Count(NonZeroUsize::new(7).unwrap()),
            Threads::Count(NonZeroUsize::new(Threads::MAX + 1).unwrap()),
        ] {
            let got = run(&file, &opts.with_threads(t), None).unwrap();
            assert!(got == one, "{filter:?} {t:?}");
        }
    }
}

#[test]
fn break_cancels() {
    let file = dsf(300_000);
    for (filter, stop) in [
        (Filter::Precise, 0usize),
        (Filter::Precise, 1),
        (Filter::Fast, 0),
        (Filter::Fast, 1),
        (Filter::Pysacd, 0),
        (Filter::Pysacd, 1),
    ] {
        let mut opts = PcmOptions::new(PcmRate::Hz88200);
        opts.filter = filter;
        let mut calls = 0;
        let mut hook = |_: Progress| {
            calls += 1;
            if calls > stop {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let r = run(&file, &opts, Some(&mut hook));
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        assert_eq!(calls, stop + 1);
    }
}

#[test]
fn convert_file_with_hook() {
    let dir = std::env::temp_dir().join(format!("dsd2pcm-progress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (inp, a, b) = (dir.join("in.dsf"), dir.join("a.wav"), dir.join("b.wav"));
    std::fs::write(&inp, dsf(100_000)).unwrap();
    let opts = PcmOptions::new(PcmRate::Hz352800);
    let mut last = None;
    let rep = convert_file_with(&inp, &a, &opts, &mut |p| {
        last = Some(p);
        ControlFlow::Continue(())
    })
    .unwrap();
    convert_file(&inp, &b, &opts).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let last = last.unwrap();
    assert_eq!((last.done, last.total), (rep.out_samples, rep.out_samples));
    let r = convert_file_with(&inp, &a, &opts, &mut |_| ControlFlow::Break(()));
    assert!(matches!(r, Err(Error::Cancelled)));
    assert_eq!(Error::Cancelled.to_string(), "conversion cancelled");
    std::fs::remove_dir_all(&dir).unwrap();
}
