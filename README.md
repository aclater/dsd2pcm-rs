# dsd2pcm-rs

DSD to PCM conversion in Rust, with DSF, DFF and WAV reading and writing.
It ships as a Rust crate (`dsd2pcm`), a C library and a command-line tool.

Status: in development, nothing released yet. dsd2pcm-rs is an independent library.

## Quick start

Needs Rust 1.83 or later.

```
cargo build --release
./target/release/dsd2pcm "01 - Track.dsf" track.wav --rate 176400 --cutoff 40
```

The tool prints the duration, the speed as a multiple of real time and the number of
samples clamped to 24 bits.

## What it does

- Reads DSF and DSDIFF (DFF, uncompressed; DST-compressed files are rejected with a clear
  error) at DSD64 to DSD1024, and WAV (16/24/32-bit integer, 32/64-bit float).
- Writes DSF, DSDIFF and WAV.
- Converts DSD64 to 24-bit PCM at 88.2, 176.4 or 352.8 kHz with one of three linear-phase
  filters:

| `--cutoff` | Passband | Stopband from | Rates |
|---|---|---|---|
| `40` (default) | 40 kHz | 48 kHz | all |
| `30` | 30 kHz | 36 kHz | all |
| `wide` | 80 kHz | 88.2 kHz (176.4 kHz), 96 kHz (352.8 kHz) | 176.4 and 352.8 kHz |

The output samples are identical to those of scarletbook-pcm 0.7.0 for the same rate,
cutoff and gain; the tests compare the two on synthetic and real input. Every sample is an
exact integer computation, so the output is the same on every CPU and build profile.
The DSD idle pattern (0x69) converts to exact zeros.

Options: `--gain DB` (-120 to +24, default 0), `--single-thread`.

Limits of this version: DSD64 input only for PCM output; WAV output up to 4 GiB.

## Licence

Dual-licensed under either of the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
or the MIT licence ([LICENSE-MIT](LICENSE-MIT)), at your option. Unless you state otherwise, any
contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0
licence, is dual-licensed as above, without any additional terms or conditions.
