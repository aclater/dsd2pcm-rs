[dsd2pcm-rs](../README.md) › Documentation

<img src="https://raw.githubusercontent.com/aclater/dsd2pcm-rs/main/docs/assets/icon-256.png" alt="" width="96">

# Documentation

The documentation of dsd2pcm: the command-line tool, the Rust crate and the C library.
Start with the [Quick start](../README.md#quick-start) in the README.

## Install

- [Platforms](../README.md#platforms): release archives, `cargo install dsd2pcm-cli`
- [C library binary releases](c-api.md#binary-releases): archives, checksums and attestations

## Use

- [User guide](user-guide.md): converting files, rates and filters, formats, troubleshooting
- [Options](options.md): every command-line option, the environment and the exit status

## Reference

- [Rust API](https://docs.rs/dsd2pcm) on docs.rs
- [C interface](c-api.md): the C library, its rules and its ABI

## Internals

- [Design](design.md): the design decisions and their reasons, and how the conversion works
- [Performance](performance.md): speed against scarletbook-pcm 0.7.0, including the gaps
- [Fuzzing](fuzzing.md): the fuzz targets and how to run them
- [Testing for long-lived processes](testing.md): soak, leak loop, Miri, ThreadSanitizer, valgrind

## Project

- [Changelog](../CHANGELOG.md) · [Plan](plan.md)
- [Contributing](../CONTRIBUTING.md) · [Development](development.md) · [Releases](release.md)
  · [Release test plan](release-test-plan.md)
- [Security policy](../SECURITY.md) · [Provenance](../PROVENANCE.md)
- [Icons](assets/README.md): the project icon set, part of the DSD Toolbox family

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
