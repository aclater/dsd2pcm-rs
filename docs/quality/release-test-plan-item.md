[dsd2pcm-rs](../../README.md) › [Documentation](../../README.md#documentation) › [Release quality comparison](README.md) › Release test plan item

# Release test plan item: quality comparison

The text of the release test plan's quality item, kept here until it is folded into
`docs/release-test-plan.md`. The example is the first published comparison, 0.1.3 against
0.1.2; for a later release, change the two tags and the file name.

## 11. Quality comparison with the previous release

Measures the candidate and the previous release with [dsd-measure](https://github.com/dsd-toolbox/dsd-measure)
0.1.1 and [suites/dsd2pcm.toml](../../suites/dsd2pcm.toml), on one machine, and writes
`docs/quality/0.1.3.md`. Nothing is timed, so it runs in parallel with the other items.
It needs Python 3.11 or later, a C compiler and about 15 minutes with the builds (the
measurements take 5 minutes and 1.7 GB of memory on an AMD Ryzen AI Max+ 395).

Install the pinned measurement suite and build the two versions under `target/quality/`
(the candidate from its branch, the previous release from its tag):

```sh
pipx install git+https://github.com/dsd-toolbox/dsd-measure@d234b7a8233ccfe391776e7c6425cbf8643c47e3
dsd-measure --version
cargo install --locked --path crates/dsd2pcm-cli --root target/quality/new
cargo install --locked --git https://github.com/aclater/dsd2pcm-rs --tag v0.1.2 --root target/quality/old dsd2pcm-cli
```

`dsd-measure --version` must print `0.1.1`. Measure both, from the repository root:

```sh
dsd-measure run suites/dsd2pcm.toml --only reference_floor levels noise alias_tones
```

Then the null test of each filter, rate and cutoff, new against old, on the suite's
multitone signal. Its SHA-256 must be
`100d1ba2cb46d98374ee5b8591b995712d12cd21c7d9370d50a1d68da3c030fc`:

```sh
dsd-measure signals target/quality/signals --profile quick --only multitone
sha256sum target/quality/signals/multitone.dsf
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old default 40 88200" --device "new default 40 88200" -o target/quality/null/default-40-88200
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old default 40 176400" --device "new default 40 176400" -o target/quality/null/default-40-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old default 40 352800" --device "new default 40 352800" -o target/quality/null/default-40-352800
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old default wide 176400" --device "new default wide 176400" -o target/quality/null/default-wide-176400
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old default wide 352800" --device "new default wide 352800" -o target/quality/null/default-wide-352800
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old pysacd 88200" --device "new pysacd 88200" -o target/quality/null/pysacd-88200
dsd-measure null --suite suites/dsd2pcm.toml --source target/quality/signals/multitone.dsf --device "old pysacd 176400" --device "new pysacd 176400" -o target/quality/null/pysacd-176400
```

Copy [the template](template.md) to `docs/quality/0.1.3.md` and fill it from
`target/quality/report/<device>/results.json` (the keys are listed at the top of the suite
file) and the first line of each `target/quality/null/*/index.md`.

Pass: every command exits 0, and either nothing changed (every figure the same to
0.01 dB, the suite's measured repeatability being 0.00 dB, and every null "identical"), or
each change is one the release notes announce. A worse figure is a FAIL unless the
release notes explain it and the decision to accept it is recorded with its reason.

Results table row:

```
| 11 | quality comparison with the previous release (docs/quality/X.Y.Z.md) | | |
```

## See also

- [Release quality comparison](README.md)
- [Template](template.md)
- [Usage](../usage.md)

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
