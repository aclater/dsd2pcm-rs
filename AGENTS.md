# Rules for contributors and agents

- **Licence boundary.** No GPL or LGPL code, tables or coefficients. Code ported from permissively
  licensed projects (for example scarletbook-pcm, MIT OR Apache-2.0) keeps its notice; record every
  source in `PROVENANCE.md`.
- **Output stability.** Output bytes for given input and settings never change without a major
  version and a release note. Byte-identical across x86_64 and aarch64, SIMD levels and build
  profiles; golden hashes in tests.
- **FMA only where exactness is proven.** Fused multiply-adds only in stages whose sums the
  tests prove exact from the tables (`decimator/exact.rs`); no `mul_add` in the generic kernels.
- **Untrusted input.** Parsers are bounds-checked, never panic on bad files, no `unsafe` without review.
  `unsafe` lives only in `decimator/dispatch.rs` (calls into `#[target_feature]` builds) and
  in the C interface's `crates/dsd2pcm-capi/src/ffi.rs` (and its tests), each block with a
  `// SAFETY:` note.
- **C interface.** Regenerate `crates/dsd2pcm-capi/include/dsd2pcm.h` with cbindgen after
  any change to the exported items; `crates/dsd2pcm-capi/check.sh` checks it and runs the C
  tests (gcc, clang, ASan). The C ABI is a contract: only append struct fields and add
  items; anything incompatible bumps `DSD2PCM_ABI_VERSION` and the SONAME (docs/c-api.md).
- Every change goes through a pull request: the `main` ruleset requires the CI checks and a
  branch up to date with `main`. `unsafe`, parsers and untrusted-input handling get a
  security review first. Every change updates README and docs where needed.
