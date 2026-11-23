# Changelog

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
