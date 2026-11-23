# Usage

## Formats

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

## Output

The output samples are identical to those of scarletbook-pcm 0.7.0 for the same rate,
cutoff and gain; the tests compare the two on synthetic and real input. Every sample is an
exact integer computation, so the output is the same on every CPU and build profile.
The DSD idle pattern (0x69) converts to exact zeros. On x86 the filters use AVX2 and FMA
when the CPU has them, chosen at run time, with the same output; the environment variable
`DSD2PCM_SIMD` (`generic`, `sse2` on x86 and x86_64, `avx2` or `avx2fma`) caps the level.

One intended difference: a DSF file whose sample count is not a multiple of 8 ends in a
byte with padding bits past the last sample, usually zeros. The reader delivers those bits
as the idle pattern 0x69's bits at the same positions, so the padding converts as silence
rather than as a step of up to -7 dBFS. The output is identical to scarletbook-pcm 0.7.0
given the same masked bytes. It differs from a conversion of the raw bytes only for such
files whose padding is not already 0x69's bits, and only in each channel's last K output
samples, with K at most `ceil(context_bytes / k) + 1` (`k` input bytes per output sample):

| Filter | Rate | Cutoff | K at most |
|---|---|---|---|
| default | 88.2 kHz | 30 / 40 | 46 / 36 |
| default | 176.4 kHz | 30 / 40 / wide | 91 / 71 / 62 |
| default | 352.8 kHz | 30 / 40 / wide | 196 / 156 / 67 |
| pysacd | 88.2 kHz | 40 | 55 |
| pysacd | 176.4 kHz | 40 | 27 |

Options: `--gain DB` (-120 to +24, default 0), `--single-thread`, `--filter pysacd`.

`--filter pysacd` (`Filter::Pysacd` in `PcmOptions` and `frames::PcmConfig`) uses pysacd's
filter instead: three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered quantiser,
bit-exact with pysacd's reference build on every CPU and SIMD level; 88.2 and 176.4 kHz
only, with a fixed response (leave `--cutoff` at the default). The default filter and its
output are unchanged.

## Command line

```
dsd2pcm IN OUT [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|pysacd] [--single-thread]
```

The `dsd2pcm` tool converts one DSF or DFF file (DSD64) to a 24-bit WAV file; WAV and FLAC
input is read by the library only. Defaults: `--rate 176400 --cutoff 40 --gain 0
--filter default`, on all cores (`--single-thread`: on the calling thread only). It prints
the rates, the length of the audio, the time taken and the clipped samples, in all and per
channel. Exit status: 0 on success, 1 when the conversion fails, 2 for a usage error.
`--help` prints the usage.

## Library

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
