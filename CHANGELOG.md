# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses
[semantic versioning](https://semver.org/). Output is deterministic within a release (the
same version, input and settings give the same bytes on every CPU and SIMD level); between
releases it may change when measured quality improves, and every such change is listed here
with its measured effect. Releases up to 0.1.2 keep their behaviour as released.

## Unreleased

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
  ([C interface](docs/c-api.md#checks)).
- CI: a check that tracked files and commit identities hold no private names or addresses
  ([testing](docs/testing.md#private-names)).
- Tests: golden SHA-256 hashes of whole output files (header and samples) from
  `convert_file`, for mono, stereo and six-channel input, DSF channel types 5 to 7 and
  DSDIFF files with speaker and numbered channel IDs, at every rate and cutoff and with
  pysacd's filter, pinned to 0.1.2's files (except DSF type 5 and the DSDIFF speakers,
  whose channel mask changed, below); the MSRV workflow also builds the tests with Rust
  1.85, which they need ([development](docs/development.md)).
- Releases: the C library for Linux with musl (x86_64, aarch64), macOS on Intel (signed and
  notarised) and Windows with MinGW-w64 (`x86_64-windows-gnu`, for gcc); the command-line
  tool for musl (statically linked) and macOS on Intel; an attested source archive
  (`dsd2pcm-<version>-src.tar.gz`, from `git archive`). On x86_64 Linux the static
  library's objects are marked for the CET shadow stack.
  [c-api.md](docs/c-api.md#binary-releases) explains how to embed the static library in a
  shared library with its symbols hidden.
- New opt-in filter `precise` (`--filter precise`, `Filter::Precise`,
  `DSD2PCM_FILTER_PRECISE`): every rate and cutoff, exact integer arithmetic like the
  default, at least 150 dB of stopband wherever ultrasonic content folds into the passband
  (the default: about 110 dB) and about half the passband ripple. With a very quiet DSD64
  source the audio-band floor falls from -128 dBFS (SINAD 122 dB) to the 24-bit output's,
  about -150 dBFS (SINAD 142 to 145 dB); it takes about twice the CPU time
  ([PCM quality](docs/pcm-quality.md#the-precise-filter)). Its taps come from
  `tools/filters/design_precise.py`. The default filter and its output are unchanged.
- Docs: [PCM quality](docs/pcm-quality.md), the measured audio quality of every filter and
  rate (frequency response, aliasing, ringing, noise, ultrasonic noise, headroom, an
  RMAA-style summary; the pysacd preset, scarletbook-pcm 0.7.0 and the foobar2000 SACD
  plug-in compared), with the measurement scripts in `tools/quality`.

### Changed

- Output stability rule: output is deterministic within a release (the same version, input
  and settings give the same bytes on every CPU and SIMD level). Between releases it may
  change when measured quality improves; every such change is listed in this changelog with
  its measured effect (`docs/quality/<version>.md`). Releases up to 0.1.2 keep their
  behaviour as released. This replaces the rule that output bytes change only with a major
  version ([design](docs/design.md#the-contract-deterministic-output-the-deepest-stopband-filter-by-default),
  [AGENTS.md](AGENTS.md)).
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
- Output: DSD to WAV conversion writes the channel mask of the input's layout (DSF channel
  type, DSDIFF channel IDs) instead of the one of its channel count. The WAV header changes
  only where the two differ: a DSF of type 5 (FL FR C LFE) was labelled quad (FL FR BL BR),
  and DSDIFF files with other speaker sets, such as FL FR LFE, were labelled by their
  count. The samples are unchanged.
- Speed in a consumer's release profile (codegen units 16, no LTO, overflow checks on, as
  a dependency): it was up to 25 % slower than with ours, now within 1 % on the measured
  CPUs. CI checks it on every pull request. Output bytes unchanged.
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

### Fixed

- Command line: a file name or option value that is not valid UTF-8 (possible on Linux and
  other Unix systems) no longer ends the tool with a panic. File names are passed to the
  system as they are; an option value that is not UTF-8 is a usage error (exit status 2).
- Whole-file conversion on all cores: when the system refuses to start a worker thread
  (its thread or memory limit), the conversion goes on with the threads that did start, at
  least the calling one, with the same output; before, it panicked (in the C interface,
  `DSD2PCM_E_INTERNAL`). The calling thread now converts too, so a conversion starts one
  thread fewer for the same parallelism ([design](docs/design.md#frame-runs-threading-and-sharing)).

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

