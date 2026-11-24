# Security policy

dsd2pcm reads files from untrusted sources, so parser and memory-safety bugs are treated as
security issues. This page says which versions get fixes and how to report a problem
privately.

## Supported versions

| Version | Security fixes |
|---|---|
| 0.1.x, latest release | Yes |
| Older 0.1.x releases | No: update to the latest 0.1.x |

A fix is released as a new patch version; a published version or tag is never changed.

## Scope

- The `dsd2pcm` library crate: the DSF, DFF, WAV and FLAC readers, the writers and the
  conversion.
- The `dsd2pcm` command-line tool (`dsd2pcm-cli`).
- The C library (`libdsd2pcm`, `include/dsd2pcm.h`) and its interface rules in
  [docs/c-api.md](docs/c-api.md).
- The release artefacts: the archives on the GitHub releases page, their checksums and
  build provenance attestations, the macOS signatures, and the crates on crates.io.
- The release and CI workflows under `.github/`.

Out of scope: bugs in dependencies that do not affect dsd2pcm (report those upstream), and
denial of service by files that are simply very large: a conversion takes time and disk
space in proportion to its input.

## Reporting a vulnerability

Do not open a public issue. Report it privately through GitHub: the repository's
**Security** tab, **Report a vulnerability**
([direct link](https://github.com/dsd-toolbox/dsd2pcm-rs/security/advisories/new)).

Please include:

- the version (`cargo tree -p dsd2pcm`, the release archive's name, or the commit);
- the platform and CPU (the SIMD level matters: `DSD2PCM_SIMD` shows whether it depends on
  one);
- what happens (panic, crash, hang, wrong output, excessive memory) and what you expected;
- a minimal input file or a program that reproduces it, and the exact command or call.

## What to expect

This is a small project. We aim to acknowledge a report within a week and to keep you
informed while it is investigated. A confirmed issue is fixed in a patch release and
published as a GitHub security advisory, with credit to you unless you prefer otherwise.
Please give us the chance to release a fix before you disclose the issue.

## How the project guards against issues

- Parsers are bounds-checked and return errors instead of panicking; the readers are fuzzed
  ([docs/fuzzing.md](docs/fuzzing.md)) on every pull request and weekly.
- `unsafe` code is confined to the SIMD dispatch and the C interface, each block with a
  safety note ([AGENTS.md](AGENTS.md)).
- Dependency advisories are watched: cargo-deny checks the RustSec advisory database on every
  pull request and weekly, and Dependabot proposes dependency updates.
- Releases are built in CI from an annotated tag on `main`, with build provenance
  attestations ([docs/release.md](docs/release.md)).

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
