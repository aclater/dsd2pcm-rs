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

## Known gaps

- Whole-file conversion of the 2-channel file at 176.4 kHz `wide` on the P-core reads 0.99x.
  The same filter is ahead converting frame runs (1.04x to 1.10x) and on the 6-channel file
  (1.05x), so the gap is likely in the whole-file path around the filter (DSF reading, WAV
  writing), not in the filter.
- Without AVX2 (the generic level, used on 32-bit x86 and older CPUs) dsd2pcm runs at 0.6x
  to 0.9x of scarletbook-pcm 0.7.0. A dedicated SSE2 level is planned.
