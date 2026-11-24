[dsd2pcm-rs](../../README.md) › [Documentation](../../README.md#documentation) › [Release quality comparison](README.md) › X.Y.Z

# Quality of X.Y.Z against W.W.W

Measured with dsd-measure 0.1.1 (commit d234b7a) and [suites/dsd2pcm.toml](../../suites/dsd2pcm.toml)
on CPU MODEL, both versions built from their tags, on YYYY-MM-DD. Changes beyond the suite's
repeatability (any change of 0.01 dB or more; a null that is no longer identical) are in bold.

| Filter | Cutoff | Rate (kHz) | Version | SINAD (dB) | THD+N (dB) | Noise, 20 Hz-20 kHz (dBFS) | Alias rejection (dB) | Null vs previous |
|---|---|---:|---|---:|---:|---:|---:|---|
| default | 40 | 88.2 | W.W.W | | | | | baseline |
| default | 40 | 88.2 | X.Y.Z | | | | | |
| default | 40 | 176.4 | W.W.W | | | | | baseline |
| default | 40 | 176.4 | X.Y.Z | | | | | |
| default | 40 | 352.8 | W.W.W | | | | not measurable | baseline |
| default | 40 | 352.8 | X.Y.Z | | | | not measurable | |
| default | wide | 176.4 | W.W.W | | | | | baseline |
| default | wide | 176.4 | X.Y.Z | | | | | |
| default | wide | 352.8 | W.W.W | | | | not measurable | baseline |
| default | wide | 352.8 | X.Y.Z | | | | not measurable | |
| pysacd | fixed | 88.2 | W.W.W | | | | | baseline |
| pysacd | fixed | 88.2 | X.Y.Z | | | | | |
| pysacd | fixed | 176.4 | W.W.W | | | | | baseline |
| pysacd | fixed | 176.4 | X.Y.Z | | | | | |

CONCLUSION: one sentence, for example "Every figure is unchanged and every null is
identical: X.Y.Z converts exactly as W.W.W did."

## See also

- [Release quality comparison](README.md): how this table is made and read
- [Usage](../usage.md): the filters, rates and cutoffs

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
