# Performance

Speed of DSD64 to 24-bit PCM with the default filter, against scarletbook-pcm 0.7.0, whose
output it matches byte for byte. Numbers are multiples of real time (higher is faster), best
of three runs over 60 s of audio, measured in memory.

- CPUs: Intel Core i9-13900H (P-core: CPU 0; E-core: CPU 12; `powersave` governor) and
  AMD Ryzen 9 3950X (Zen 2, CPU 4, `performance` governor), Fedora, one benchmark at a time.
- Build: release, SIMD level picked at run time (AVX2 with FMA on both CPUs).
- Inputs: a 2-channel and a 6-channel DSF from real SACDs.
- Tools: `cargo run --release --example bench -- FILE.dsf 60` (whole file, DSF to WAV) and
  `cargo run --release --example sacdfs_path -- FILE.dsf 60 all` (runs of SACD frames, as a
  SACD file system reads them).

## Frame runs of four frames, one thread (`sacdfs_path`, mode `run 4`)

| Rate | Cutoff | 2 ch Zen 2 | 2 ch P-core | 2 ch E-core | 6 ch Zen 2 | 6 ch P-core | 6 ch E-core |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.06x | 1.07x | 1.18x | 1.10x | 1.07x | 1.20x |
| 88.2 kHz | Standard | 1.05x | 1.09x | 1.20x | 1.11x | 1.08x | 1.23x |
| 176.4 kHz | Low | 1.09x | 1.12x | 1.20x | 1.12x | 1.13x | 1.22x |
| 176.4 kHz | Standard | 1.08x | 1.12x | 1.19x | 1.11x | 1.12x | 1.25x |
| 176.4 kHz | Wide | 1.09x | 1.04x | 1.10x | 1.12x | 1.04x | 1.10x |
| 352.8 kHz | Low | 1.18x | 1.25x | 1.21x | 1.20x | 1.21x | 1.24x |
| 352.8 kHz | Standard | 1.17x | 1.22x | 1.21x | 1.20x | 1.21x | 1.24x |
| 352.8 kHz | Wide | 1.15x | 1.17x | 1.18x | 1.17x | 1.17x | 1.19x |

Ratio of dsd2pcm's speed to scarletbook-pcm 0.7.0's; below 1.00x is slower.

## One frame per call, one thread (`sacdfs_path`, mode `run 1`)

What a caller that converts one frame at a time, with its neighbours, sees.

| Rate | Cutoff | 2 ch Zen 2 | 2 ch P-core | 2 ch E-core | 6 ch Zen 2 | 6 ch P-core | 6 ch E-core |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.05x | 1.10x | 1.09x | 1.09x | 1.10x | 1.13x |
| 88.2 kHz | Standard | 1.05x | 1.10x | 1.11x | 1.11x | 1.12x | 1.15x |
| 176.4 kHz | Low | 1.03x | 1.07x | 1.09x | 1.07x | 1.08x | 1.09x |
| 176.4 kHz | Standard | 1.04x | 1.09x | 1.10x | 1.07x | 1.08x | 1.11x |
| 176.4 kHz | Wide | 1.07x | 1.10x | 1.06x | 1.09x | 1.10x | 1.07x |
| 352.8 kHz | Low | 1.08x | 1.14x | 1.13x | 1.11x | 1.13x | 1.15x |
| 352.8 kHz | Standard | 1.09x | 1.16x | 1.13x | 1.12x | 1.14x | 1.16x |
| 352.8 kHz | Wide | 1.12x | 1.17x | 1.16x | 1.15x | 1.16x | 1.17x |

## Whole file, 2 channels, one thread

| Rate | Cutoff | Zen 2: scarletbook-pcm 0.7.0 | Zen 2: dsd2pcm | P-core: scarletbook-pcm 0.7.0 | P-core: dsd2pcm | E-core: scarletbook-pcm 0.7.0 | E-core: dsd2pcm |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 292 | 351 | 316 | 360 | 121 | 143 |
| 88.2 kHz | Standard | 340 | 388 | 360 | 398 | 141 | 164 |
| 176.4 kHz | Low | 223 | 237 | 238 | 252 | 85 | 95 |
| 176.4 kHz | Standard | 259 | 267 | 274 | 287 | 101 | 111 |
| 176.4 kHz | Wide | 174 | 184 | 187 | 185 | 66 | 70 |
| 352.8 kHz | Low | 159 | 191 | 176 | 205 | 66 | 77 |
| 352.8 kHz | Standard | 176 | 209 | 197 | 226 | 76 | 88 |
| 352.8 kHz | Wide | 147 | 169 | 161 | 182 | 60 | 68 |

scarletbook-pcm converts on one thread; dsd2pcm uses all cores unless told otherwise.

## aarch64, `neon` level (GitHub-hosted runners, indicative)

The `neon` level (picked on every aarch64 CPU), one thread, against scarletbook-pcm 0.7.0:
median ratio over 5 rounds with the crate order alternating, on generated 60 s 2- and
6-channel test files (not SACD rips; `.github/workflows/bench.yml`). Linux:
`ubuntu-24.04-arm` (Neoverse N2), every round within 0.1 of the median. macOS: `macos-15`
(Apple M1, virtual; shared and noisy, lowest round 1.27x). At the generic level, before
`neon`, the same Linux runner read 0.97x to 1.04x.

| Rate | Cutoff | Linux `run 1` 2 ch / 6 ch | Linux `run 4` 2 ch / 6 ch | Linux whole file 2 ch / 6 ch | macOS `run 1` 2 ch / 6 ch | macOS `run 4` 2 ch / 6 ch | macOS whole file 2 ch / 6 ch |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.83x / 1.82x | 2.06x / 2.03x | 2.08x / 2.03x | 1.60x / 1.46x | 1.59x / 1.71x | 1.61x / 1.63x |
| 88.2 kHz | Standard | 1.79x / 1.77x | 2.00x / 1.98x | 2.00x / 1.97x | 1.50x / 1.50x | 1.62x / 1.61x | 1.76x / 1.68x |
| 176.4 kHz | Low | 2.17x / 2.16x | 2.29x / 2.27x | 2.17x / 2.21x | 1.75x / 1.92x | 1.83x / 1.77x | 1.89x / 1.99x |
| 176.4 kHz | Standard | 2.10x / 2.10x | 2.20x / 2.18x | 2.08x / 2.12x | 1.92x / 1.77x | 1.83x / 1.78x | 1.90x / 2.02x |
| 176.4 kHz | Wide | 2.09x / 2.07x | 2.15x / 2.14x | 2.07x / 2.11x | 1.74x / 1.79x | 1.88x / 1.92x | 1.83x / 1.80x |
| 352.8 kHz | Low | 2.11x / 2.11x | 2.23x / 2.22x | 2.13x / 2.15x | 1.91x / 1.88x | 1.95x / 1.79x | 1.89x / 1.88x |
| 352.8 kHz | Standard | 2.06x / 2.05x | 2.16x / 2.15x | 2.04x / 2.04x | 1.83x / 1.78x | 1.86x / 1.90x | 1.88x / 1.73x |
| 352.8 kHz | Wide | 2.27x / 2.26x | 2.29x / 2.27x | 2.16x / 2.16x | 1.90x / 1.96x | 1.97x / 1.94x | 2.03x / 1.70x |

## 32-bit x86 (i686), one thread

Built for `i686-unknown-linux-gnu` and run on the Intel Core i9-13900H above (P-core: CPU 0;
E-core: CPU 12, a Gracemont core of the Atom family), 2 channels, 30 s of audio. The SIMD
level is forced to `sse2` (`DSD2PCM_SIMD=sse2`), the level that a CPU with SSE2 to SSSE3
and no AVX2 picks, such as the Intel Atom of small NAS units; scarletbook-pcm 0.7.0 has
one path on 32-bit x86. "Before" is the generic level of the kernels before the `sse2`
level existed (what such a CPU ran then).

| Rate | Cutoff | `run 4` P-core before | `run 4` P-core | `run 4` E-core before | `run 4` E-core | Whole file E-core: scarletbook-pcm 0.7.0 | Whole file E-core: dsd2pcm |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.03x | 1.12x | 0.98x | 1.19x | 88.7 | 102.8 |
| 88.2 kHz | Standard | 1.03x | 1.11x | 0.97x | 1.14x | 101.8 | 119.6 |
| 176.4 kHz | Low | 0.90x | 1.17x | 0.92x | 1.19x | 61.3 | 73.4 |
| 176.4 kHz | Standard | 0.93x | 1.10x | 0.91x | 1.15x | 62.3 | 78.4 |
| 176.4 kHz | Wide | 1.02x | 1.14x | 0.97x | 1.19x | 44.8 | 52.7 |
| 352.8 kHz | Low | 0.94x | 1.12x | 1.01x | 1.24x | 48.0 | 60.8 |
| 352.8 kHz | Standard | 0.96x | 1.20x | 0.95x | 1.19x | 55.3 | 69.4 |
| 352.8 kHz | Wide | 0.93x | 1.24x | 0.90x | 1.21x | 45.0 | 52.9 |

Whole-file ratios are 1.12x to 1.27x on both core types. Frame by frame (`run 1`) the
`sse2` level is 1.00x to 1.22x on both core types; one frame at 88.2 kHz is level with
scarletbook-pcm on the P-core (0.96x to 1.00x across runs). With AVX2 and FMA, the level
picked on this CPU, i686 builds run at 1.25x to 1.87x on the P-core and 1.05x to 1.40x on
the E-core.

## x86-64 without AVX2, one thread

x86-64 builds (default target CPU, no `RUSTFLAGS`) with the SIMD level forced to `sse2`
(`DSD2PCM_SIMD=sse2`), the level picked on x86-64 CPUs without AVX2 such as the Celeron and
Pentium J and N series. scarletbook-pcm 0.7.0 picks its AVX2 path on these test CPUs, so
for this table it was built with its run-time detection patched to return its generic path,
the one it runs on a CPU without AVX2 and FMA. "Before" is dsd2pcm's generic level, which
such CPUs ran until this level existed. 2 channels, 30 s of audio; Intel Core i9-13900H
P-core (CPU 0) and E-core (CPU 12) and AMD Ryzen 9 3950X (CPU 4).

| Rate | Cutoff | `run 4` P-core before | `run 4` P-core | `run 4` E-core before | `run 4` E-core | `run 4` Zen 2 before | `run 4` Zen 2 |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 0.88x | 1.16x | 0.98x | 1.21x | 0.89x | 1.22x |
| 88.2 kHz | Standard | 0.91x | 1.16x | 0.95x | 1.20x | 0.89x | 1.23x |
| 176.4 kHz | Low | 0.98x | 1.12x | 1.07x | 1.15x | 1.02x | 1.29x |
| 176.4 kHz | Standard | 1.00x | 1.12x | 1.06x | 1.17x | 1.00x | 1.26x |
| 176.4 kHz | Wide | 0.86x | 1.13x | 0.85x | 1.09x | 0.84x | 1.19x |
| 352.8 kHz | Low | 1.02x | 1.16x | 1.15x | 1.22x | 1.06x | 1.33x |
| 352.8 kHz | Standard | 1.03x | 1.17x | 1.17x | 1.26x | 1.06x | 1.32x |
| 352.8 kHz | Wide | 1.01x | 1.15x | 1.03x | 1.13x | 0.99x | 1.30x |

One frame per call (`run 1`): 1.03x to 1.14x on the P-core, 1.06x to 1.12x on the E-core,
1.09x to 1.27x on Zen 2. Whole file: 1.10x to 1.16x (P-core), 1.10x to 1.20x (E-core),
1.20x to 1.34x (Zen 2).

## Known gaps

- Whole-file conversion of the 2-channel file at 176.4 kHz `wide` on the P-core reads 0.99x.
  The same filter is ahead converting frame runs (1.04x to 1.10x) and on the 6-channel file
  (1.05x), so the gap is likely in the whole-file path around the filter (DSF reading, WAV
  writing), not in the filter.
