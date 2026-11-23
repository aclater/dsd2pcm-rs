# Rules for contributors and agents

- **Licence boundary.** No GPL or LGPL code, tables or coefficients. Code ported from permissively
  licensed projects (for example scarletbook-pcm, MIT OR Apache-2.0) keeps its notice; record every
  source in `PROVENANCE.md`.
- **Output stability.** Output bytes for given input and settings never change without a major
  version and a release note. Byte-identical across x86_64 and aarch64, SIMD levels and build
  profiles; golden hashes in tests.
- **Untrusted input.** Parsers are bounds-checked, never panic on bad files, no `unsafe` without review.
- Every change goes through review with green checks; every change updates README and docs where needed.
