[dsd2pcm-rs](../../README.md) › [Documentation](../../README.md#documentation) › Quality tools

# Quality measurement tools

The scripts behind [docs/pcm-quality.md](../../docs/pcm-quality.md). They measure DSD to
PCM decoders as black boxes: test signals are generated, encoded to DSD, decoded by each
decoder's own command and analysed from the output files. Nothing here links to the
dsd2pcm crate; the decoders are run by path.

| Script | In | Out |
|---|---|---|
| `signals.toml` | | The test signals: frequencies, levels, lengths, lead-in silence and ramps, PCM rates, DSD rates, modulators |
| `modulator.py` | | The reference 1-bit modulator (5th order, error feedback; its design is in the file) |
| `gen_signals.py` | `signals.toml` | PCM test files (24-bit WAV), DSD files (DSF) and `signals.json` (every tone and level as generated) |
| `decoders.toml` | | The decoders: command templates and settings |
| `decode.py` | DSF files | One WAV per decoder, setting and signal, and `runs.json` (commands, exit status, SHA-256 of inputs and outputs) |
| `postfilter.py` | WAV files | The same, through a separate reference linear-phase low-pass (for the 30 kHz comparison) |
| `analyse.py` | WAV files (and DSF sources) | `results.json` (metrics) and `curves.json` (plot data); the analysis settings are written into `results.json`. Alias tones are also related to each tone's level measured in the bitstream (pass the signal in `--sources`), and `loud_alias` is detected narrow-band |
| `filters.py` | `tables.rs`, `pysacd_taps.rs`, `tables_precise.rs` (`--precise`) | `filters.json`: the filters' exact response, from their taps; with `--apply`, the same filter in 64-bit floating point run on a DSF file (a reference decoder without rounding or word length; `--cutoff precise-40` and so on for the precise filter) |
| `plot.py` | JSON | SVG charts |
| `report.py` | JSON | The Markdown tables of the page |
| `sbpcm/` | raw DSD | scarletbook-pcm 0.7.0 as a command-line decoder (a separate Cargo project) |

## Quick start

From the repository root, on Linux, with Rust and Python 3 (tested with Python 3.15):

```sh
python3 -m venv venv
venv/bin/pip install -r tools/quality/requirements.txt
cargo build --release -p dsd2pcm-cli
cargo build --release --manifest-path tools/quality/sbpcm/Cargo.toml
venv/bin/python tools/quality/gen_signals.py --out out/sig --no-sox
venv/bin/python tools/quality/decode.py --signals-dir out/sig --out out/dec --tool dsd2pcm=target/release/dsd2pcm --tool sbpcm=tools/quality/sbpcm/target/release/sbpcm-ref
venv/bin/python tools/quality/postfilter.py --in out/dec/dsd2pcm/352k8-wide --out out/dec/lp30-ref/352k8
mkdir -p out/dec/taps-f64/176k4-30 out/dec/taps-f64/176k4-40 out/dec/taps-f64/176k4-wide out/dec/taps-f64/176k4-pysacd
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_sine1k-dsd-dsd64-ref.dsf --rate 176400 --cutoff 30 --out out/dec/taps-f64/176k4-30/quiet_sine1k-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_alias-dsd-dsd64-ref.dsf --rate 176400 --cutoff 30 --out out/dec/taps-f64/176k4-30/quiet_alias-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_sine1k-dsd-dsd64-ref.dsf --rate 176400 --cutoff 40 --out out/dec/taps-f64/176k4-40/quiet_sine1k-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_alias-dsd-dsd64-ref.dsf --rate 176400 --cutoff 40 --out out/dec/taps-f64/176k4-40/quiet_alias-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_sine1k-dsd-dsd64-ref.dsf --rate 176400 --cutoff wide --out out/dec/taps-f64/176k4-wide/quiet_sine1k-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_alias-dsd-dsd64-ref.dsf --rate 176400 --cutoff wide --out out/dec/taps-f64/176k4-wide/quiet_alias-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_sine1k-dsd-dsd64-ref.dsf --rate 176400 --cutoff pysacd --out out/dec/taps-f64/176k4-pysacd/quiet_sine1k-dsd-dsd64-ref.wav
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --apply out/sig/dsd/quiet_alias-dsd-dsd64-ref.dsf --rate 176400 --cutoff pysacd --out out/dec/taps-f64/176k4-pysacd/quiet_alias-dsd-dsd64-ref.wav
venv/bin/python tools/quality/analyse.py --signals-dir out/sig --decoded out/dec --out out/results.json --curves out/curves.json --sources idle,silence,sine1k_0,level_steps,smpte,ccif,fr_tones,xtalk_l,alias_steps,multitone,jtest,quiet_sine1k,quiet_alias
venv/bin/python tools/quality/filters.py --tables crates/dsd2pcm/src/decimator/tables.rs --pysacd crates/dsd2pcm/src/filters/pysacd_taps.rs --out out/filters.json
venv/bin/python tools/quality/plot.py --filters out/filters.json --results out/results.json --curves out/curves.json --out docs/assets/quality
venv/bin/python tools/quality/report.py --filters out/filters.json --results out/results.json
```

Generating the signals takes about half an hour on 8 cores (the modulator is pure Python);
everything needs about 12 GB of disk, most of it decoded WAVs. `--no-sox` leaves out the
SoX-DSD versions of three signals; to make them, build SoX-DSD (Måns Rullgård's DSD branch
of SoX, [mansr/sox](https://github.com/mansr/sox), GPL; it is only run, as a separate
program, never linked or copied) and pass `--sox` with the path of its `sox` program
instead.

Decoders run elsewhere (for example a Windows player) are measured the same way: put their
WAVs in `out/dec/<decoder>/<setting>/`, named as the DSF inputs with `.wav`, and run
`analyse.py`.

## See also

- [PCM quality](../../docs/pcm-quality.md): the results and what they mean
- [Design](../../docs/design.md): the converter being measured

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
