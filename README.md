<img src="https://raw.githubusercontent.com/aclater/dsd2pcm-rs/main/docs/assets/icon-256.png" alt="" width="96">

# dsd2pcm-rs

[![CI](https://github.com/aclater/dsd2pcm-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/aclater/dsd2pcm-rs/actions/workflows/ci.yml)
[![CodeQL](https://github.com/aclater/dsd2pcm-rs/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/aclater/dsd2pcm-rs/actions/workflows/codeql.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#licence-and-credits)
[![MSRV 1.83](https://img.shields.io/badge/MSRV-1.83-orange)](Cargo.toml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/aclater/dsd2pcm-rs/badge)](https://scorecard.dev/viewer/?uri=github.com/aclater/dsd2pcm-rs)
[![crates.io](https://img.shields.io/crates/v/dsd2pcm.svg)](https://crates.io/crates/dsd2pcm)
[![docs.rs](https://docs.rs/dsd2pcm/badge.svg)](https://docs.rs/dsd2pcm)

dsd2pcm converts DSD (DSF and DSDIFF files, or Super Audio CD frames) to 24-bit PCM, as a Rust
crate, a C library and a command-line tool. Its output is the same, byte for byte, on every
CPU, and on most workloads it is faster than scarletbook-pcm 0.7.0, whose output it
reproduces ([performance](docs/performance.md)).

## Platforms

| System | How | Guide |
|---|---|---|
| Linux | Release builds of the tool and the C library for x86_64, aarch64 and 32-bit x86, glibc 2.17 or later; `cargo install dsd2pcm-cli` | [usage](docs/usage.md) · [C interface](docs/c-api.md) |
| macOS 11 or later | Release builds for Apple silicon (tool and C library), signed and notarised | [usage](docs/usage.md) · [C interface](docs/c-api.md) |
| Windows 10 and 11, x64 | Release builds (tool and C library, C runtime linked in). Not code-signed | [usage](docs/usage.md) · [C interface](docs/c-api.md) |
| Any Rust target | The `dsd2pcm` crate | [docs.rs](https://docs.rs/dsd2pcm) |

Every release is tested in CI on all five release platforms; the speed figures come from an
Intel Core i9-13900H (P- and E-cores) and an AMD Ryzen 9 3950X.

## Quick start

On Linux x86_64, with a DSF file named `01 - Track.dsf` in the current directory:

```
curl -LO https://github.com/aclater/dsd2pcm-rs/releases/download/v0.1.1/dsd2pcm-0.1.1-x86_64-linux.tar.gz
tar xzf dsd2pcm-0.1.1-x86_64-linux.tar.gz
./dsd2pcm-0.1.1-x86_64-linux/dsd2pcm "01 - Track.dsf" "01 - Track.wav"
```

This writes a 176.4 kHz, 24-bit WAV and prints the duration, the speed and the number of
clipped samples. With Rust installed, `cargo install dsd2pcm-cli` installs the same tool.
Other platforms and options: [docs/usage.md](docs/usage.md).

## Documentation

- [Usage](docs/usage.md): formats, rates, filters, options, the library interface
- [C interface](docs/c-api.md): the C library, its ABI and binary releases
- [PCM quality](docs/pcm-quality.md): measured audio quality of every filter and rate, and the decisions behind it, following [Archimago](https://archimago.blogspot.com/)'s DSD-to-PCM method, with [Audio Science Review](https://www.audiosciencereview.com/) and [Stereophile](https://www.stereophile.com/content/measurements-maps-precision) as references for measurement practice
- [Performance](docs/performance.md): speed against scarletbook-pcm 0.7.0, including the gaps
- [Design](docs/design.md): how the conversion works and why its output is exact
- [Changelog](CHANGELOG.md) · [Plan](docs/plan.md)
- [Development](docs/development.md) · [Fuzzing](docs/fuzzing.md) · [Testing for long-lived processes](docs/testing.md) · [Releases](docs/release.md)
- [Provenance](PROVENANCE.md) · [Rules for contributors](AGENTS.md)
- [Security policy](SECURITY.md): supported versions and how to report a vulnerability

## Licence and credits

Dual-licensed under either of the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
or the MIT licence ([LICENSE-MIT](LICENSE-MIT)), at your option. Unless you state otherwise, any
contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0
licence, is dual-licensed as above, without any additional terms or conditions.

Credits ([PROVENANCE.md](PROVENANCE.md) has the details):

- [scarletbook-pcm](https://crates.io/crates/scarletbook-pcm) 0.7.0 (MIT OR Apache-2.0): the
  default filters, their taps and the frame-run interface are ported from it.
- [pysacd](https://github.com/aclater/pysacd) (filter code relicensed MIT OR Apache-2.0 by its
  copyright holder): the opt-in `pysacd` filter.
- Sebastian Gesemann's dsd2pcm (BSD-3-Clause): the byte lookup-table technique of the
  `pysacd` filter's first stage; no code taken.
- [claxon](https://crates.io/crates/claxon) (Apache-2.0): FLAC decoding.

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
