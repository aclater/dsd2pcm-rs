[dsd2pcm-rs](../../README.md) › [Documentation](../README.md) › [Release quality comparison](README.md) › Quality of X.Y.Z against W.W.W

# Quality of X.Y.Z against W.W.W

Measured with: dsd-measure 0.1.2 (commit 3eaec2d) and [suites/dsd2pcm.toml](../../suites/dsd2pcm.toml);
dsd2pcm X.Y.Z at commit CANDIDATE-SHA and W.W.W from tag vW.W.W (commit TAG-SHA), both built
with Rust RUST-VERSION; on CPU MODEL (the model as `lscpu` prints it, not a host name),
on YYYY-MM-DD. Changes beyond the suite's measured run-to-run spread (any change of 0.01 dB
or more; a null that is no longer identical) are in bold.

SINAD is −THD+N; THD counts the harmonics only. The raw results are in `data/X.Y.Z/`
([raw data](README.md#raw-data), CC BY 4.0).

| Filter | Cutoff | Rate (kHz) | Version | SINAD (dB) | THD (dB) | Noise, 20 Hz-20 kHz (dBFS) | Alias rejection (dB below the tone) | Null vs previous |
|---|---|---:|---|---:|---:|---:|---:|---|
| fast | 40 | 88.2 | W.W.W | | | | | baseline |
| fast | 40 | 88.2 | X.Y.Z | | | | | |
| fast | 40 | 176.4 | W.W.W | | | | | baseline |
| fast | 40 | 176.4 | X.Y.Z | | | | | |
| fast | 40 | 352.8 | W.W.W | | | | not measurable | baseline |
| fast | 40 | 352.8 | X.Y.Z | | | | not measurable | |
| fast | wide | 176.4 | W.W.W | | | | | baseline |
| fast | wide | 176.4 | X.Y.Z | | | | | |
| fast | wide | 352.8 | W.W.W | | | | not measurable | baseline |
| fast | wide | 352.8 | X.Y.Z | | | | not measurable | |
| pysacd | fixed | 88.2 | W.W.W | | | | | baseline |
| pysacd | fixed | 88.2 | X.Y.Z | | | | | |
| pysacd | fixed | 176.4 | W.W.W | | | | | baseline |
| pysacd | fixed | 176.4 | X.Y.Z | | | | | |
| precise | 40 | 88.2 | X.Y.Z | | | | | against W.W.W fast |
| precise | 40 | 176.4 | X.Y.Z | | | | | against W.W.W fast |
| precise | 40 | 352.8 | X.Y.Z | | | | not measurable | against W.W.W fast |
| precise | wide | 176.4 | X.Y.Z | | | | | against W.W.W fast |
| precise | wide | 352.8 | X.Y.Z | | | | not measurable | against W.W.W fast |

`fast` is the default filter of 0.1.x; from 0.2.0 the default is `precise`. While the
previous release is 0.1.x, each precise row's null is against its default (fast) at the same
cutoff and rate. Once the previous release has the precise filter, add a W.W.W row before
each precise row and null it against that.

CONCLUSION: one sentence, for example "Every figure is unchanged and every null is
identical: X.Y.Z converts exactly as W.W.W did."

## See also

- [Release quality comparison](README.md): how this table is made and read
- [User guide](../user-guide.md#choose-the-rate-and-the-filter): the filters, rates and cutoffs

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
