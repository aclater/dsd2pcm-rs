[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Performance

# Performance

Speed of DSD64 to 24-bit PCM with the `precise` filter (the default from 0.2.0) and the
`fast` filter (scarletbook-pcm 0.7.0's, the default up to 0.1.2), against scarletbook-pcm
0.7.0. `precise` trades CPU time for quality: it takes 1.3 to 1.9 times scarletbook-pcm's
time. `fast` gives scarletbook-pcm's output byte for byte and is faster than it on x86-64.

## precise and fast against scarletbook-pcm 0.7.0

Measured 2026-11-24 on pre-release builds of 0.2.0: `fast` with the 0.2.0 kernels, and
`precise` from the build that added it, which does not yet have the 0.2.0 kernel changes
(its `fast` path ran at 0.1.2's speed there). Intel Core i9-13900H, Fedora, `powersave`
governor, turbo on; built for the x86-64 baseline (`-C target-cpu=x86-64`, as the release
builds), AVX2 kernels picked at run time. Inputs: generated music-like DSD64 files
(`.github/scripts/make-test-dsf.py --music 30 2` and `--music 10 6`), converted in memory;
one warm-up and five timed runs per cell, median reported. Every `fast` output had the same
hash as scarletbook-pcm 0.7.0's. "Frames" is runs of four SACD frames with their
neighbours, as a SACD file system reads them; "file" is whole-file conversion.

Speed relative to scarletbook-pcm 0.7.0 (above 1.00x is faster), and `precise` in multiples
of real time:

| Setting | Cores | 2 ch `fast` | 2 ch `precise` | 2 ch `precise`, × real time | 6 ch `fast` | 6 ch `precise` | 6 ch `precise`, × real time |
|---|---|---:|---:|---:|---:|---:|---:|
| Frames, 88.2 kHz, 40 kHz | P-core, 1 thread | 1.11x | 0.53x | 183 | 1.12x | 0.54x | 60 |
| Frames, 176.4 kHz, 40 kHz | P-core, 1 thread | 1.15x | 0.63x | 159 | 1.14x | 0.64x | 52 |
| Frames, 352.8 kHz, wide | P-core, 1 thread | 1.19x | 0.74x | 118 | 1.19x | 0.75x | 38 |
| File, 176.4 kHz, 40 kHz | P-core, 1 thread | 1.16x | 0.64x | 162 | 1.16x | 0.65x | 54 |
| Frames, 88.2 kHz, 40 kHz | E-core, 1 thread | 1.18x | 0.52x | 71 | 1.21x | 0.54x | 24 |
| Frames, 176.4 kHz, 40 kHz | E-core, 1 thread | 1.19x | 0.64x | 60 | 1.21x | 0.65x | 20 |
| Frames, 352.8 kHz, wide | E-core, 1 thread | 1.18x | 0.71x | 42 | 1.19x | 0.72x | 14 |
| File, 176.4 kHz, 40 kHz | E-core, 1 thread | 1.17x | 0.64x | 62 | 1.20x | 0.65x | 21 |
| Frames, 88.2 kHz, 40 kHz | 12 P-core threads | 1.22x | 0.55x | 1217 | 1.21x | 0.56x | – |
| Frames, 176.4 kHz, 40 kHz | 12 P-core threads | 1.20x | 0.64x | 1052 | 1.22x | 0.65x | – |
| Frames, 352.8 kHz, wide | 12 P-core threads | 1.22x | 0.75x | 779 | 1.25x | 0.77x | – |
| Frames, 88.2 kHz, 40 kHz | 8 E-core threads | 1.17x | 0.53x | 544 | 1.19x | 0.54x | – |
| Frames, 176.4 kHz, 40 kHz | 8 E-core threads | 1.16x | 0.62x | 460 | 1.16x | 0.62x | – |
| Frames, 352.8 kHz, wide | 8 E-core threads | 1.11x | 0.67x | 315 | 1.13x | 0.68x | – |

- `precise` takes 1.80 to 1.91 times scarletbook-pcm's time at 88.2 kHz, 1.53 to 1.62 times
  at 176.4 kHz and 1.30 to 1.50 times at 352.8 kHz `wide`. Against `fast` in the same build
  it costs 1.55 to 2.17 times the time on these settings (1.28 to 1.41 times whole-file on
  all cores, where scarletbook-pcm, which converts a whole file on one thread, has no row).
- `fast` is 1.11 to 1.25 times scarletbook-pcm's speed on every setting here.
- The spread of the five runs in a cell was 1.5 % or less in most cells and 3.9 % at most;
  each cell is one process, so treat differences under about 2 % as level.

## Weakest realistic CPU

<!-- TODO before the 0.2.0 tag: fill from a timing run on the Intel Core i9-13900H
(x86-64 baseline build, one E-core): precise against fast, stereo and 5.1, 88.2 and
176.4 kHz, with the method of the section above. -->

Weakest realistic CPU (x86-64 baseline, single efficiency core): `precise` against `fast`,
stereo and 5.1, 88.2 and 176.4 kHz. To be measured before the 0.2.0 release.

## fast against scarletbook-pcm 0.7.0 (0.1.x measurements)

The sections below measure the `fast` filter (the default up to 0.1.2), against
scarletbook-pcm 0.7.0, whose output it matches byte for byte. They were taken on 0.1.x
releases; the 0.2.0 kernels measured as fast or faster on the same settings
([above](#precise-and-fast-against-scarletbook-pcm-070)). Numbers are multiples of real
time (higher is faster), best of three runs over 60 s of audio, measured in memory.

- CPUs: Intel Core i9-13900H (P-core: CPU 0; E-core: CPU 12; `powersave` governor) and
  AMD Ryzen 9 3950X (Zen 2, CPU 4, `performance` governor), Fedora, one benchmark at a time.
- Build: release, SIMD level picked at run time (AVX2 with FMA on both CPUs).
- Inputs: a 2-channel and a 6-channel DSF from real SACDs.
- Tools: `cargo run --release --example bench -- FILE.dsf 60` (whole file, DSF to WAV) and
  `cargo run --release --example sacdfs_path -- FILE.dsf 60 all` (runs of SACD frames, as a
  SACD file system reads them).
- Other platforms: `.github/workflows/bench.yml`, run by hand, runs both examples on
  GitHub-hosted runners (Linux aarch64, macOS on Apple silicon, Linux x86_64) on generated
  2- and 6-channel test files, in rounds with the crate order alternating, and writes the
  median ratios to the job summary. Shared runners: those numbers are indicative. Locally:
  `cargo build --release --locked -p dsd2pcm --examples`, then
  `.github/scripts/bench-rounds.sh target/bench/logs` and
  `python3 -I .github/scripts/bench-summary.py target/bench/logs local`.

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

## aarch64 (GitHub-hosted runners, indicative)

From `.github/workflows/bench.yml` on dsd2pcm 0.1.1, with generated 60 s test files (not SACD
rips), one thread, median ratio of dsd2pcm's speed to scarletbook-pcm 0.7.0's over the
rounds (lowest to highest in brackets). Linux: `ubuntu-24.04-arm` (Neoverse N2, 4 rounds, the
round-to-round spread under 0.02). macOS: `macos-15` (Apple M1, virtual, 5 rounds; shared
and noisy, spreads up to about ±0.2).

| Rate | Cutoff | Linux `run 4` 2 ch / 6 ch | Linux whole file 2 ch / 6 ch | macOS `run 4` 2 ch / 6 ch | macOS whole file 2 ch / 6 ch |
|---|---|---|---|---|---|
| 88.2 kHz | Low | 1.02x / 1.02x | 1.02x / 1.02x | 1.11x / 1.08x | 1.06x / 0.96x |
| 88.2 kHz | Standard | 1.02x / 1.02x | 1.01x / 1.02x | 1.12x / 1.05x | 1.05x / 1.02x |
| 176.4 kHz | Low | 1.00x / 1.00x | 0.97x / 0.99x | 1.08x / 0.95x | 1.05x / 1.04x |
| 176.4 kHz | Standard | 1.00x / 1.00x | 0.97x / 0.99x | 0.96x / 1.03x | 1.07x / 1.03x |
| 176.4 kHz | Wide | 1.00x / 1.00x | 0.98x / 0.99x | 1.05x / 0.99x | 0.91x / 1.09x |
| 352.8 kHz | Low | 1.03x / 1.03x | 1.01x / 1.01x | 1.13x / 1.06x | 1.03x / 1.09x |
| 352.8 kHz | Standard | 1.04x / 1.04x | 1.02x / 1.01x | 1.11x / 1.12x | 1.12x / 1.09x |
| 352.8 kHz | Wide | 0.98x / 0.98x | 0.97x / 0.97x | 0.98x / 1.01x | 1.04x / 0.94x |

On aarch64 dsd2pcm is level with scarletbook-pcm 0.7.0 (0.97x to 1.04x on Linux, every mode,
`run 1` included); the x86-64 lead comes from the AVX2 kernels. Every run checked that both
crates give the same bytes on both platforms.

## precise against fast, by rate and cutoff

`--filter precise` against `--filter fast` (not against scarletbook-pcm), whole file,
2 channels, one thread, best of three over 20 s of audio
(`cargo run --release --example bench_filters -- FILE.dsf 20`, pinned with `taskset`).
Measured on the build that added `precise` (0.1.2's kernels). Multiples of real time, and
`precise`'s time over `fast`'s:

| Rate | Cutoff | P-core | E-core | Zen 2 | i686 on the P-core, `sse2` level |
|---|---|---:|---:|---:|---:|
| 88.2 kHz | 30 | 170 (2.14x) | 63 (2.25x) | 163 (2.15x) | 82 (2.12x) |
| 88.2 kHz | 40 | 196 (2.06x) | 75 (2.18x) | 191 (2.05x) | 97 (2.07x) |
| 176.4 kHz | 30 | 149 (1.83x) | 53 (1.86x) | 142 (1.81x) | 71 (1.82x) |
| 176.4 kHz | 40 | 171 (1.82x) | 64 (1.82x) | 166 (1.79x) | 85 (1.77x) |
| 176.4 kHz | wide | 95 (2.06x) | 32 (2.28x) | 93 (2.12x) | 40 (2.16x) |
| 352.8 kHz | 30 | 115 (1.79x) | 42 (1.84x) | 107 (1.75x) | 58 (1.84x) |
| 352.8 kHz | 40 | 128 (1.78x) | 48 (1.82x) | 119 (1.73x) | 67 (1.81x) |
| 352.8 kHz | wide | 116 (1.56x) | 43 (1.61x) | 111 (1.50x) | 55 (1.67x) |

On all cores (120 s of audio, 2 channels) `precise` takes 1.18x to 2.00x `fast`'s time
on the i9-13900H (20 threads) and 1.15x to 1.36x on the Zen 2 (32 threads), converting at
227 to 449 and 365 to 804 times real time. With two channels the threaded `fast` filter is partly
limited by work outside the filter, so these ratios vary more from run to run than the
one-thread ones.

The cost follows the taps per output: the first stage reads a 12-byte window (16 bytes for
`wide`) instead of 8 (12), and every later stage is two filters of about 80 dB each instead
of one of 110 dB ([design](design.md#the-precise-filter-filterprecise)). The output is the
same at every SIMD level, as for `fast`.

## Known gaps

- Whole-file conversion of the 2-channel file at 176.4 kHz `wide` on the P-core reads 0.99x.
  The same filter is ahead converting frame runs (1.04x to 1.10x) and on the 6-channel file
  (1.05x), so the gap is likely in the whole-file path around the filter (DSF reading, WAV
  writing), not in the filter.
- aarch64 (above): level with scarletbook-pcm 0.7.0, 2 to 3 % behind at 352.8 kHz `wide`
  and in whole-file conversion at 176.4 kHz.

## See also

- [Design](design.md#speed): why the kernels are fast
- [PCM quality](pcm-quality.md#the-precise-filter): what `precise` buys for its CPU time
- [Development](development.md): building and testing
- [Changelog](../CHANGELOG.md): the changes behind each release

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
