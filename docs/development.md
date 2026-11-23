# Development

CI (`.github/workflows`) runs fmt, clippy, the docs build and the tests on Linux (x86_64,
aarch64, i686), macOS (arm64) and Windows (x86_64), the tests once per SIMD level the CPU
has, plus a short soak test, the C tests under AddressSanitizer and LeakSanitizer, the
minimum Rust version, cargo-deny, a fuzz smoke test, CodeQL and zizmor; Miri and
ThreadSanitizer run weekly ([testing.md](testing.md)). The speed workflow (`bench.yml`, by
hand only) is described in [docs/performance.md](performance.md). The tests need Rust 1.85
or later (a test-only dependency uses edition 2024); the crates themselves build with 1.83.
Locally:

```
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
