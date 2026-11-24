# Contributing

Bug reports, fixes and improvements are welcome. This page says how changes get in; the
rules every change follows are in [AGENTS.md](AGENTS.md).

## Reporting

- Bugs and requests: a GitHub issue with the version, the platform and CPU, the command or
  call, and a small input file if one is needed to reproduce it.
- Security issues: privately, as [SECURITY.md](SECURITY.md) describes; never in a public
  issue.

## Changes

- Every change goes through a pull request against `main`; the required CI checks must pass
  on a branch that is up to date with `main`.
- Output is deterministic within a release: the same version, input and settings give the
  same bytes on every CPU and SIMD level, and the tests hold golden hashes at every level.
  Between releases output may change when measured quality improves; every such change is
  listed in the CHANGELOG with its measured effect (`docs/quality/<version>.md`). Releases up
  to 0.1.2 keep their behaviour as released.
- `unsafe` code, parsers and untrusted-input handling get a security review first.
- Update the README and the docs in the same pull request where the change needs it.
- [docs/development.md](docs/development.md) has the commands CI runs, to run them locally
  first; [docs/fuzzing.md](docs/fuzzing.md) covers the fuzz targets.

## Licence

Unless you state otherwise, any code contribution you intentionally submit for inclusion in
this work, as defined in the Apache-2.0 licence, is dual-licensed under the MIT licence and the
Apache License, Version 2.0, without any additional terms or conditions. Contributions to the
documentation and measurement data (the parts the README's
[Licence and credits](README.md#licence-and-credits) lists) are licensed under CC BY 4.0
([LICENSES/CC-BY-4.0.txt](LICENSES/CC-BY-4.0.txt)). Material taken from elsewhere keeps its
own terms: name its source and licence in the change and in [PROVENANCE.md](PROVENANCE.md).

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
