# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses
[semantic versioning](https://semver.org/). Output is deterministic within a release (the
same version, input and settings give the same bytes on every CPU and SIMD level); between
releases it may change when measured quality improves, and every such change is listed here
with its measured effect. Releases up to 0.1.2 keep their behaviour as released.

## Unreleased

## 0.2.0 - 2026-11-24

Quality: [docs/quality/0.2.0.md](docs/quality/0.2.0.md), every filter measured against 0.1.2
with dsd-measure 0.1.2.

### Changed

#### Breaking changes (Rust API)

Each with what to change in a caller. The C interface stays ABI version 1, with additions only.

- `Filter::Default` is removed; `Filter::Fast` is that filter (0.1.x's default), and
  `Filter::Precise` is now `Filter::default()`, so `PcmOptions::new` and
  `frames::PcmConfig::new` select it (see Output below). Migration: use `Filter::Fast` where
  `Filter::Default` was named to keep its output. The variants name the filters, not which
  one is the default, so a later change of default renames nothing.
  `decimator::Decimator` keeps scarletbook-pcm's filters.
- `Error::Format`, `Error::Unsupported` and `Error::InvalidArgument` hold `FormatError`,
  `UnsupportedError` and `ArgumentError` instead of a `String`: a kind to match on
  (`FormatErrorKind`: `NotRecognised`, `Truncated`, `Malformed`, `LimitExceeded`;
  `UnsupportedKind`; `ArgumentKind`) and the same message. `Display` of `Error` is
  unchanged. Migration: `Error::Format(m)` binds the details; use `m.message()` where the
  string was used, or match on `m.kind()`.
- `PcmOptions::threads` is a `Threads` (`All`, `Single`, `Count(NonZeroUsize)`) instead of
  a `bool`. Migration: `true` is `Threads::All`, `false` is `Threads::Single`.
- `#[non_exhaustive]` on `PcmOptions`, `PcmConfig`, `Report`, `ConvertStats`, `FrameRun`,
  `frames::Run`, `DsdInfo`, `PcmInfo`, and the enums `PcmRate`, `Cutoff`, `WavFormat`,
  `PcmOut` and `SimdLevel`, so that later versions can add fields and variants.
  Migration: build `PcmOptions` with `PcmOptions::new(rate)` and `with_cutoff`,
  `with_gain_db`, `with_threads`, `with_filter` (new), or set fields on that value;
  `PcmConfig`, `FrameRun` and `Run` with their existing `new` and `with_*`; `DsdInfo` and
  `PcmInfo` with the new `DsdInfo::new` and `PcmInfo::new`; add a wildcard arm to a
  `match` on those enums.
- `io::DsdWrite` and `io::PcmRead` are sealed (implemented by the crate's writers and
  readers only), so methods can be added to them. Migration: none for callers; a type
  outside the crate can no longer implement them. `io::DsdRead` stays open, since
  `dsd64_to_pcm` converts callers' own sources.

#### Output

- Output stability rule: output is deterministic within a release (the same version, input
  and settings give the same bytes on every CPU and SIMD level). Between releases it may
  change when measured quality improves; every such change is listed in this changelog with
  its measured effect (`docs/quality/<version>.md`). Releases up to 0.1.2 keep their
  behaviour as released. This replaces the rule that output bytes change only with a major
  version ([design](docs/design.md#the-contract-deterministic-output-the-deepest-stopband-filter-by-default),
  [AGENTS.md](AGENTS.md)).
- **The default filter is now `precise`; 0.1.x's default stays available as `fast`.**
  A conversion with default settings gives other bytes than 0.1.x: SINAD with a very quiet
  DSD64 source rises from about 122 dB to 142 to 145 dB (116 to 144 dB with `wide`), at
  about 1.8 times the CPU time on one core (1.5 to 2.3 times;
  [PCM quality](docs/pcm-quality.md#the-precise-filter),
  [performance](docs/performance.md#precise-against-fast-by-rate-and-cutoff)). `fast`
  (scarletbook-pcm 0.7.0's filters) is lower CPU, lower quality, and gives the bytes of
  0.1.x's default and of scarletbook-pcm 0.7.0, unchanged.
  - Command line: `--filter precise|fast|pysacd`, default `precise`. `--filter default` is
    still accepted and means precise, as `DSD2PCM_FILTER_DEFAULT` does, so 0.1.x scripts
    keep running; add `--filter fast` for 0.1.x's bytes. The help gives each filter's
    trade-off.
  - C interface (ABI version 1, additions only): new `DSD2PCM_FILTER_FAST` (3).
    `DSD2PCM_FILTER_DEFAULT` keeps its value, 0, and now selects the precise filter, so
    zero-initialised options (the `*_INIT` macros) do too; the other constants and every
    struct are unchanged.
  - Rust: see the breaking changes above.
- `Filter::Pysacd` (`--filter pysacd`) computes pysacd's filter in `f64`: SINAD +0.72 /
  +0.25 dB at 88.2 / 176.4 kHz (now 127.97 / 128.13 dB, above pysacd 0.5.x's 127.81 /
  128.07 dB; dsd-measure 0.1.1, 1 kHz at 0 dB SACD). Output bytes change; they are the
  same on every CPU and SIMD level. pysacd's taps, response, dither and seeding are
  unchanged. A conversion on one core takes 1.2 to 1.3 times as long at the AVX2 level and
  1.5 to 1.8 times at the generic level ([design](docs/design.md#the-pysacd-preset)).
- Output: DSD to WAV conversion writes the channel mask of the input's layout (DSF channel
  type, DSDIFF channel IDs) instead of the one of its channel count. The WAV header changes
  only where the two differ: a DSF of type 5 (FL FR C LFE) was labelled quad (FL FR BL BR),
  and DSDIFF files with other speaker sets, such as FL FR LFE, were labelled by their
  count. The samples are unchanged.

#### Other changes

- Docs, methodology and measurement data are now CC BY 4.0; code stays MIT OR Apache-2.0.
  Added NOTICE and CITATION.cff ([licence and credits](README.md#licence-and-credits)).
- Docs: the README, [performance](docs/performance.md), [PCM quality](docs/pcm-quality.md),
  the [user guide](docs/user-guide.md) and [options](docs/options.md) lead with the
  `precise` filter's measured quality (SINAD 142 to 145 dB with a very quiet DSD64 source,
  151 dB or more of rejection of everything that folds into the audio band) and its cost
  (1.3 to 1.9 times scarletbook-pcm 0.7.0's CPU time on an Intel Core i9-13900H), and name
  `fast` as the scarletbook-pcm-identical option with lower CPU time and lower quality
  (1.11 to 1.25 times scarletbook-pcm's speed; SINAD 122 dB with the same source).
- The repository moved to
  [github.com/dsd-toolbox/dsd2pcm-rs](https://github.com/dsd-toolbox/dsd2pcm-rs): links,
  badges, the crates' `repository` field and the attestation commands
  (`gh attestation verify -R dsd-toolbox/dsd2pcm-rs --signer-workflow dsd-toolbox/dsd2pcm-rs/.github/workflows/release.yml`)
  use the new address. The old addresses redirect.
- Speed in a consumer's release profile (codegen units 16, no LTO, overflow checks on, as
  a dependency): it was up to 25 % slower than with ours, now within about 3 % on the
  measured CPUs (up to 1.031 times; the CI gate allows 5 %). CI checks it on every pull
  request. Output bytes unchanged.
- `frames::Converter`: the first-stage values kept between calls cost next to nothing
  when consecutive calls are not adjacent runs (pooled converters): only the bytes the next
  run could share are kept. Output bytes unchanged.
- `frames::Converter`: a new converter's first call no longer runs a throwaway pass to
  size its buffers, and the 13-bit first-stage tables, which the AVX2 levels with paired
  tables never use, are built only when needed. A new converter plus its first 4-frame run
  takes about as long as scarletbook-pcm 0.7.0's.
- Command line: `--` ends the options, so a file whose name starts with `-` can be given
  (`dsd2pcm -- -take1.dsf out.wav`). An unknown argument that starts with `-` is now a usage
  error; before, one that did not start with `--` was taken as a file name.
- Docs in the DSD Toolbox layout: a [documentation index](docs/README.md), the
  [user guide](docs/user-guide.md) and [options](docs/options.md) pages (from
  `docs/usage.md`), and [CONTRIBUTING.md](CONTRIBUTING.md).
- C interface: a struct larger than the library's own (from a newer header) is accepted
  only when its bytes past the library's size are 0; a set field the library does not know
  is refused with `DSD2PCM_E_INVALID_ARGUMENT` naming the byte, instead of being ignored.
  An output struct's extra bytes are left as they are, as before. A `struct_size` above
  4096 is refused (it catches a size that was never set). The library never reads past the
  caller's `struct_size`, and fields the caller's struct lacks read as 0, as before
  ([C interface](docs/c-api.md#abi)).
- WAV input: `WAVE_FORMAT_EXTENSIBLE` files are checked: a sub-format GUID other than
  `KSDATAFORMAT`'s PCM and IEEE float (for example Ambisonic B-format, whose first bytes
  read like PCM's) is `Unsupported`, and more valid bits than the container is
  `Malformed`. Valid bits of 0 or below the container (20 in 24, as `flac -d` writes)
  read as before.
- FLAC input: a stream with more frames than STREAMINFO declares is read to its end, as
  the reference decoder and ffmpeg do; before, the frames past the declared total were
  dropped without a word. The new `FlacReader::declared_frames` keeps STREAMINFO's count,
  and `info().frames` is the count delivered once the stream has ended. A stream with
  fewer frames stays an error (`Truncated`), as with the reference decoder. Bytes after the
  last frame that are not a frame (an ID3v1 tag, padding) are still not audio.
- Readers: the rules for files that bend their format (WAV cut short, DSDIFF `DSD ` chunk
  not a multiple of the channel count, DSF tag over 16 MiB) are documented with a test
  each ([design](docs/design.md#robustness)); their behaviour is unchanged.

### Added

- No-clobber output: the tool's `-n` / `--no-clobber`, the library's `convert_file_to` with
  `OutputOptions { no_clobber, .. }`, and the C interface's
  `dsd2pcm_file_options.no_clobber` (appended; callers built against the 0.1.0 header keep
  working) with the new status `DSD2PCM_E_EXISTS`. An existing output is refused and left
  as it was, and the result is put in place without ever replacing a file that appeared
  during the conversion: a hard link, or on file systems without hard links an exclusive
  create then a rename ([design](docs/design.md#output-files)).
- Channel layouts: `WavReader::channel_mask` and `FlacReader::channel_mask` (and
  `PcmRead::channel_mask`) give the channel mask a file declares (`WAVE_FORMAT_EXTENSIBLE`,
  or FLAC's `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment); `DsdRead::channel_mask` the
  layout of a DSF channel type or of DSDIFF channel IDs; `WavWriter::with_channel_mask`,
  `DsfWriter::set_channel_mask`, `dsf::channel_mask_for_type` and
  `dsf::channel_type_for_mask` ([user guide](docs/user-guide.md#formats)).
- C library: CI, `crates/dsd2pcm-capi/check.sh` and the release archive checks verify that
  the libraries hold no claxon (FLAC decoder) symbols, by their symbol tables (`nm`), and
  CI checks that the check catches a library built with `--features flac`
  ([C interface](docs/c-api.md#checks)). The check fails closed (an nm error, or an archive
  member nm did not list, fails it), has its own tests on small archives with a planted
  claxon symbol, and reads Mach-O symbol tables on macOS: Xcode's `nm` otherwise reads the
  LLVM bitcode embedded in Rust's `compiler_builtins` objects, fails on bitcode from a newer
  LLVM, and skipped those members.
- CI: a check that tracked files and commit identities hold no private names or addresses
  ([testing](docs/testing.md#private-names)).
- Docs: the shared DSD Toolbox docs checks in `docs.yml` (page and README shape, links in
  every Markdown file, the facts regions, a docs change with every code change on pull
  requests, and weekly the documented commands: the Quick start and user guide against the
  published release, the build, test and fuzzing commands in a copy of the commit, and a
  refresh of the facts regions onto the branch `docs/facts-update`), and a `docs-audit`
  workflow to run after each release
  ([releasing](docs/release.md#after-the-release-run-the-docs-audit)). lychee now checks the
  external links only; the shared check covers relative links and anchors.
- Tests: golden SHA-256 hashes of whole output files (header and samples) from
  `convert_file`, for mono, stereo and six-channel input, DSF channel types 5 to 7 and
  DSDIFF files with speaker and numbered channel IDs, at every rate and cutoff and with
  pysacd's filter, pinned to 0.1.2's files (except DSF type 5 and the DSDIFF speakers,
  whose channel mask changed, and pysacd's, now computed in `f64`; see Changed, Output);
  the MSRV workflow also builds the tests with Rust 1.85, which they need ([development](docs/development.md)).
- Releases: the C library for Linux with musl (x86_64, aarch64), macOS on Intel (signed and
  notarised) and Windows with MinGW-w64 (`x86_64-windows-gnu`, for gcc); the command-line
  tool for musl (statically linked) and macOS on Intel; an attested source archive
  (`dsd2pcm-<version>-src.tar.gz`, from `git archive`). On x86_64 Linux the static
  library's objects are marked for the CET shadow stack.
  [c-api.md](docs/c-api.md#binary-releases) explains how to embed the static library in a
  shared library with its symbols hidden.
- Releases: the C library's own source archive, `libdsd2pcm-<version>-src.tar.gz`
  (attested, with a `.sha256` file): `dsd2pcm` and `dsd2pcm-capi` as a workspace of their
  own whose `Cargo.lock` lists only them, so `cargo build --offline --locked --release -p
  dsd2pcm-capi` builds it with an empty `CARGO_HOME` and no network, for a build inside
  another project's source distribution. Its manifests leave out the `flac` feature, claxon
  and the dev-dependencies, which Cargo otherwise looks up even when unused; the C library
  never has `flac`, and its output is the same (the C example converts the same test files
  to the same bytes as with the repository's build, at every SIMD level). It is made
  reproducibly by `.github/scripts/make-capi-src.sh`, and CI checks on every pull request
  that a second run gives the same bytes and that it builds offline and runs its example
  ([C interface](docs/c-api.md#offline-source-build)).
- New filter `precise` (`--filter precise`, `Filter::Precise`,
  `DSD2PCM_FILTER_PRECISE`), the default since this release (see Changed, Output): every rate and
  cutoff, exact integer arithmetic like scarletbook-pcm's filters, at least 150 dB of stopband
  wherever ultrasonic content folds into the passband
  (`fast`: about 110 dB) and about half the passband ripple. With a very quiet DSD64
  source the audio-band floor falls from -128 dBFS (SINAD 122 dB) to the 24-bit output's,
  about -150 dBFS (SINAD 142 to 145 dB); it takes about twice the CPU time
  ([PCM quality](docs/pcm-quality.md#the-precise-filter)). Its taps come from
  `tools/filters/design_precise.py`.
- Tests: golden SHA-256 hashes of the precise filter, named and as the default, at every
  SIMD level, for the sample tests (`tests/identity.rs`) and the whole output files
  (`tests/golden_files.rs`); the fast filter's golden hashes are unchanged.
  The C tests check every `DSD2PCM_FILTER_*` value and what each selects.
- Docs: [PCM quality](docs/pcm-quality.md), the measured audio quality of every filter and
  rate (frequency response, aliasing, ringing, noise, ultrasonic noise, headroom, an
  RMAA-style summary; the pysacd preset, scarletbook-pcm 0.7.0 and the foobar2000 SACD
  plug-in compared), with the measurement scripts in `tools/quality`.
- `Threads::Count(n)` runs a conversion on at most n threads (up to 256), and the C
  interface's `dsd2pcm_file_options.threads` takes n > 1 the same way (it was refused), as
  does the tool's new `--threads N` (1 to 256; 1 is `--single-thread`). The output bytes are
  the same for every count (tested from 1 to 7, all cores and above the cap, with every
  filter; the tool's test at 1, 2, 3, 5, 7, 64 and 256 against all cores).
- `WavReader::declared_frames`: the frames a WAV file's `data` size declares, which is more
  than `info().frames` when the file is cut short. `FlacReader::declared_frames`: the
  frames STREAMINFO declares, which is less than `info().frames` at the end of a stream that
  holds more. `DsfReader::skipped_metadata_len`: the
  size of a DSF tag over `MAX_METADATA` (16 MiB) that was not loaded; before, such a tag
  was dropped without a trace ([design](docs/design.md#robustness)).
- C interface: `tests/struct_size.c` checks `struct_size` versioning from C, also under
  AddressSanitizer and UndefinedBehaviorSanitizer: a `dsd2pcm_file_options` of the 0.1.0
  size (no `no_clobber`) passed as a heap block of exactly that size and as one followed
  by a set `no_clobber` that must not be read, the current size, larger ones with zero and
  non-zero extra bytes, and the same for the other structs.
- Windows: `dsd2pcm.exe` double-clicked in Explorer (no arguments, and alone in a console
  window of its own) prints how to run it from PowerShell or Terminal and waits for Enter
  instead of closing at once; it then exits with 2, as for missing arguments. Started from a
  shell, with arguments, without a console or in a console without a window, it behaves as
  before ([user guide](docs/user-guide.md#windows)).

### Fixed

- Tests: the soak test failed every run of 1000 iterations at 0.2.0's code without a leak
  (the live heap rose 1.1 MiB from its 20 to 50 % window to the second half, all of it the
  kept converters' buffers growing). It now judges memory after a warm-up of two rounds of
  its cycle and only in runs of 1000 iterations or more (shorter runs report); the live
  heap leaves out the kept converters' growth, which has its own check, and the resident
  set is judged by its medians with a bound that grows with the worker count. A self-test,
  `soak_catches_a_leak`, checks that leaks put in on purpose fail the checks
  ([testing](docs/testing.md#soak-test)).
- CI: Miri runs on every pull request in two jobs, `miri (capi)` and `miri (core)`, of
  under 20 minutes each; the core job hit its 150-minute limit. Three tests that take over
  50 minutes each under Miri run natively only ([testing](docs/testing.md#miri)).
- Command line: a file name or option value that is not valid UTF-8 (possible on Linux and
  other Unix systems) no longer ends the tool with a panic. File names are passed to the
  system as they are; an option value that is not UTF-8 is a usage error (exit status 2).
- Whole-file conversion on all cores: when the system refuses to start a worker thread
  (its thread or memory limit), the conversion goes on with the threads that did start, at
  least the calling one, with the same output; before, it panicked (in the C interface,
  `DSD2PCM_E_INTERNAL`). The calling thread now converts too, so a conversion starts one
  thread fewer for the same parallelism ([design](docs/design.md#frame-runs-threading-and-sharing)).
- `frames::Converter` with the precise filter: at 176.4 kHz on the avx2fma level, a long
  run after a short run with neighbours grew a buffer, so a call after the first could
  allocate (one reallocation per converter). The buffer the short runs keep is now sized
  for the long ones too. Output bytes unchanged; `tests/frames_alloc.rs` now checks every
  filter at every SIMD level.

## 0.1.2 - 2026-11-23

This release contains security fixes; upgrading is recommended. Versions 0.1.0 and 0.1.1 are
yanked from crates.io.

### Added

- Testing for long-lived processes: a soak test (thousands of conversions in one process,
  live heap, RSS and threads checked flat), a C create-convert-free leak loop under ASan
  and LSan, and Miri, ThreadSanitizer and valgrind runs ([testing](docs/testing.md)).
- Docs: checked against the code and the 0.1.1 release (commands, file names, versions);
  a security policy (`SECURITY.md`); the design decisions with their reasons
  ([design](docs/design.md)); speed on GitHub-hosted aarch64 Linux and macOS runners; CI
  checks for links, the README Quick start and version drift.

### Changed

- x86_64 without AVX2 (e.g. Celeron and Pentium J and N series of NAS units): the `sse2`
  level now runs there too, at 1.09x to 1.34x the speed of scarletbook-pcm 0.7.0's path for
  CPUs without AVX2 on runs of frames and whole files (1.03x to 1.27x one frame per call),
  where the generic level ran at 0.84x to 1.17x ([performance](docs/performance.md)).
  Output bytes unchanged.
- C library: built without FLAC reading (unreachable from C); the archives now contain no
  third-party code.
- Command line: `--help` prints the usage to standard output and exits with status 0 (it
  exited with 2); the usage explains `--single-thread`.
- Output files (`convert_file`, `convert_file_with`, `dsd2pcm_convert_file` and the tool):
  the output is written to `.dsd2pcm-PID-N.tmp` in the output's directory and renamed into
  place. A SIGKILL or power loss may leave that file behind; it is safe to delete.
- A symbolic link at the output path is replaced, not followed.
- A replaced file keeps its permission bits, but not its owner, ACLs or extended
  attributes.
- These outputs are now refused: a directory or a path ending in a slash; a device, FIFO or
  socket (including `/dev/null`); an existing file that is not writable; an output whose
  directory is not writable; a hard link to the input (Unix).
- The command-line tool exits with 130 on SIGINT and 143 on SIGTERM and leaves no file.
- The FLAC reader allows 4096 metadata blocks including STREAMINFO (4097 before).
- New dependency of the command-line tool: `signal-hook`.

### Fixed

- DSF input whose sample count is not a multiple of 8: the padding bits of the last byte
  convert as silence (the reader delivers them as 0x69's bits); before, zero padding gave a
  click in each channel's last few output samples ([user guide](docs/user-guide.md#output)).
- Readers: opening a DFF, WAV or FLAC file with many small chunks takes time linear in its
  size with any buffered reader; DFF and WAV files may have at most 4096 chunks before the
  audio data (and DFF 4096 property chunks), as FLAC already had for metadata blocks.
- Output files: `convert_file`, `convert_file_with`, `dsd2pcm_convert_file` and the tool
  check the input and options before writing, write to a temporary file next to the output
  and rename it into place only on success, so a failed or cancelled conversion leaves an
  existing output as it was and never leaves a partial WAV; an output that is a hard link to
  the input is refused on Unix, as the same path and symbolic links already were. The tool
  also stops cleanly on Ctrl-C or SIGTERM (Ctrl-Break on Windows): it removes the
  temporary file and exits with 130 or 143.
- `WavWriter::write_raw` refuses data past the 4 GiB RIFF limit when it is written, not
  only at `finish`.
- C interface: a struct from a caller built against an older header is read for its
  `struct_size` bytes only, with the missing fields taken as 0, and the 0.1.0 sizes are
  fixed per target layout (64-bit, i686, other 32-bit), so fields can be appended later
  without refusing old callers. No change in behaviour today.
- Docs: C interface rules (C99, alignment of zero-length buffers, 32-bit struct sizes, error
  messages); identical output on x86 assumes SSE2 floating point.
- 32-bit targets: an output count past 2^32 samples per channel no longer panics in
  `dsd64_to_pcm_with`.
- Testing: the sanitizer soak run no longer fails on its short warm-up.
- Release checks: a pipeline check could pass or fail wrongly when its reader stopped early.

## 0.1.1 - 2026-11-23

Faster, same output: every output byte is unchanged from 0.1.0 at every SIMD level.

- Speed (x86_64 with AVX2 and FMA): passes run in four lanes of output runs, the first stage
  reads two outputs per lookup from byte tables that fit in the L1 cache, and a
  `frames::Converter` reuses first-stage values between calls when the input bytes are the
  same. One-frame-per-call conversion is now ahead of scarletbook-pcm 0.7.0 on the measured
  Intel P- and E-cores and on Zen 2 ([performance](docs/performance.md)).
- 32-bit x86: a new SIMD level, `sse2` (`DSD2PCM_SIMD=sse2`), picked on CPUs without AVX2
  such as Atom: 1.1x to 1.3x the speed of scarletbook-pcm 0.7.0 where the generic level ran
  at 0.9x to 1.0x.
- Releases: the macOS tool and C library are signed (Developer ID, hardened runtime) and
  notarised. Windows builds are still not code-signed.
- Docs: README in the DSD Toolbox layout; usage and development notes in `docs/`.

## 0.1.0 - 2026-11-23

First release.

- DSD64 to 24-bit PCM at 88.2, 176.4 and 352.8 kHz with three cutoffs, sample-identical to
  scarletbook-pcm 0.7.0 on every CPU and SIMD level; multithreaded whole-file conversion.
- `dsd2pcm::frames`: conversion of runs of SACD frames with neighbouring frames, with
  scarletbook-pcm 0.7.0's interface.
- Opt-in `Filter::Pysacd` (`--filter pysacd`): pysacd's three-stage filter, bit-exact to its
  reference build on every CPU, with dither seeded per SACD frame.
- Reading DSF, DSDIFF, WAV (including `data` size 0 or 0xFFFFFFFF as "to the end") and FLAC;
  writing DSF, DSDIFF and WAV (`WAVE_FORMAT_EXTENSIBLE` above 16 bits or 2 channels).
- Progress and cancel hooks; `Error::Cancelled`.
- A C interface (`libdsd2pcm`, ABI 1), shipped as release archives with a pkg-config file.
- The `dsd2pcm` command-line tool.

