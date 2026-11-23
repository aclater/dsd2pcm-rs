# Testing for long-lived processes

A program that keeps converters for days (a file system, a player, a server) needs more than
correct output: memory that levels off, threads that end, no leaks through the C interface
and no undefined behaviour or data races. These checks cover that, on Linux.

| Check | Where | When |
|---|---|---|
| Soak test, short (1000 iterations) | `crates/dsd2pcm/tests/soak.rs`, `ci.yml` (x86_64, aarch64) | every pull request and push |
| C leak loop under AddressSanitizer, LeakSanitizer and UBSan | `crates/dsd2pcm-capi/tests/leakloop.c`, `ci.yml` (C library) | every pull request and push |
| Miri: the C interface's tests and the core crate's feasible tests | `sanitizers.yml` (miri-capi, miri-core) | weekly, by hand, pull requests that touch `crates/dsd2pcm-capi` |
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

It asserts, after a warm-up (the first 20 % of iterations):

- the live heap (bytes allocated and not freed, counted by a global allocator) after each
  iteration of the second half is at most 64 KiB above the highest value of 20 to 50 %: the
  leak measure, whatever the allocator does with freed memory;
- on Linux, the highest resident set size (`VmRSS`) of the second half is within 10 % or
  32 MiB, whichever is larger, of the highest of 20 to 50 %. The resident set also holds
  what the allocator keeps of freed memory over the worker threads' arenas, so it wanders
  by tens of MiB without a leak;
- on Linux, after every threaded conversion the thread count (`/proc/self/task`) is back
  at its value before the run.

```
DSD2PCM_SOAK_ITERATIONS=1000 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture
DSD2PCM_SOAK_SECONDS=3600 cargo test --release -p dsd2pcm --test soak -- --ignored --nocapture
```

`DSD2PCM_SOAK_RSS_REPORT_ONLY=1` prints the memory figures without the check, for sanitizer
builds, whose shadow memory grows with the run.

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

```
CBINDGEN=cbindgen crates/dsd2pcm-capi/check.sh          # LEAKLOOP_ITERATIONS=100 by default
ASAN_CC=clang .github/scripts/capi-check.sh crates/dsd2pcm-capi
```

## Miri

```
rustup toolchain install nightly-2026-11-01 --component miri,rust-src
cargo +nightly-2026-11-01 miri test -p dsd2pcm-capi
.github/scripts/miri-core.sh            # with RUSTUP_TOOLCHAIN=nightly-2026-11-01
```

Miri interprets the program and checks every memory access, so it finds undefined
behaviour, use after free, data races between its threads and memory leaked at exit. The
kernels run at the generic level (`DSD2PCM_SIMD=generic`, passed in with
`-Zmiri-env-forward`; Miri runs no SIMD). Tests that use files or convert seconds of audio
are left out, each with its reason: `#[cfg_attr(miri, ignore = "...")]` in the tests,
`--skip` in `.github/scripts/miri-core.sh` for the library's kernel proofs.

Before 0.1.2 on nightly-2026-11-01, no findings; about 10 minutes for the C interface
and 12 for the core crate on a Ryzen AI Max+ 395 (one core each):

| Part | Run | Left out |
|---|---|---|
| C interface (`dsd2pcm-capi`) | 11 tests | 8: 6 use files, 2 convert 40 or 70 frames |
| Library unit tests | 6 tests | 5: the kernel proofs and pysacd's long renders |
| `containers` (DSF, DFF, WAV readers and writers) | 14 tests | 1: thousands of corrupted files (fuzzed instead) |
| `flac` | 9 tests, the FLAC files embedded | 2: temporary files; thousands of corrupted files |
| `frames` | 1 test (argument errors) | 5: tens of frames at every setting |
| `identity`, `pysacd`, `progress`, `frames_alloc`, `soak` | not run | seconds of audio in every test |

## ThreadSanitizer

```
RUSTFLAGS=-Zsanitizer=thread DSD2PCM_SOAK_ITERATIONS=100 DSD2PCM_SOAK_RSS_REPORT_ONLY=1 \
  cargo +nightly-2026-11-01 test -Zbuild-std --target x86_64-unknown-linux-gnu \
  -p dsd2pcm --test progress --test frames --test soak -- --include-ignored
RUSTFLAGS=-Zsanitizer=thread \
  cargo +nightly-2026-11-01 test -Zbuild-std --target x86_64-unknown-linux-gnu -p dsd2pcm-capi --lib
```

Standard library and crates are rebuilt instrumented, so the scoped worker threads of the
whole-file path, their atomics and mutexes, and the C interface's busy-handle checks are all
seen. Before 0.1.2 on nightly-2026-11-01: no reports; frames 6 tests, progress 3, the soak
test 100 iterations (728 conversions, 65 threaded), the C interface's 19 tests.

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
