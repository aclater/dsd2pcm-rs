[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Options

# Options

Every option of the `dsd2pcm` command-line tool, its default and its limits, and the build
settings for projects that use the crate. The [user guide](user-guide.md) explains when to
use them.

## Synopsis

```text
dsd2pcm [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter precise|fast|pysacd] [--single-thread | --threads N] [-n|--no-clobber] [--] IN OUT
```

`IN` is a DSF or DFF file (DSD64); `OUT` is the WAV file to write (24-bit PCM). An existing
`OUT` is replaced only when the conversion succeeds, and never with `-n`
([output files](user-guide.md#output-files-and-interruptions)). Options and file names may
come in any order. An argument that starts with `-` is an option; `--` ends the options, so
`dsd2pcm -- -take1.dsf -take1.wav` converts a file whose name starts with `-`. File names
are passed to the system as they are, in any encoding it allows (on Linux any bytes);
option values must be UTF-8, and a value that is not is a usage error.

## Options

| Option | Values | Default | Notes |
|---|---|---|---|
| `--rate` | `88200`, `176400`, `352800` (Hz) | `176400` | Output sample rate. |
| `--cutoff` | `30`, `40`, `wide` | `40` | Filter: passband to 30 kHz, 40 kHz, or 80 kHz (`wide`, 176.4 and 352.8 kHz only). `80` is accepted for `wide`. |
| `--gain` | -120 to +24 (dB) | `0` | Applied before quantising to 24 bits. |
| `--filter` | `precise`, `fast`, `pysacd` | `precise` | `precise`: at least 150 dB of stopband, every rate and cutoff; `default` is accepted for it. `fast`: scarletbook-pcm 0.7.0's filter, byte-identical with it, lower quality (about 110 dB of stopband) at about 0.55 times precise's CPU time ([performance](performance.md#precise-against-fast-by-rate-and-cutoff)); the default up to 0.1.2. `pysacd`: pysacd's filter and dithered quantiser computed in `f64`, 88.2 and 176.4 kHz only, standard cutoff only. |
| `--single-thread` | | all cores | Convert on the calling thread only. |
| `--threads` | 1 to 256 | all cores | Convert on at most N threads, the calling one included (`Threads::Count` in Rust, `threads` in C). `1` is `--single-thread`; 0, a count above 256 or anything else is a usage error. The output bytes are the same with any count. |
| `-n`, `--no-clobber` | | off | Refuse an existing `OUT` (exit status 1, the file left as it was), and never replace a file that appears at `OUT` during the conversion. |
| `--` | | | Ends the options: every later argument is a file name. |
| `-h`, `--help` | | | Print the usage to standard output and exit. |
| `-V`, `--version` | | | Print the version and the SIMD level in use to standard output, and exit. No `IN` or `OUT` is needed. |

`--version` prints two lines:

```
dsd2pcm 0.2.1
simd: avx2fma
```

The second line is the level the conversion kernels use on this machine: `generic`, `sse2`, `avx2` or `avx2fma`, the best the CPU has, lowered by `DSD2PCM_SIMD` if that is set (see [Environment](#environment)). Every level gives the same output bytes; the line is for bug reports and benchmarks. A machine without the x86 levels (Apple silicon, any aarch64 machine) reports `generic`. The set of names may grow in later versions, so a script should print it, not match on it.

## Environment

| Variable | Values | Effect |
|---|---|---|
| `DSD2PCM_SIMD` | `generic`, `sse2`, `avx2`, `avx2fma` | Caps the SIMD level picked at run time (`sse2` and above on x86 and x86_64 only). The output bytes are the same at every level. `dsd2pcm --version` shows the level that results. |

## Exit status

| Status | Meaning |
|---|---|
| 0 | Converted, or `--help` or `--version`. |
| 1 | The conversion failed or the options do not fit together; the message names the reason. |
| 2 | Usage error: an unknown option, a missing value, an option value that is not UTF-8, or a missing file name. |
| 130 | Interrupted by SIGINT (Ctrl-C). |
| 143 | Interrupted by SIGTERM. |

## Build settings for consumers

From 0.2.0 no settings are needed: a release profile with many codegen units and overflow
checks (16 units, no LTO, overflow checks on) keeps the crate's speed, which CI checks on
every pull request ([design](design.md#release-profiles-of-consumers)). Up to 0.1.2 such a
profile measured 25 % slower, with the same output bytes; a project that depends on 0.1.2
can set, in its own `Cargo.toml`:

```toml
[profile.release.package.dsd2pcm]
codegen-units = 1
overflow-checks = false
```

## See also

- [User guide](user-guide.md): formats, filters, output files and troubleshooting
- [C interface](c-api.md): the same options for C programs
- [Design](design.md): the filters, the SIMD levels and consumer profiles

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
