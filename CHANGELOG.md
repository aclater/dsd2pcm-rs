# Changelog

## Unreleased

- x86_64 without AVX2 (e.g. Celeron and Pentium J and N series of NAS units): the `sse2`
  level now runs there too, where the generic level ran at 0.6x to 0.9x the speed of
  scarletbook-pcm 0.7.0 ([performance](docs/performance.md)). Output bytes unchanged.
- Command line: `--help` prints the usage to standard output and exits with status 0 (it
  exited with 2); the usage explains `--single-thread`.
- Docs: checked against the code and the 0.1.1 release (commands, file names, versions);
  a security policy (`SECURITY.md`).

### Fixed

- DSF input whose sample count is not a multiple of 8: the padding bits of the last byte
  convert as silence (the reader delivers them as 0x69's bits); before, zero padding gave a
  click in each channel's last few output samples ([usage](docs/usage.md#output)).
- Readers: opening a DFF, WAV or FLAC file with many small chunks takes time linear in its
  size with any buffered reader; DFF and WAV files may have at most 4096 chunks before the
  audio data (and DFF 4096 property chunks), as FLAC already had for metadata blocks.
- Output files: `convert_file`, `convert_file_with`, `dsd2pcm_convert_file` and the tool
  check the input and options before writing, write to a temporary file next to the output
  and rename it into place only on success, so a failed or cancelled conversion leaves an
  existing output as it was and never leaves a partial WAV; an output that is a hard link to
  the input is refused on Unix, as the same path and symbolic links already were.
- `WavWriter::write_raw` refuses data past the 4 GiB RIFF limit when it is written, not
  only at `finish`.
- 32-bit targets: an output count past 2^32 samples per channel no longer panics in
  `dsd64_to_pcm_with`.

## 0.1.1

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
  notarized. Windows builds are still not code-signed.
- Docs: README in the DSD Toolbox layout; usage and development notes in `docs/`.

## 0.1.0

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
