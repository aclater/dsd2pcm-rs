# Provenance of the pysacd reference

This directory generates `src/filters/pysacd_taps.rs` and `sha256.txt`, the golden hashes of
`Filter::Pysacd`. It is a test reference only and is not compiled into the crate.

| File | Source |
|---|---|
| `pysacd/dsp.c`, `pysacd/dsp.h` | pysacd `csrc/dsp.c` and `csrc/dsp.h` at commit 94b2c53 (2026-11-23), relicensed MIT OR Apache-2.0 by their copyright holder (pysacd `licenses/README.md`). Changed in dsd2pcm 0.2.0 to compute in `double`; see below. |
| `pysacd/LICENSE-MIT`, `pysacd/LICENSE-APACHE` | pysacd `licenses/dsp-LICENSE-*`, unchanged |
| `pysacd_ref.c` | The quantiser loop follows pysacd `tools/pcm_vectors/pcm_vectors.c` (pysacd PR #197; itself copied from `process_batch()` in `csrc/sacdconv.c`), changed to one stream per area and a per-frame dither seed. Since 0.2.0 the quantiser is in `double` and uses dsd2pcm's `db_to_linear` series. |
| `sacd.h`, `gen.sh`, `inputs.rs` | New |

## The change to `double` (dsd2pcm 0.2.0)

**What changed.** The filter's response is pysacd's: the same Kaiser windows, the same taps
(`H1` in `double`, `H2` and `H3` in `float`), the same TPDF dither and seeding. Only the
arithmetic changed:

- the stage 1 byte tables stay in `double`, as `dsd2pcm_init` computes them, instead of
  being rounded to `float`;
- the signals between stages and the output are `double`;
- each stage 2 and 3 output is `acc += (double)h[k] * x[k]` in tap order, with the `float`
  taps widened exactly;
- the quantiser is `lrint(y * (8388607.0 * db_to_linear(gain)) + (double)dith)`, where
  `db_to_linear` is dsd2pcm's fixed series (`decimator::db_to_linear`), not libm's `pow`, so
  that every gain gives the same bits on every platform.

**Why.** pysacd's filter summed every output in `float`, strictly in tap order. That
rounding error grows with the signal and cost about 0.7 dB of SINAD at 88.2 kHz and 0.25 dB at
176.4 kHz (1 kHz at 0 dB SACD). pysacd's own 0.5.x wheels hid part of it: they are built with
`-ffast-math`, whose vectorised partial sums shorten each sum, but their bytes then differ by
CPU. The `double` path measures above every 0.5.x wheel and gives the same bytes on every CPU
and SIMD level (docs/design.md, "The pysacd preset").

**Build.** `gen.sh` builds with `-O2 -ffp-contract=off`. Contraction must stay off: each
product is rounded before it is added, and a fused multiply-add changes the bits: built for
aarch64 with GCC 16's default contraction, all 16 `f64_*` hashes change (the 24-bit outputs
of these inputs happen to survive it). With `-ffp-contract=off` the list is the same on
aarch64 and x86-64.

**Before 0.2.0.** dsd2pcm 0.1.x matched pysacd's `float` reference build (the unchanged
`dsp.c`, `-O2 -ffp-contract=off`); git history keeps that source and its hashes. Those bytes
are produced only by dsd2pcm 0.1.x.
