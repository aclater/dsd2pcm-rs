[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Options

# Options

Every option of the `dsd2pcm` command-line tool, its default and its limits. The
[user guide](user-guide.md) explains when to use them.

## Synopsis

```text
dsd2pcm IN OUT [--rate 88200|176400|352800] [--cutoff 30|40|wide] [--gain DB] [--filter default|pysacd] [--single-thread]
```

`IN` is a DSF or DFF file (DSD64); `OUT` is the WAV file to write (24-bit PCM). An existing
`OUT` is replaced. Options may come before or after the file names.

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
| 1 | The conversion failed; the message names the reason. |
| 2 | Usage error: an unknown option, a missing value or a missing file name. |

## See also

- [User guide](user-guide.md): formats, filters and troubleshooting
- [C interface](c-api.md): the same options for C programs
- [Design](design.md): the filters and the SIMD levels

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
