[dsd2pcm-rs](../README.md) › Documentation

<img src="https://raw.githubusercontent.com/dsd-toolbox/dsd2pcm-rs/main/docs/assets/icon-256.png" alt="" width="96">

# Documentation

The documentation of dsd2pcm: the command-line tool, the Rust crate and the C library.
Start with the [Quick start](../README.md#quick-start) in the README.

## Install

- [Platforms](../README.md#platforms): release archives, `cargo install dsd2pcm-cli`
- [Releases](https://github.com/dsd-toolbox/dsd2pcm-rs/releases): the latest is <!-- facts:release repo=dsd-toolbox/dsd2pcm-rs fmt="{version} ({date})" -->0.2.0 (2026-11-25)<!-- /facts -->
- [C library binary releases](c-api.md#binary-releases): archives, checksums and attestations

## Use

- [User guide](user-guide.md): converting files, rates and filters, formats, troubleshooting
- [Options](options.md): every command-line option, the environment and the exit status

## Reference

- [Rust API](https://docs.rs/dsd2pcm) on docs.rs
- [C interface](c-api.md): the C library, its rules and its ABI

## Internals

- [Design](design.md): the design decisions and their reasons, and how the conversion works
- [Performance](performance.md): speed of `precise` and `fast` against scarletbook-pcm 0.7.0, including the gaps
- [PCM quality](pcm-quality.md): the measured audio quality of every filter and rate
- [Release quality comparison](quality/README.md): each release measured against the previous one
- [Fuzzing](fuzzing.md): the fuzz targets and how to run them
- [Testing for long-lived processes](testing.md): soak, leak loop, Miri, ThreadSanitizer, valgrind

## Project

- [Changelog](../CHANGELOG.md) · [Plan](plan.md)
- [Contributing](../CONTRIBUTING.md) · [Development](development.md)
- [Security policy](../SECURITY.md) · [Provenance](../PROVENANCE.md)
- [Icons](assets/README.md): the project icon set, part of the DSD Toolbox family

## Release

- [Releases](release.md): how a release is made, signed and published
- [Release test plan](release-test-plan.md): the tests every release passes before its tag
- [After the release: run the docs audit](release.md#after-the-release-run-the-docs-audit)

## See also

- [README](../README.md): what dsd2pcm is, its platforms and the Quick start
- [dsd-measure](https://github.com/dsd-toolbox/dsd-measure): the measurement toolkit behind the quality pages
- [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox): the other projects and the shared documentation standard

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
