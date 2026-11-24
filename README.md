<img src="https://raw.githubusercontent.com/dsd-toolbox/dsd2pcm-rs/main/docs/assets/icon-256.png" alt="" width="96">

# dsd2pcm-rs

[![CI](https://github.com/dsd-toolbox/dsd2pcm-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/dsd-toolbox/dsd2pcm-rs/actions/workflows/ci.yml)
[![CodeQL](https://github.com/dsd-toolbox/dsd2pcm-rs/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/dsd-toolbox/dsd2pcm-rs/actions/workflows/codeql.yml)
<!-- TODO: OpenSSF Scorecard badge (https://api.scorecard.dev/projects/github.com/dsd-toolbox/dsd2pcm-rs/badge) once scorecard.yml has run on main under dsd-toolbox and that URL returns 200; on 2026-11-24 it returned 404 ("invalid repo path"). -->
[![crates.io](https://img.shields.io/crates/v/dsd2pcm.svg)](https://crates.io/crates/dsd2pcm)
[![docs.rs](https://docs.rs/dsd2pcm/badge.svg)](https://docs.rs/dsd2pcm)
[![MSRV 1.83](https://img.shields.io/badge/MSRV-1.83-orange)](Cargo.toml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence-and-credits)

dsd2pcm converts DSD (DSF and DFF files, or SACD frames) to 24-bit PCM, as a Rust
crate, a C library and a command-line tool.

- **Quality first.** The default filter, `precise`, has 150 dB or more of stopband wherever
  ultrasonic content folds into the audio band. With a very quiet DSD64 source it measures a
  SINAD of 142 to 145 dB, the limit of 24-bit output, and every tone that folds into the
  audio band is rejected by 151 dB or more
  ([PCM quality](docs/pcm-quality.md#the-precise-filter)). It takes 1.3 to 1.9 times the
  CPU time of scarletbook-pcm 0.7.0 on an Intel Core i9-13900H
  ([performance](docs/performance.md)).
- **`fast` for lower CPU, at lower quality.** `--filter fast` is scarletbook-pcm 0.7.0's
  filter and byte-identical to it. On x86-64 it runs at 1.11 to 1.25 times scarletbook-pcm's
  speed (Intel Core i9-13900H); on aarch64 the two are level. Its stopband limits SINAD to
  122 dB (116 dB with `wide`) with the same quiet source, and alias rejection to 111 to 115 dB.
- **Deterministic.** Within a release, the same input and settings give the same bytes on
  every CPU and SIMD level (with SSE2 floating point on x86). Output may change between
  releases when measured quality improves; each change is in the [changelog](CHANGELOG.md)
  with its measured effect ([release quality comparison](docs/quality/README.md)).

## Platforms

| System | How | Guide |
|---|---|---|
| Linux | Release builds of the tool and the C library for x86_64, aarch64 and 32-bit x86, glibc 2.17 or later; from 0.2.0 also for x86_64 and aarch64 with musl (a static tool; the C library for musl 1.2.3 or later, Alpine 3.17 or later); `cargo install dsd2pcm-cli` | [user guide](docs/user-guide.md) · [C interface](docs/c-api.md) |
| macOS 11 or later | Release builds for Apple silicon (tool and C library), from 0.2.0 also for Intel, signed and notarised | [user guide](docs/user-guide.md) · [C interface](docs/c-api.md) |
| Windows 10 and 11, x64 | Release builds (tool and C library, C runtime linked in); from 0.2.0 also the C library for MinGW-w64 gcc. Not code-signed | [user guide](docs/user-guide.md) · [C interface](docs/c-api.md) |
| Any Rust target | The `dsd2pcm` crate | [docs.rs](https://docs.rs/dsd2pcm) |

Every release is tested in CI on all its release platforms (nine from 0.2.0 on); the speed figures come from an
Intel Core i9-13900H (P- and E-cores) and an AMD Ryzen 9 3950X.

## Quick start

On Linux x86_64, with a DSF file named `01 - Track.dsf` in the current directory:

```sh
curl -LO https://github.com/dsd-toolbox/dsd2pcm-rs/releases/download/v0.1.2/dsd2pcm-0.1.2-x86_64-linux.tar.gz
tar xzf dsd2pcm-0.1.2-x86_64-linux.tar.gz
./dsd2pcm-0.1.2-x86_64-linux/dsd2pcm "01 - Track.dsf" "01 - Track.wav"
```

This writes a 176.4 kHz, 24-bit WAV and prints the duration, the speed and the number of
clipped samples. With Rust installed, `cargo install dsd2pcm-cli` installs the same tool.
Options: [docs/options.md](docs/options.md); more in the [user guide](docs/user-guide.md).

### Windows

On Windows 10 or 11, x86_64, in PowerShell, with the same DSF file in the current folder:

<!-- docs-run: skip="Windows" -->
```powershell
Invoke-WebRequest https://github.com/dsd-toolbox/dsd2pcm-rs/releases/download/v0.1.2/dsd2pcm-0.1.2-x86_64-windows.zip -OutFile dsd2pcm-0.1.2-x86_64-windows.zip
Expand-Archive dsd2pcm-0.1.2-x86_64-windows.zip -DestinationPath .
.\dsd2pcm-0.1.2-x86_64-windows\dsd2pcm.exe "01 - Track.dsf" "01 - Track.wav"
```

The archive is also on the [release page](https://github.com/dsd-toolbox/dsd2pcm-rs/releases/tag/v0.1.2).
`dsd2pcm.exe` is not code-signed, so Windows SmartScreen may warn when it first runs; choose
**More info**, then **Run anyway**. The [user guide](docs/user-guide.md#windows) covers the
`PATH`, quoting and double-clicking.

## Documentation

The [documentation index](docs/README.md) lists every page.

- [User guide](docs/user-guide.md) · [Options](docs/options.md): converting files, rates, filters, formats
- [C interface](docs/c-api.md): the C library, its ABI and binary releases
- [PCM quality](docs/pcm-quality.md): measured audio quality of every filter and rate, and the decisions behind it, following [Archimago](https://archimago.blogspot.com/)'s DSD-to-PCM method, with [Audio Science Review](https://www.audiosciencereview.com/) and [Stereophile](https://www.stereophile.com/content/measurements-maps-precision) as references for measurement practice
- [Performance](docs/performance.md): speed of `precise` and `fast` against scarletbook-pcm 0.7.0, including the gaps
- [Release quality comparison](docs/quality/README.md): each release measured against the previous one
- [Design](docs/design.md): the design decisions and their reasons
- [Changelog](CHANGELOG.md) · [Plan](docs/plan.md)
- [Contributing](CONTRIBUTING.md) · [Development](docs/development.md) · [Fuzzing](docs/fuzzing.md) · [Testing for long-lived processes](docs/testing.md) · [Releases](docs/release.md)
- [Security policy](SECURITY.md) · [Provenance](PROVENANCE.md) · [Rules for contributors](AGENTS.md)

## Licence and credits

**Code** is dual-licensed under either of the Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or the MIT licence ([LICENSE-MIT](LICENSE-MIT)), at your
option. [NOTICE](NOTICE) holds the attributions to keep with it. Unless you state otherwise, any
code contribution you intentionally submit for inclusion in this work, as defined in the
Apache-2.0 licence, is dual-licensed as above, without any additional terms or conditions.

**Documentation, measurement data and creative assets** are licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
([LICENSES/CC-BY-4.0.txt](LICENSES/CC-BY-4.0.txt)): every README (this one included), the
pages under `docs/` (the methodology and the quality pages, `docs/quality/`, included), the
icons and charts under `docs/assets/`, the measurement suite `suites/`, and the signal and
decoder definitions of `tools/quality/`. Code examples and commands in these files may also be
used under the code licence. Not covered: facts and figures cited from other sources, which
keep their own terms. Suggested attribution:
"dsd2pcm documentation, the dsd2pcm-rs authors, https://github.com/dsd-toolbox/dsd2pcm-rs,
CC BY 4.0". Contributions to these files are licensed the same way.
[CITATION.cff](CITATION.cff) says how to cite the software.

Credits ([PROVENANCE.md](PROVENANCE.md) has the details):

- [scarletbook-pcm](https://crates.io/crates/scarletbook-pcm) 0.7.0 (MIT OR Apache-2.0): the
  `fast` filter (the default up to 0.1.2), its taps and the frame-run interface are ported
  from it.
- [pysacd](https://github.com/dsd-toolbox/pysacd) (filter code relicensed MIT OR Apache-2.0 by its
  copyright holder): the opt-in `pysacd` filter.
- Sebastian Gesemann's dsd2pcm (BSD-3-Clause): the byte lookup-table technique of the
  `pysacd` filter's first stage; no code taken.
- [claxon](https://crates.io/crates/claxon) (Apache-2.0): FLAC decoding.

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
