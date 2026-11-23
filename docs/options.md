[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Options

# Options

Every option of the `dsd2pcm` command-line tool, its default and its limits, and the build
settings for projects that use the crate. The [user guide](user-guide.md) explains when to
use them.

## Synopsis

```text
dsd2pcm IN OUT [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|pysacd] [--single-thread]
```

`IN` is a DSF or DFF file (DSD64); `OUT` is the WAV file to write (24-bit PCM). An existing
`OUT` is replaced only when the conversion succeeds
([output files](user-guide.md#output-files-and-interruptions)). Options may come before or
after the file names.

## Options

| Option | Values | Default | Notes |
|---|---|---|---|
| `--rate` | `88200`, `176400`, `352800` (Hz) | `176400` | Output sample rate. |
| `--cutoff` | `30`, `40`, `wide` | `40` | Filter: passband to 30 kHz, 40 kHz, or 80 kHz (`wide`, 176.4 and 352.8 kHz only). `80` is accepted for `wide`. |
| `--gain` | -120 to +24 (dB) | `0` | Applied before quantising to 24 bits. |
| `--filter` | `default`, `pysacd` | `default` | `pysacd`: pysacd's filter and dithered quantiser, 88.2 and 176.4 kHz only, standard cutoff only. |
| `--single-thread` | | all cores | Convert on the calling thread only. |
| `-h`, `--help` | | | Print the usage to standard output and exit. |

## Environment

| Variable | Values | Effect |
|---|---|---|
| `DSD2PCM_SIMD` | `generic`, `sse2`, `avx2`, `avx2fma` | Caps the SIMD level picked at run time (`sse2` and above on x86 and x86_64 only). The output bytes are the same at every level. |

## Exit status

| Status | Meaning |
|---|---|
| 0 | Converted, or `--help`. |
| 1 | The conversion failed or the options do not fit together; the message names the reason. |
| 2 | Usage error: an unknown option, a missing value or a missing file name. |
| 130 | Interrupted by SIGINT (Ctrl-C). |
| 143 | Interrupted by SIGTERM. |

## Build settings for consumers

dsd2pcm's kernels are fastest when the compiler sees each loop whole. A release profile
with many codegen units and overflow checks (16 units, no LTO, overflow checks on) measured
25 % slower; the output bytes are the same. Until 0.1.3 makes the kernels robust to such
profiles, a project that depends on the crate can set, in its own `Cargo.toml`:

```toml
[profile.release.package.dsd2pcm]
codegen-units = 1
overflow-checks = false
```

## See also

- [User guide](user-guide.md): formats, filters, output files and troubleshooting
- [C interface](c-api.md): the same options for C programs
- [Design](design.md): the filters, the SIMD levels and consumer profiles

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
