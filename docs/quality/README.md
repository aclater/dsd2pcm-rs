[dsd2pcm-rs](../../README.md) › [Documentation](../README.md) › Release quality comparison

# Release quality comparison

Every release from 0.2.0 on publishes `docs/quality/<version>.md`, which measures it against
the previous release.

Both versions are built from source (the release candidate and the previous release's tag) and measured on one machine with
[dsd-measure](https://github.com/dsd-toolbox/dsd-measure) 0.1.2 (commit 3eaec2d, pinned in
[suites/dsd2pcm.toml](../../suites/dsd2pcm.toml)): the same synthetic DSD64 signals go
through every filter at every rate it supports (precise and fast, fast being 0.1.x's
default, at 88.2, 176.4 and 352.8 kHz with cutoff 40, plus cutoff wide at 176.4 and
352.8 kHz; pysacd at 88.2 and 176.4 kHz), and the
table gives SINAD (−THD+N) and THD of a 1 kHz tone at 0 dB SACD, the noise floor from 20 Hz
to 20 kHz, alias rejection (88.2 and 176.4 kHz only) and a null test of the new output against
the old one on the suite's multitone signal. The suite is deterministic: three runs on the same
two builds gave the same figures to the last digit (0.1.1 suite, 2026-11-24; repeated with
dsd-measure 0.1.2 for [0.2.0](0.2.0.md#repeatability)). That is the run-to-run spread on one fixed set of signals, not a variation
over other signals or modulators. So a change of 0.01 dB or more, or a
null that is no longer identical, is a real change and is flagged, and a regression blocks the
release unless it is accepted; an accepted change or regression is recorded, with its
reason, in that version's page and in its CHANGELOG entry. The exact commands are in
[item 11 of the release test plan](../release-test-plan.md#11-quality-comparison-with-the-previous-release),
the page layout in the [template](template.md). Every page states, in its "Measured with"
line, the two dsd2pcm commits, the dsd-measure version and commit and the CPU model. The
figures are what the suite reads; they make no claim about audibility.

## Pages

- [0.2.0](0.2.0.md) against 0.1.2: the precise default and the f64 pysacd filter measure
  better than or the same as 0.1.2 in every figure; the fast filter converts exactly as
  0.1.2's default did ([raw results](data/0.2.0/README.md))

## Raw data

Each page's raw results are in `data/<version>/`: dsd-measure's `results.json` and
`provenance.json` for every device and every null test, and the comparison's `results.json`,
as the suite wrote them, with the absolute paths of the two builds made relative to the
repository root (`target/quality/new/bin/dsd2pcm`). They record each converter's SHA-256,
the suite file's SHA-256 and the SHA-256 of every input and output file. The figures in a
page are read from these files.

## Licence

The published pages here (this README included), the suite, the figures they hold and the raw data under `data/` are licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
([LICENSES/CC-BY-4.0.txt](../../LICENSES/CC-BY-4.0.txt)), like the rest of the documentation;
the code stays MIT OR Apache-2.0
([Licence and credits](../../README.md#licence-and-credits)).

## See also

- [Template](template.md)
- [Measurement data](data/README.md): checksums behind published measurements
- [Release test plan](../release-test-plan.md#11-quality-comparison-with-the-previous-release): item 11
- [User guide](../user-guide.md#choose-the-rate-and-the-filter): the filters, rates and cutoffs

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
