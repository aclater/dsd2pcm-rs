# Performance

Speed of DSD64 to 24-bit PCM with the default filter, against scarletbook-pcm 0.7.0, whose
output it matches byte for byte. Numbers are multiples of real time (higher is faster), best
of three runs over 60 s of audio, measured in memory.

- Host: Intel Core i9-13900H (P-core: CPU 0; E-core: CPU 12), Fedora, `powersave` governor,
  one benchmark at a time.
- Build: release, SIMD level picked at run time (AVX2 with FMA on this CPU).
- Inputs: a 2-channel and a 6-channel DSF from real SACDs.
- Tools: `cargo run --release --example bench -- FILE.dsf 60` (whole file, DSF to WAV) and
  `cargo run --release --example sacdfs_path -- FILE.dsf 60 all` (runs of SACD frames, as a
  SACD file system reads them).

## Frame runs, one thread (`sacdfs_path`, mode `run 4`)

| Rate | Cutoff | 2 ch P-core | 2 ch E-core | 6 ch P-core | 6 ch E-core |
|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.08x | 1.11x | 1.10x | 1.13x |
| 88.2 kHz | Standard | 1.11x | 1.12x | 1.11x | 1.15x |
| 176.4 kHz | Low | 1.13x | 1.14x | 1.14x | 1.16x |
| 176.4 kHz | Standard | 1.13x | 1.13x | 1.14x | 1.15x |
| 176.4 kHz | Wide | 1.03x | 1.07x | 1.05x | 1.08x |
| 352.8 kHz | Low | 1.22x | 1.17x | 1.20x | 1.26x |
| 352.8 kHz | Standard | 1.21x | 1.16x | 1.22x | 1.18x |
| 352.8 kHz | Wide | 1.17x | 1.15x | 1.18x | 1.16x |

Ratio of dsd2pcm's speed to scarletbook-pcm 0.7.0's; below 1.00x is slower.

## Whole file, 2 channels, P-core

| Rate | Cutoff | scarletbook-pcm 0.7.0 | dsd2pcm, 1 thread |
|---|---|---|---|
| 88.2 kHz | Low | 312 | 365 |
| 88.2 kHz | Standard | 358 | 411 |
| 176.4 kHz | Low | 235 | 258 |
| 176.4 kHz | Standard | 271 | 292 |
| 176.4 kHz | Wide | 185 | 184 |
| 352.8 kHz | Low | 173 | 210 |
| 352.8 kHz | Standard | 194 | 235 |
| 352.8 kHz | Wide | 160 | 183 |

scarletbook-pcm converts on one thread; dsd2pcm uses all cores unless told otherwise.

## Known gaps

- Frame runs of one frame at 88.2 kHz (mode `run 1`) and 176.4 kHz `wide` whole-file
  conversion are level with scarletbook-pcm 0.7.0 (0.98x to 1.04x, within run-to-run
  noise).
- Without AVX2 (the generic level, used on 32-bit x86 and older CPUs) dsd2pcm runs at 0.6x
  to 0.9x of scarletbook-pcm 0.7.0. A dedicated SSE2 level is planned.
