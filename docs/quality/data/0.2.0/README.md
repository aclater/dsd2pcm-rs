[dsd2pcm-rs](../../../../README.md) › [Documentation](../../../README.md) › [Release quality comparison](../../README.md) › Raw results for 0.2.0

# Raw results for 0.2.0

The dsd-measure 0.1.2 output behind [Quality of 0.2.0 against 0.1.2](../../0.2.0.md), from
the first of its three runs (the three gave the same figures).

- `report/<device>/results.json` and `provenance.json`: one folder per device of
  [suites/dsd2pcm.toml](../../../../suites/dsd2pcm.toml), `new` being 0.2.0 and `old` 0.1.2.
  The table's figures are under `metrics` (the keys are listed at the top of the suite file).
- `report/comparison/results.json`: all devices side by side.
- `null/<filter>-<cutoff>-<rate>/` and `null/precise-vs-old-fast-<cutoff>-<rate>/`: the null
  tests, `results.json` and `provenance.json`.
- `plots/`: the three graphs on the 0.2.0 page, as dsd-measure drew them.

The absolute paths of the two builds are made relative to the repository root; nothing else
is changed ([raw data](../../README.md#raw-data)).

## Licence

These files are licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
([LICENSES/CC-BY-4.0.txt](../../../../LICENSES/CC-BY-4.0.txt)), like the rest of the
documentation.

## See also

- [Quality of 0.2.0 against 0.1.2](../../0.2.0.md): the page these files support
- [Release quality comparison](../../README.md): how the comparison is made

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
