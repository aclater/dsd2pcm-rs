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
| 88.2 kHz | Low | 1.02x | 1.01x | 1.04x | 1.03x |
| 88.2 kHz | Standard | 1.03x | 1.01x | 1.05x | 1.03x |
| 176.4 kHz | Low | 1.02x | 1.00x | 1.03x | 1.02x |
| 176.4 kHz | Standard | 1.03x | 1.01x | 1.04x | 1.02x |
| 176.4 kHz | Wide | 1.03x | 0.99x | 1.04x | 1.00x |
| 352.8 kHz | Low | 1.07x | 1.05x | 1.07x | 1.13x |
| 352.8 kHz | Standard | 1.08x | 1.05x | 1.09x | 1.07x |
| 352.8 kHz | Wide | 1.02x | 0.99x | 1.03x | 1.00x |

Ratio of dsd2pcm's speed to scarletbook-pcm 0.7.0's; below 1.00x is slower.

## Whole file, 2 channels, P-core

| Rate | Cutoff | scarletbook-pcm 0.7.0 | dsd2pcm, 1 thread | dsd2pcm, 20 threads |
|---|---|---|---|---|
| 88.2 kHz | Low | 316 | 342 | 1025 |
| 88.2 kHz | Standard | 361 | 382 | 1094 |
| 176.4 kHz | Low | 236 | 231 | 525 |
| 176.4 kHz | Standard | 273 | 265 | 494 |
| 176.4 kHz | Wide | 186 | 183 | 614 |
| 352.8 kHz | Low | 175 | 185 | 301 |
| 352.8 kHz | Standard | 196 | 206 | 336 |
| 352.8 kHz | Wide | 164 | 164 | 294 |

scarletbook-pcm converts on one thread; dsd2pcm uses all cores unless told otherwise.

## Known gaps

- On one thread, 2-channel 176.4 kHz whole-file conversion is 2 to 3 % slower than
  scarletbook-pcm 0.7.0, and the `wide` cutoff on an E-core is up to 2 % slower (frame runs of one frame).
- Without AVX2 (the generic level, used on 32-bit x86 and older CPUs) dsd2pcm runs at 0.6x
  to 0.9x of scarletbook-pcm 0.7.0. A dedicated SSE2 level is planned.
