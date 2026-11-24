[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Development

# Development

CI (`.github/workflows`) runs fmt, clippy, the docs build and the tests on Linux (x86_64,
aarch64, i686), macOS (arm64) and Windows (x86_64), the tests once per SIMD level the CPU
has, plus a short soak test, the C tests under AddressSanitizer and LeakSanitizer, the
minimum Rust version, cargo-deny, a fuzz smoke test, CodeQL, zizmor and the check that
the precise filter's tables are what `tools/filters/design_precise.py --check` generates; Miri and
ThreadSanitizer run weekly ([testing.md](testing.md)). The speed workflow (`bench.yml`, by
hand only) is described in [docs/performance.md](performance.md).

Rust versions: the library, the command-line tool and the C library build with Rust 1.83
(`rust-version` in `Cargo.toml`). The test suite needs Rust 1.85 or later, because its
dev-dependency scarletbook-pcm 0.7.0 uses edition 2024 (rust-version 1.85).
`.github/workflows/msrv.yml` checks and builds the workspace with 1.83 and builds the tests
with 1.85.
Locally:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_PROFILE_DEV_OPT_LEVEL=2 .github/scripts/test-simd-levels.sh
```

Releases are built, tested and published by `.github/workflows/release.yml` from an
annotated `v*` tag; [docs/release.md](release.md) lists what is published and the
manual steps.

`.github/workflows/docs.yml` keeps the docs true: it checks every link in the README,
CHANGELOG, SECURITY.md and `docs/` (lychee, `.github/lychee.toml`), runs the README's
Quick start block literally on the release it names, and fails when the workspace version,
the lock files, the newest CHANGELOG section and the release references in the docs
disagree (`python3 -I .github/scripts/check-doc-versions.py`).

## See also

- [Contributing](../CONTRIBUTING.md): how changes get in
- [Testing](testing.md): checks for long-lived processes
- [Fuzzing](fuzzing.md): the fuzz targets
- [Releases](release.md): publishing a version

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
