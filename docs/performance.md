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
  SACD file system reads them). The Zen 2 rows use 30 s of audio.

## Frame runs, one thread (`sacdfs_path`, mode `run 4`)

| Rate | Cutoff | 2 ch P-core | 2 ch E-core | 2 ch Zen 2 | 6 ch P-core | 6 ch E-core | 6 ch Zen 2 |
|---|---|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.09x | 1.16x | 1.01x | 1.10x | 1.19x | 1.07x |
| 88.2 kHz | Standard | 1.12x | 1.20x | 1.01x | 1.12x | 1.21x | 1.07x |
| 176.4 kHz | Low | 1.14x | 1.21x | 1.07x | 1.15x | 1.22x | 1.11x |
| 176.4 kHz | Standard | 1.13x | 1.21x | 1.04x | 1.15x | 1.23x | 1.09x |
| 176.4 kHz | Wide | 1.04x | 1.07x | 1.00x | 1.04x | 1.08x | 1.03x |
| 352.8 kHz | Low | 1.23x | 1.29x | 1.16x | 1.24x | 1.32x | 1.20x |
| 352.8 kHz | Standard | 1.24x | 1.29x | 1.15x | 1.24x | 1.22x | 1.19x |
| 352.8 kHz | Wide | 1.18x | 1.15x | 1.08x | 1.19x | 1.16x | 1.11x |

Ratio of dsd2pcm's speed to scarletbook-pcm 0.7.0's; below 1.00x is slower.

## Whole file, 2 channels, one thread

| Rate | Cutoff | P-core: scarletbook-pcm 0.7.0 | P-core: dsd2pcm | Zen 2: scarletbook-pcm 0.7.0 | Zen 2: dsd2pcm |
|---|---|---|---|---|---|
| 88.2 kHz | Low | 316 | 364 | 292 | 322 |
| 88.2 kHz | Standard | 360 | 407 | 338 | 377 |
| 176.4 kHz | Low | 236 | 254 | 216 | 253 |
| 176.4 kHz | Standard | 272 | 289 | 260 | 285 |
| 176.4 kHz | Wide | 185 | 183 | 176 | 184 |
| 352.8 kHz | Low | 173 | 205 | 160 | 188 |
| 352.8 kHz | Standard | 194 | 228 | 177 | 207 |
| 352.8 kHz | Wide | 161 | 179 | 148 | 161 |

The P-core 176.4 kHz `wide` row reads 0.99x in this 8-row run but 1.02x to 1.03x when run on
its own (three rounds); the `powersave` governor makes later rows run cooler or warmer.
scarletbook-pcm converts on one thread; dsd2pcm uses all cores unless told otherwise.

## Known gaps

- On Zen 2, frame runs of one frame per call (mode `run 1`) with 2 channels are slower than
  scarletbook-pcm 0.7.0 at 88.2 kHz (0.91x to 0.97x) and level at 176.4 kHz `standard` and
  `wide` (0.98x); per-call cost on short passes. 6 channels and longer runs are ahead.
- Without AVX2 (the generic level, used on 32-bit x86 and older CPUs) dsd2pcm runs at 0.6x
  to 0.9x of scarletbook-pcm 0.7.0. A dedicated SSE2 level is planned.
