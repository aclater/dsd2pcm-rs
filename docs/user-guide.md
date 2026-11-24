[dsd2pcm-rs](../README.md) › [Documentation](README.md) › User guide

# User guide

How to convert DSD to PCM with the `dsd2pcm` tool or the `dsd2pcm` crate: the formats it
reads and writes, the choices of rate, filter and gain, how it writes its output files, and
what to do when a conversion fails. Every option is listed in [options.md](options.md).

## Convert a file

`dsd2pcm` converts one DSF or DFF file (DSD64) to a 24-bit WAV file, by default at
176.4 kHz with the 40 kHz filter, on all cores. With the tool on the `PATH` (`cargo install
dsd2pcm-cli`, or the release archive of the [Quick start](../README.md#quick-start)):

<!-- docs-run: skip="needs dsd2pcm on the PATH and a DSF file of the reader; the README Quick start runs the same conversion" -->
```sh
dsd2pcm "01 - Track.dsf" "01 - Track.wav"
```

It prints the input and output rates, the length of the audio, the time taken and the
number of clipped samples, in all and per channel:

```text
2822400 Hz -> 176400 Hz, 2.00 s of audio in 0.02 s (91.4x real time), clipped samples: 0 [0, 0]
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

<!-- docs-run: skip="needs dsd2pcm on the PATH and a DSF file of the reader; the README Quick start runs the same conversion" -->
```sh
dsd2pcm "01 - Track.dsf" "01 - Track.wav" --rate 88200 --cutoff 30 --gain -3
```

The filter itself is chosen with `--filter`:

| `--filter` | What it is | Quality | CPU time |
|---|---|---|---|
| `precise` (default) | Exact integer filter with at least 150 dB of stopband wherever ultrasonic content would fold into the passband | SINAD 142 to 145 dB with a very quiet DSD64 source, the limit of 24-bit output | 1.3 to 1.9 times scarletbook-pcm 0.7.0's (Intel Core i9-13900H) |
| `fast` | scarletbook-pcm 0.7.0's filter, byte-identical with it; the default up to 0.1.2 | About 110 dB of stopband: SINAD 122 dB with the same source (116 dB with `wide`) | Lower: 1.11 to 1.25 times scarletbook-pcm's speed (Intel Core i9-13900H) |
| `pysacd` | pysacd's filter and dithered quantiser; 88.2 and 176.4 kHz only | See [PCM quality](pcm-quality.md#comparisons) | – |

`precise` (`Filter::Precise`) and `fast` (`Filter::Fast`) share the cutoffs, rates, exact
integer arithmetic, centring and rounding; `precise` has about half the passband ripple.
On the real discs measured, the recording's own noise set the floor with either filter,
and `precise` lowered it by 1.1 dB in the quietest second; the difference shows most with sources quieter
than `fast`'s -128 dBFS floor, and in measurements
([PCM quality](pcm-quality.md#the-precise-filter), [performance](performance.md)). Use
`fast` for lower CPU time, or for output byte-identical with scarletbook-pcm 0.7.0 or with
dsd2pcm 0.1.x's default.

`--filter pysacd` (`Filter::Pysacd` in `PcmOptions` and `frames::PcmConfig`) uses pysacd's
filter instead: three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered quantiser,
bit-exact with pysacd's reference build on every CPU and SIMD level; 88.2 and 176.4 kHz
only, with a fixed response (leave `--cutoff` at the default). [design.md](design.md) gives
the reasons for each filter.

## Output

Within a release, the output is deterministic: every sample of `precise` and `fast` is an
exact integer computation, so the same input and settings give the same bytes on every CPU,
SIMD level and build profile. With `fast` the samples are identical to those of
scarletbook-pcm 0.7.0 for the same rate, cutoff and gain; the tests compare the two on
synthetic and real input. Between releases the output may change when measured quality
improves; the [changelog](../CHANGELOG.md) lists every such change with its measured
effect.
On x86 this assumes SSE2 floating point, as on x86_64 and Rust's i686 targets (the release
builds); targets that use the x87 unit (i586 and older) are not covered.
The DSD idle pattern (0x69) converts to exact zeros. On x86 the filters use AVX2 and FMA
when the CPU has them, chosen at run time, with the same output; the environment variable
`DSD2PCM_SIMD` (`generic`, `sse2` on x86 and x86_64, `avx2` or `avx2fma`) caps the level.

One intended difference: a DSF file whose sample count is not a multiple of 8 ends in a
byte with padding bits past the last sample, usually zeros. The reader delivers those bits
as the idle pattern 0x69's bits at the same positions, so the padding converts as silence
rather than as a step of up to -7 dBFS. With `fast` the output is identical to scarletbook-pcm 0.7.0
given the same masked bytes. It differs from a conversion of the raw bytes only for such
files whose padding is not already 0x69's bits, and only in each channel's last K output
samples, with K at most `ceil(context_bytes / k) + 1` (`k` input bytes per output sample):

| Filter | Rate | Cutoff | K at most |
|---|---|---|---|
| fast | 88.2 kHz | 30 / 40 | 46 / 36 |
| fast | 176.4 kHz | 30 / 40 / wide | 91 / 71 / 62 |
| fast | 352.8 kHz | 30 / 40 / wide | 196 / 156 / 67 |
| precise | 88.2 kHz | 30 / 40 | 83 / 64 |
| precise | 176.4 kHz | 30 / 40 / wide | 164 / 126 / 113 |
| precise | 352.8 kHz | 30 / 40 / wide | 350 / 274 / 119 |
| pysacd | 88.2 kHz | 40 | 55 |
| pysacd | 176.4 kHz | 40 | 27 |

## Output files and interruptions

`convert_file` and `convert_file_with` (and the `dsd2pcm` tool) check the input, its rate,
the options, the output size and the output path before writing. They refuse an output that
is the input (by path, symbolic link or, on Unix, hard link), a directory or a path ending
in a separator, an existing device, FIFO or socket, an existing file that is not writable,
and a directory that cannot be written. The WAV goes to a temporary file
`.dsd2pcm-PID-N.tmp` in the output's directory, which is renamed over the output only on
success and given the permissions of the file it replaces (not its owner, ACLs or extended
attributes). On any error or cancel an existing output is left as it was and no partial
file remains. A symbolic link at the output path is replaced, not followed.

`convert_file_to` takes `OutputOptions` as well; with `no_clobber` set, anything at the
output path is refused with `Error::Io` of kind `AlreadyExists`, before converting and again
when the result is put in place, which never replaces a file that appeared meanwhile
([design](design.md#output-files)).

The `dsd2pcm` tool treats Ctrl-C, SIGINT and SIGTERM (on Windows Ctrl-C and Ctrl-Break) as a
cancel: an interrupted conversion leaves no file and an existing output as it was, and the
tool exits with status 130 (SIGINT) or 143 (SIGTERM); a second signal ends it at once. If the
process is killed outright (SIGKILL, a crash, power loss), a hidden `.dsd2pcm-PID-N.tmp` file
may remain next to the output; it is safe to delete. The library installs no signal handlers.

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
  order: FL FR FC LFE BL BR), or one given to `WavWriter::with_channel_mask`.
- Channel layouts: `DsdRead::channel_mask` gives the speakers a DSD file declares: the DSF
  channel type (specification 1.01: 1 mono C, 2 FL FR, 3 FL FR C, 4 FL FR BL BR, 5 FL FR C
  LFE, 6 FL FR C BL BR, 7 FL FR C LFE BL BR) when it matches the channel count, or the
  DSDIFF `CHNL` IDs (`SLFT`/`MLFT`, `SRGT`/`MRGT`, `C`, `LFE`, `LS`, `RS`) when each is a
  speaker, none twice, in WAV's channel order; otherwise none (numbered IDs such as `C000`,
  an order WAV cannot express). `WavReader::channel_mask` and `FlacReader::channel_mask`
  (also `PcmRead::channel_mask`) give the mask a file declares: `dwChannelMask`, or the
  `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` Vorbis comment. `DsfWriter::set_channel_mask` sets the
  DSF type from a mask (the type of the count when no type has that mask). A DSD to WAV
  conversion writes the input's layout as the WAV channel mask (since 0.2), or the mask of
  the channel count when the input declares none.
- Converts DSD64 to PCM. The command-line tool converts DSF and DFF only; WAV and FLAC
  input is read by the library.

## Use the library

`dsd2pcm::convert_file` converts a DSF or DFF file to a WAV file with `PcmOptions` (rate,
cutoff, filter, gain, threads), the same conversion as the tool.

`dsd2pcm::frames::Converter` converts SACD frames (channels interleaved byte by byte, 4704
bytes per channel per frame) a run at a time, with the frames before and after the run as
context, into interleaved 24-bit PCM, with clip counts per frame. Its interface is that of
scarletbook-pcm 0.7.0's `Converter::convert`, and with `Filter::Fast` so are its output
bytes; any run with its
neighbours gives the bytes of that run in a whole-area conversion. One converter per
thread; after the first call it allocates nothing.

`convert_file_with` and `dsd64_to_pcm_with` take a progress hook: it gets the output
samples done and the total at each block, on the calling thread, and stops the conversion
with `Error::Cancelled` when it returns `ControlFlow::Break`. `Error` is `#[non_exhaustive]`.

A project that depends on the crate gets the best speed with the build settings in
[options.md](options.md#build-settings-for-consumers). The API reference is on
[docs.rs](https://docs.rs/dsd2pcm); the C library is described in [c-api.md](c-api.md).

## Limits

Limits of this version: DSD64 input only for PCM output; WAV output up to 4 GiB.
[plan.md](plan.md) lists what comes next.

## Troubleshooting

| Message | Cause |
|---|---|
| `unsupported: input is PCM, not DSD` | The input is a WAV or FLAC file: the tool converts DSF and DFF only. |
| `invalid argument: the wide filter is not available at 88.2 kHz` | `--cutoff wide` needs `--rate 176400` or `--rate 352800`. |
| `invalid argument: the pysacd filter has 88.2 and 176.4 kHz only` | `--filter pysacd` needs `--rate 88200` or `--rate 176400`. |
| `invalid argument: gain 30 dB is outside -120 to +24 dB` | `--gain` takes -120 to +24. |
| `unsupported: DST-compressed DSDIFF; decompress it first` | The DFF file is DST-compressed; decompress it to plain DFF or DSF first. |
| `invalid argument: output is the input file` | The output path names the input file (also through a link). |
| `invalid argument: output path names a directory, not a file` | Give a file name, not a folder. |
| `unsupported: WAV output over 4 GiB (RF64 is not supported yet)` | Split the input, or use a lower rate. |

A non-zero clipped count means the filter's output exceeded full scale on those samples;
a negative `--gain` (for example `--gain -3`) avoids it.

## See also

- [Options](options.md): every command-line option and its default
- [C interface](c-api.md): the C library
- [Design](design.md): the decisions behind the filters and the output
- [Performance](performance.md): speed of `precise` and `fast` against scarletbook-pcm 0.7.0

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
