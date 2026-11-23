# dsd2pcm-rs

[![ci](https://github.com/aclater/dsd2pcm-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/aclater/dsd2pcm-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/dsd2pcm.svg)](https://crates.io/crates/dsd2pcm)
[![docs.rs](https://docs.rs/dsd2pcm/badge.svg)](https://docs.rs/dsd2pcm)

DSD to PCM conversion in Rust, with DSF, DFF and WAV reading and writing, and FLAC reading.
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

Speed against scarletbook-pcm 0.7.0, including where it is still slower: [docs/performance.md](docs/performance.md).

## What it does

- Reads DSF and DSDIFF (DFF, uncompressed; DST-compressed files are rejected with a clear
  error) at DSD64 to DSD1024, WAV (16/24/32-bit integer, 32/64-bit float) and FLAC
  (8/12/16/20/24-bit, 1 to 6 channels, optionally after an ID3v2 tag). A WAV
  `data` size left at 0 or 0xFFFFFFFF (streaming writers, some encoders past 4 GiB) reads
  to the end of the file; if other chunks follow it, the file is rejected. FLAC and WAV of
  the same samples read identically. FLAC reading is the cargo feature `flac` (on by
  default; `--no-default-features` builds without it and its one dependency, `claxon`).
- Writes DSF, DSDIFF and WAV. WAV files with more than 16 bits, float samples or more than two
  channels use `WAVE_FORMAT_EXTENSIBLE` with the channel mask of the channel count (SACD
  order: FL FR FC LFE BL BR).
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
The DSD idle pattern (0x69) converts to exact zeros. On x86 the filters use AVX2 and FMA
when the CPU has them, chosen at run time, with the same output; the environment variable
`DSD2PCM_SIMD` (`generic`, `avx2` or `avx2fma`) caps the level.

Options: `--gain DB` (-120 to +24, default 0), `--single-thread`, `--filter pysacd`.

`--filter pysacd` (`Filter::Pysacd` in `PcmOptions` and `frames::PcmConfig`) uses pysacd's
filter instead: three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered quantiser,
bit-exact with pysacd's reference build on every CPU and SIMD level; 88.2 and 176.4 kHz
only, with a fixed response (leave `--cutoff` at the default). The default filter and its
output are unchanged.

As a library, `dsd2pcm::frames::Converter` converts Super Audio CD frames (channels
interleaved byte by byte, 4704 bytes per channel per frame) a run at a time, with the frames
before and after the run as context, into interleaved 24-bit PCM, with clip counts per frame.
Its interface and output bytes are those of scarletbook-pcm 0.7.0's `Converter::convert`;
any run with its neighbours gives the bytes of that run in a whole-area conversion. One
converter per thread; after the first call it allocates nothing.

`convert_file_with` and `dsd64_to_pcm_with` take a progress hook: it gets the output
samples done and the total at each block, on the calling thread, and stops the conversion
with `Error::Cancelled` when it returns `ControlFlow::Break`. `Error` is `#[non_exhaustive]`.

Limits of this version: DSD64 input only for PCM output; WAV output up to 4 GiB.

## C interface

The crate `crates/dsd2pcm-capi` builds `libdsd2pcm.so` and `libdsd2pcm.a`, with the header
`crates/dsd2pcm-capi/include/dsd2pcm.h`. It offers whole-file conversion with a progress and
cancel callback, and a frame-run converter with the semantics of `dsd2pcm::frames`. Every
function returns a status code, checks its pointers and lengths and never lets a panic
reach the caller. The structs carry their size, so the ABI (version 1, SONAME
`libdsd2pcm.so.1`) can grow compatibly. See [docs/c-api.md](docs/c-api.md).

A minimal program, `crates/dsd2pcm-capi/tests/minimal.c`, converts one stereo frame of
DSD silence. To build and run it on Linux, from the repository root:

```
cargo build --release -p dsd2pcm-capi
cc -std=c11 -I crates/dsd2pcm-capi/include crates/dsd2pcm-capi/tests/minimal.c target/release/libdsd2pcm.a -lpthread -ldl -lm -o minimal
./minimal
```

It prints `dsd2pcm 0.1.0: 2352 samples per channel, first byte 0`. To link the shared
library instead (its SONAME is `libdsd2pcm.so.1`, so it needs that name at run time):

```
ln -sf libdsd2pcm.so target/release/libdsd2pcm.so.1
cc -std=c11 -I crates/dsd2pcm-capi/include crates/dsd2pcm-capi/tests/minimal.c -L target/release -ldsd2pcm -o minimal
LD_LIBRARY_PATH=target/release ./minimal
```

## Development and releases

CI (`.github/workflows`) runs fmt, clippy, the docs build and the tests on Linux (x86_64,
aarch64, i686), macOS (arm64) and Windows (x86_64), the tests once per SIMD level the CPU
has, plus the minimum Rust version, cargo-deny, a fuzz smoke test, CodeQL and zizmor. The
tests need Rust 1.85 or later (a test-only dependency uses edition 2024); the crates
themselves build with 1.83. Locally:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_PROFILE_DEV_OPT_LEVEL=2 .github/scripts/test-simd-levels.sh
```

Releases are built, tested and published by `.github/workflows/release.yml` from an
annotated `v*` tag; [docs/release.md](docs/release.md) lists what is published and the
manual steps.

## Licence

Dual-licensed under either of the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
or the MIT licence ([LICENSE-MIT](LICENSE-MIT)), at your option. Unless you state otherwise, any
contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0
licence, is dual-licensed as above, without any additional terms or conditions.
