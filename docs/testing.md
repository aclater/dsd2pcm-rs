[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Testing for long-lived processes

# Testing for long-lived processes

A program that keeps converters for days (a file system, a player, a server) needs more than
correct output: memory that levels off, threads that end, no leaks through the C interface
and no undefined behaviour or data races. These checks cover that, on Linux.

| Check | Where | When |
|---|---|---|
| Soak test, short (1000 iterations), and its leak self-test | `crates/dsd2pcm/tests/soak.rs`, `ci.yml` (x86_64, aarch64) | every pull request and push |
| C leak loop under AddressSanitizer, LeakSanitizer and UBSan | `crates/dsd2pcm-capi/tests/leakloop.c`, `ci.yml` (C library) | every pull request and push |
| Miri: the C interface's tests and the core crate's feasible tests | `sanitizers.yml` (miri (capi), miri (core-lib), miri (core-integration)) | every pull request, weekly, by hand |
| ThreadSanitizer: threaded whole-file path, frame runs, soak, C interface | `sanitizers.yml` (tsan) | weekly, by hand |
| Soak test, one hour | by hand, before a release | results below |
| Valgrind memcheck of the command-line tool | by hand, before a release | results below |

## Soak test

`tests/soak.rs` runs thousands of conversions in one process. Each iteration takes the next
setting of a cycle over every rate, cutoff, filter and channel count (1, 2, 5, 6), invalid
combinations included as error paths, and a pseudo-random area of 3 to 14 frames (every
16th iteration 230 frames, over 1 MiB per channel, so the threaded file path reads several
blocks). It converts the area with a `frames::Converter` kept for the whole run and with a
fresh one, in one buffer and frame by frame; as an in-memory DSF or DFF file through
`dsd64_to_pcm`, threaded and on one thread; cancels a conversion from the progress hook;
reads the WAV output and a FLAC file back; feeds the readers truncated files; and every
tenth iteration converts a file on disk with `convert_file_with`. The paths' outputs are
compared with each other.

It judges the memory of a run of 1000 iterations or more. A shorter run, the default of
300 included, prints the figures and says it did not judge them. After a warm-up of two
rounds of the cycle (216 iterations: every kept converter exists and has run twice), the
rest of the run is cut in two windows, the plateau and the late one. It asserts:

- the live heap (bytes allocated and not freed, counted by a global allocator), less what
  the kept converters grew by, rises at most 64 KiB from the plateau's highest value to
  the late window's: the leak measure, whatever the allocator does with freed memory;
- the kept converters (one per valid setting, 72) grow on at most 1 % of their calls in
  the late window, and by at most 4 MiB. Each one's buffers grow while it meets calls
  that need more than it holds, which gets rarer as the run goes on; a leak in a call
  grows on every call;
- on Linux, the median resident set size (`VmRSS`) of the late window is within 25 % or
  64 MiB plus 2 MiB per worker thread, whichever is larger, of the plateau's median. The
  resident set also holds what the allocator keeps of freed memory in the worker threads'
  arenas, so single readings jump by tens of MiB without a leak, which the median does not
  follow, and the medians themselves drift while the arenas fill (by up to 46 MiB with 4
  workers and 49 MiB with 16 in 1000 iterations, below). This is the coarse check, for
  memory the live heap does not count; the live heap is the precise one;
- on Linux, after every threaded conversion the thread count (`/proc/self/task`) is back
  at its value before the run.

A second test, `soak_catches_a_leak`, shows that the checks catch a leak: a run of 1000
iterations that keeps 1 MiB per iteration must fail the live-heap and resident-set checks,
and one that keeps 1 KiB per iteration and 64 bytes per call of a kept converter must fail
the live-heap and kept-converter checks. `--ignored` runs both; `--test-threads=1` runs
the soak test first, as CI does (they never overlap, but the second starts with what the
allocator kept from the first). The one-hour run is the soak test alone (`--exact soak`).

```sh
DSD2PCM_SOAK_ITERATIONS=1000 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture --test-threads=1
DSD2PCM_SOAK_SECONDS=3600 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture --exact soak
```

`DSD2PCM_SOAK_RSS_REPORT_ONLY=1` prints the resident set without the check, for sanitizer
builds, whose shadow memory grows with the run; `DSD2PCM_SOAK_HEAP_REPORT_ONLY=1` does the
same for the live heap and the kept converters.

At 0.2.0 the checks before this one (the live heap's highest value of 20 to 50 % of the
run as the plateau, no separate account of the kept converters) failed every run of 1000
iterations without a leak: the live heap rose 1.1 MiB to the second half, all of it the
kept converters' buffers, which still grow now and then after their first call (about 70
KiB at a time, 10 times in the late window of a run of 1000 iterations). The live heap
less that growth is the same to the byte at every tenth of the run after the first.

At 0.2.0, the CI build (optimisation level 2, debug assertions), 1000 iterations, on a
Ryzen AI Max+ 395 with the process held to 2.5 cores of CPU time on average (its cgroup
frozen whenever it ran ahead), so that the worker count follows the CPUs it may run on
(`taskset`) and not the limit:

| Worker threads | Runs passed | Live heap, late against plateau | Kept converters, late window | Resident set medians, plateau and late (MiB) | Margin to the RSS bound (MiB) |
|---|---|---|---|---|---|
| 4 (`taskset -c 0-3`) | 10 of 10 | +0 bytes in every run (bound 64 KiB) | grew on 10 of 2354 calls, by 633 KiB (bounds 23 calls, 4 MiB) | 177 to 234; 199 to 241 | 26 to 85 |
| 16 (`taskset -c 0-15`) | 2 of 2 (one with an earlier bound of 25 % or 64 MiB) | the same | the same | 222, 271; 270, 294 | 47 and 72 against this bound |
| 32 (all) | 1 of 1, and 1 of 1 in the release build | the same | the same | 273, 317; release 258, 307 | 84; release 79 |

The leak self-test caught every leak in all these runs: with 1 MiB per iteration the live
heap rose 392 MiB and the late median of the resident set was 157 to 247 MiB over its
bound (4 workers); with 1 KiB per iteration and 64 bytes per kept call the live heap rose
392 KiB and the kept converters grew on all 2354 of their late calls.

One hour (the release build, 32 worker threads, the same CPU-time hold): 410 916
iterations, 3 425 255 conversions (301 339 threaded); the live heap less the kept
converters' growth 156 750 KiB at every tenth of the run, to the byte (the test's arrays
of figures are most of it); the kept converters grew on none of their 1 287 080 calls in
the late window; resident set 869 to 948 MiB at the tenths, medians 875 and 914 MiB
(bound 1094); threads 2 before and after.

Before 0.1.2, with the live-heap check, one hour each:

| CPU, worker threads | Iterations | Conversions (threaded) | Live heap | Resident set (MiB, each tenth of the run) | Threads |
|---|---|---|---|---|---|
| AMD Ryzen AI Max+ 395, 16 (`taskset -c 0-15`) | 511 238 | 3 551 370 (312 425) | 119 277 KiB at every tenth of the run, to the byte | 324 316 361 366 380 376 380 380 393 393; second half 353 to 407, median 384 | 2 before, 2 after, and after every threaded run |

Shorter runs on an Intel Core i9-13900H with 4 worker threads agree: 20 minutes (129 548
iterations), the same live heap to the byte, resident set 150 to 190 MiB; and one hour
before the live-heap check (387 206 iterations), resident set 147 to 193 MiB.

The live heap is the 40 converters the test keeps (one per valid setting and gain, their
buffers grown to the longest run they met, 230 frames) and the test's own arrays of
samples (64 MiB, allocated before the run): it stops moving once every kept converter has
met its longest run, and stays there to the byte. The resident set grows with the number of
worker threads (each has its own allocator arena) and wanders within them; the peak (`time
-v`) was 497 MiB with 16 workers and 282 to 286 MiB with 4, during the 230-frame
six-channel iterations.

## C leak loop

`crates/dsd2pcm-capi/tests/leakloop.c` repeats, in one process, everything a long-lived C
program does: for every rate, cutoff and filter (invalid combinations included) a
converter is created, converts in one buffer as packed 24-bit and one buffer per frame as
`int32_t` with neighbours, goes through the error paths (NULL pointers, bad lengths and
formats, overlapping buffers, a too-small `struct_size`) and is freed; then whole-file
conversions, threaded and on one thread, with a progress callback, cancelled before and
after the first block, and refused (missing input, a file that is not DSD, bad options,
output equal to input); and a thread that ends after setting its last error. CI builds it
with clang's AddressSanitizer, LeakSanitizer and UndefinedBehaviorSanitizer and runs 100
iterations with `detect_leaks=1`: one block left allocated fails it. Without a sanitizer it
also checks the resident set levels off. Locally:

```sh
CBINDGEN=cbindgen crates/dsd2pcm-capi/check.sh          # LEAKLOOP_ITERATIONS=100 by default
ASAN_CC=clang .github/scripts/capi-check.sh crates/dsd2pcm-capi
```

## Miri

```sh
rustup toolchain install nightly-2026-11-01 --component miri,rust-src
DSD2PCM_SIMD=generic MIRIFLAGS=-Zmiri-env-forward=DSD2PCM_SIMD cargo +nightly-2026-11-01 miri test -p dsd2pcm-capi
RUSTUP_TOOLCHAIN=nightly-2026-11-01 .github/scripts/miri-core.sh
```

Miri interprets the program and checks every memory access, so it finds undefined
behaviour, use after free, data races between its threads and memory leaked at exit. The
kernels run at the generic level (`DSD2PCM_SIMD=generic`, passed in with
`-Zmiri-env-forward`; Miri runs no SIMD). Tests that use files or convert seconds of audio
are left out, each with its reason: `#[cfg_attr(miri, ignore = "...")]` in the tests,
`--skip` in `.github/scripts/miri-core.sh` for the library's kernel proofs and one
integration test.

`sanitizers.yml` runs both on every pull request, as the jobs `miri (capi)` and
`miri (core)`, side by side, and every week. Each takes under 20 minutes, so both can be
required checks; until 0.2.0 Miri ran on pull requests only when they touched the C
interface, and with 0.2.0's tests the core job hit its 150-minute limit.

At 0.2.0 on nightly-2026-11-01, no findings in the tests run; wall time with the build, on
one core of a Ryzen AI Max+ 395:

| Part | Run | Left out |
|---|---|---|
| C interface (`dsd2pcm-capi`), 11 minutes | 14 tests | 9: 7 use files, 2 convert 40 or 70 frames |
| Library unit tests | 7 tests | 9: 2 use files; the kernel proofs, every plan against the integer reference and pysacd's long renders |
| `containers` (DSF, DFF, WAV readers and writers) | 15 tests | 2: thousands of corrupted files (fuzzed instead); a whole 1 MiB block converted with two filters |
| `flac` | 10 tests, the FLAC files embedded | 2: temporary files; thousands of corrupted files |
| `frames` | 1 test (argument errors) | 6: tens of frames at every setting |
| `identity`, `pysacd`, `progress`, `frames_alloc`, `soak` | not run | seconds of audio in every test |

The core crate's part (`miri-core.sh`) took 14 minutes. Three of the tests left
out ran in CI until 0.2.0 but take over 50 minutes each under Miri:
`decimator::tests::plans_match_reference`, `decimator::tests::precise_plans_match_the_design`
and `output_counts_past_32_bits_do_not_panic`. Under Miri's default floating-point model
the two plan tests also fail; with `-Zmiri-deterministic-floats` the first passes in 68
minutes (the cause is not yet known). All three run natively on every pull request, at
every SIMD level.

## ThreadSanitizer

```sh
export CARGO_PROFILE_DEV_OPT_LEVEL=2
RUSTFLAGS=-Zsanitizer=thread DSD2PCM_SOAK_ITERATIONS=100 \
  cargo +nightly-2026-11-01 test -Zbuild-std --target x86_64-unknown-linux-gnu \
  -p dsd2pcm --test progress --test frames --test soak -- --include-ignored --skip soak_catches_a_leak
RUSTFLAGS=-Zsanitizer=thread \
  cargo +nightly-2026-11-01 test -Zbuild-std --target x86_64-unknown-linux-gnu -p dsd2pcm-capi --lib
```

With 100 iterations the soak test prints its memory figures without judging them (it
judges 1000 or more), so this run looks for races, not leaks; its leak self-test, 1000
iterations each, is left to the plain build.

Standard library and crates are rebuilt instrumented, so the scoped worker threads of the
whole-file path, their atomics and mutexes, and the C interface's busy-handle checks are all
seen. Before 0.1.2 on nightly-2026-11-01: no reports; frames 6 tests, progress 3, the soak
test 100 iterations (728 conversions, 65 threaded), the C interface's 19 tests. At 0.2.0, with the
commands above: no reports; frames 7 tests, progress 3, the soak test 100 iterations, the
C interface's 23 tests.

## Valgrind

The command-line tool, threaded (8 worker threads), on a 20-second excerpt of a stereo
DSD64 DSF file, under `valgrind --tool=memcheck --leak-check=full --show-leak-kinds=all
--errors-for-leak-kinds=definite,indirect --track-origins=yes`. Before 0.1.2, valgrind
3.28.0, AMD Ryzen AI Max+ 395:

| Run | Heap | Definitely lost | Indirectly lost | Possibly lost | Errors |
|---|---|---|---|---|---|
| 176.4 kHz, default filter | 751 allocations, 38.5 MB | 0 | 0 | 64 bytes in 1 block | 0 |
| 88.2 kHz, pysacd filter | 824 allocations, 16.7 MB | 0 | 0 | 64 bytes in 1 block | 0 |
| 352.8 kHz, default filter | 679 allocations, 59.7 MB | 0 | 0 | 64 bytes in 1 block | 0 |

The 64 bytes are the standard library's handle of the main thread (`Thread::new_current`,
created once when the first scoped thread starts), which lives until exit by design; the
rest still reachable at exit (under 200 KB) are the standard library's own buffers.

## Private names

`ci.yml` (job `no private names`, every pull request and push) runs
`.github/scripts/check-private-names.sh`: tracked files and the author and committer of
each new commit must hold no `home.arpa` name, no private (RFC 1918) IPv4 address and no
match of the repository variable `PRIVATE_NAMES`, a regular expression kept outside the
repository; without the variable the built-in patterns still run. It exits 0 when clean, 1
on a hit and 2 when the check itself fails (an invalid or match-everything regex, a git
error), so an error never passes; `.github/scripts/check-private-names-test.sh` (run first
in the same job) checks each of those cases in throwaway repositories. Locally:
`PRIVATE_NAMES='pattern' .github/scripts/check-private-names.sh`.

## See also

- [Development](development.md): the checks CI runs
- [C interface](c-api.md#checks): the C library checks
- [Fuzzing](fuzzing.md): the readers against bad input

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
