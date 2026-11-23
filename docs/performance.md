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

## Known gaps

- Whole-file conversion of the 2-channel file at 176.4 kHz `wide` on the P-core reads 0.99x.
  The same filter is ahead converting frame runs (1.04x to 1.10x) and on the 6-channel file
  (1.05x), so the gap is likely in the whole-file path around the filter (DSF reading, WAV
  writing), not in the filter.
- On x86-64 without AVX2 (the generic level, older CPUs) dsd2pcm runs at 0.6x to 0.9x of
  scarletbook-pcm 0.7.0; 32-bit x86 has its own `sse2` level (above).
