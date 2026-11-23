[dsd2pcm-rs](../README.md) › [Documentation](README.md) › User guide

# User guide

How to convert DSD to PCM with the `dsd2pcm` tool or the `dsd2pcm` crate: the formats it
reads and writes, the choices of rate, filter and gain, and what to do when a conversion
fails. Every option is listed in [options.md](options.md).

## Convert a file

`dsd2pcm` converts one DSF or DFF file (DSD64) to a 24-bit WAV file, by default at
176.4 kHz with the 40 kHz filter, on all cores. With the tool on the `PATH` (`cargo install
dsd2pcm-cli`, or the release archive of the [Quick start](../README.md#quick-start)):

```sh
dsd2pcm "01 - Track.dsf" "01 - Track.wav"
```

It prints the input and output rates, the length of the audio, the time taken and the
number of clipped samples, in all and per channel:

```text
2822400 Hz -> 176400 Hz, 2.00 s of audio in 0.01 s (284.6x real time), clipped samples: 0 [0, 0]
```

## Choose the rate and the filter

The output is 24-bit PCM at 88.2, 176.4 or 352.8 kHz (`--rate`), through one of three
linear-phase filters (`--cutoff`):

| `--cutoff` | Passband | Stopband from | Rates |
|---|---|---|---|
| `40` (default) | 40 kHz | 48 kHz | all |
| `30` | 30 kHz | 36 kHz | all |
| `wide` | 80 kHz | 88.2 kHz (176.4 kHz), 96 kHz (352.8 kHz) | 176.4 and 352.8 kHz |

For example, 88.2 kHz with the 30 kHz filter, 3 dB quieter:

```sh
dsd2pcm "01 - Track.dsf" "01 - Track.wav" --rate 88200 --cutoff 30 --gain -3
```

The output samples are identical to those of scarletbook-pcm 0.7.0 for the same rate,
cutoff and gain; the tests compare the two on synthetic and real input. Every sample is an
exact integer computation, so the output is byte-identical on every CPU and build profile.
The DSD idle pattern (0x69) converts to exact zeros. On x86 the filters use AVX2 and FMA
when the CPU has them, chosen at run time, with the same output; the environment variable
`DSD2PCM_SIMD` (`generic`, `sse2` on x86 and x86_64, `avx2` or `avx2fma`) caps the level.

`--filter pysacd` (`Filter::Pysacd` in `PcmOptions` and `frames::PcmConfig`) uses pysacd's
filter instead: three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered quantiser,
bit-exact with pysacd's reference build on every CPU and SIMD level; 88.2 and 176.4 kHz
only, with a fixed response (leave `--cutoff` at the default). The default filter and its
output are unchanged.

## Formats

- Reads DSF and DFF (DSDIFF, uncompressed; DST-compressed files are rejected with a clear
  error) at DSD64 to DSD1024, WAV (16/24/32-bit integer, 32/64-bit float) and FLAC
  (8/12/16/20/24-bit, 1 to 6 channels, optionally after an ID3v2 tag). A WAV
  `data` size left at 0 or 0xFFFFFFFF (streaming writers, some encoders past 4 GiB) reads
  to the end of the file; if other chunks follow it, the file is rejected. FLAC and WAV of
  the same samples read identically. FLAC reading is the cargo feature `flac` (on by
  default; `--no-default-features` builds without it and its one dependency, `claxon`).
- Writes DSF, DFF and WAV. WAV files with more than 16 bits, float samples or more than two
  channels use `WAVE_FORMAT_EXTENSIBLE` with the channel mask of the channel count (SACD
  order: FL FR FC LFE BL BR).
- Converts DSD64 to PCM. The command-line tool converts DSF and DFF only; WAV and FLAC
  input is read by the library.

## Use the library

`dsd2pcm::convert_file` converts a DSF or DFF file to a WAV file with `PcmOptions` (rate,
cutoff, filter, gain, threads), the same conversion as the tool.

`dsd2pcm::frames::Converter` converts SACD frames (channels interleaved byte by byte, 4704
bytes per channel per frame) a run at a time, with the frames before and after the run as
context, into interleaved 24-bit PCM, with clip counts per frame. Its interface and output
bytes are those of scarletbook-pcm 0.7.0's `Converter::convert`; any run with its
neighbours gives the bytes of that run in a whole-area conversion. One converter per
thread; after the first call it allocates nothing.

`convert_file_with` and `dsd64_to_pcm_with` take a progress hook: it gets the output
samples done and the total at each block, on the calling thread, and stops the conversion
with `Error::Cancelled` when it returns `ControlFlow::Break`. `Error` is `#[non_exhaustive]`.

The API reference is on [docs.rs](https://docs.rs/dsd2pcm); the C library is described in
[c-api.md](c-api.md).

## Limits

DSD64 input only for PCM output; WAV output up to 4 GiB. [plan.md](plan.md) lists what
comes next.

## Troubleshooting

| Message | Cause |
|---|---|
| `unsupported: input is PCM, not DSD` | The input is a WAV or FLAC file: the tool converts DSF and DFF only. |
| `invalid argument: the wide filter is not available at 88.2 kHz` | `--cutoff wide` needs `--rate 176400` or `--rate 352800`. |
| `invalid argument: the pysacd filter has 88.2 and 176.4 kHz only` | `--filter pysacd` needs `--rate 88200` or `--rate 176400`. |
| `invalid argument: gain 30 dB is outside -120 to +24 dB` | `--gain` takes -120 to +24. |
| `unsupported: DST-compressed DSDIFF; decompress it first` | The DFF file is DST-compressed; decompress it to plain DFF or DSF first. |
| `invalid argument: output is the input file` | The output path names the input file. |
| `unsupported: WAV output over 4 GiB (RF64 is not supported yet)` | Split the input, or use a lower rate. |

A non-zero clipped count means the filter's output exceeded full scale on those samples;
a negative `--gain` (for example `--gain -3`) avoids it.

## See also

- [Options](options.md): every command-line option and its default
- [C interface](c-api.md): the C library
- [Design](design.md): how the conversion works and why its output is exact
- [Performance](performance.md): speed against scarletbook-pcm 0.7.0

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
