# Design

## Data flow

Readers deliver DSD as one byte vector per channel, earliest sample in bit 7, 1 = +1
(DSDIFF order; the DSF reader and writer reverse the bits of each byte). PCM readers deliver
`f64` per channel with full scale at +-1.0.

## FLAC input (`flac.rs`, feature `flac`)

`FlacReader` parses the container itself: leading ID3v2 tags are skipped (at most 16; sizes
must be syncsafe and inside the file), then `fLaC` and the metadata blocks. STREAMINFO must
come first and be 34 bytes; every other block is skipped by seeking after its length is
checked against the file length, so nothing in it is allocated or parsed. `claxon` (0.4.3,
Apache-2.0) is handed a minimal stream of `fLaC`, STREAMINFO marked last, and the file from
the first audio frame, so it only decodes frames; its metadata parsers (Vorbis comment and
application blocks, the code with its `set_len` on uninitialised buffers) never run.
claxon's byte reader indexes its buffer unchecked with the count `read` returns, so every
read from the caller's reader passes through an adapter that caps that count at the buffer
length; an over-reporting `Read` gives errors or wrong samples, never out-of-bounds access
(`lying_reader_is_safe`, also run under Miri). claxon is pinned to `=0.4.3`.

STREAMINFO must declare 1 to 6 channels and 8, 12, 16, 20 or 24 bits (claxon does not
decode 32-bit frames: `Unsupported`). Each decoded block is checked against STREAMINFO: same
channel count, every sample inside the declared bit depth. A `b`-bit sample `v` reads as
`v * 2^-(b-1)`, the WAV reader's scaling, so 16 and 24-bit FLAC and WAV of the same
samples give bit-identical `f64` (`tests/flac.rs`). Memory: one decoded block (at most
65 535 frames by 6 channels of `i32`) plus the caller's output; `read_frames` returns at
most 2^20 frames per call.

Length: STREAMINFO's total. A total of 0 (unknown, as written by streaming encoders) is the
FLAC form of WAV's read-to-end rule: the reader decodes the stream once to count the frames,
then restarts at the first frame; a stream without frames is an error. If that pass fails
and the file ends in an ID3v1 tag (`TAG` and 125 bytes), the frames are counted again
without those 128 bytes. A stream that ends
before its declared length is an error; frames after it are ignored. Frame CRCs are checked
(claxon skips the check only in `cfg(fuzzing)` builds).

## DSD64 to PCM

The decimator follows scarletbook-pcm 0.7.0 exactly (see PROVENANCE.md):

1. S1: a symmetric FIR at 2.8224 MHz evaluated with one 256-entry table per input byte,
   output at 352.8 kHz, centred on the first sample of each byte.
2. Then, by rate and cutoff: a half-band decimator to 176.4 kHz, a sharp FIR (30 or 40 kHz),
   a second decimation to 88.2 kHz or a half-band interpolation back to 352.8 kHz; `wide`
   uses its own S1 and one FIR.

Samples between stages are integers on a 2^26 grid, coefficients are integers, and every
sum is exact in `f64`; each stage rounds half to even. Output sample `n` is centred on DSD
sample `d * n` (`d` = 32, 16 or 8), so there is no delay. The input is taken to continue
with the idle pattern 0x69 before its start and after its end.

The streaming driver feeds any number of bytes through the stages, keeping each stage's
history; it starts 4096 idle bytes before the input and flushes with idle bytes, so each
output sample is the same function of the input as in scarletbook-pcm's whole-area
conversion. Output length: `ceil(samples / d)` per channel.

## The precise filter (`Filter::Precise`)

An opt-in filter on the same decimator, kernels and exactness proof as the default: integer
taps, exact integer sums in `f64`, half-to-even rounding at each stage end, the same
quantiser, the same centring (no delay) and linear phase. Only the tables
(`decimator/tables_precise.rs`) and the stage plan differ. Its aim is a stopband of 150 dB
wherever ultrasonic content folds into the passband; the default's is about 110 dB, which
with a very quiet source sets an audio-band floor near -128 dBFS
([PCM quality](pcm-quality.md#the-precise-filter)).

The taps come from `tools/filters/design_precise.py` (NumPy, hash-pinned in
`tools/filters/requirements.txt`), which writes the tables deterministically and checks the
committed ones (`--check`, run in CI on x86-64; `--report` prints every stage's response
and the largest partial sum of every plan).

- **Why cascades.** The exactness proof needs every partial sum below 2^53. With samples on
  the 2^26 grid, a stage's taps can carry about 52 - 26 - log2(sum of the gains) bits, about
  22 to 24. One 150 dB sharp filter needs about 28-bit taps (in the design trials, rounding
  its taps to 24 to 27 bits left 120 to 144 dB), so it cannot be exact. Two filters of about 80 dB in a
  row, each with 21 or 22-bit taps, give the sum of their stopbands in dB: 160 dB and more,
  with the same exact arithmetic.
- **S1** is the one stage that cannot be split: it decimates the bit stream, so it must
  remove everything that folds into the passband by itself. Its sums are taken from tables
  in 32-bit integers, which leaves room for 2^29-scaled taps (sum of absolute taps below
  2^31). It is a multiband design: 150 dB and more only in the bands that fold onto what the
  later stages keep (k x 352.8 kHz +- 48 kHz for the 30 and 40 kHz filters, +- 96 kHz for
  `wide`), don't-care between them, where the later stages' own stopbands remove what S1
  passes. 95 taps (a 12-byte window) for 30 and 40 kHz, 127 taps (16 bytes) for `wide`;
  the kernels gained the 16-byte window as a constant size. Like the default S1 it has the
  factor `(1 + z^-1)^2 (1 + z^-4)`, so the idle pattern converts to exact zero.
- **P**, a short low-pass at 352.8 kHz (25 taps), takes S1's 2^29 grid back to 2^26 and adds
  its stopband in front of the half-band (128.4 to 176.4 kHz, which folds onto 0 to 48 kHz).
  At 352.8 kHz output it runs again after the interpolation, against its images.
- **Plans** (each stage rounds to the 2^26 grid):

| Cutoff | Rate | Stages after S1 |
|---|---|---|
| 30, 40 | 88.2 kHz | P, half-band /2, F, F /2 |
| 30, 40 | 176.4 kHz | P, half-band /2, F, F |
| 30, 40 | 352.8 kHz | P, half-band /2, F, F, half-band x2, P |
| wide | 176.4 kHz | F80, F80 /2 |
| wide | 352.8 kHz | F40, F40 (at 352.8 kHz: 80 kHz passband, stop from 96 kHz) |

  F is F40 (111 taps) or F30 (149 taps) at 176.4 kHz; F80 has 217 taps. A stage after an
  interpolation by 2 has an even half length, so that its inputs start at an even index.
- **Taps.** Each stage is designed by the Remez exchange (a NumPy port of scarletbook-pcm
  0.7.0's `design.rs`, see PROVENANCE.md), rounded to integers summing exactly to 2^q, then
  improved by a greedy integer search that moves one tap pair at a time while the largest
  stopband error falls and the passband stays within its ripple. Every stage's passband
  ripple is at most 1e-4 (linear). The composite's largest deviation from its DC gain to
  20 kHz is ±0.0019 dB (±0.0022 dB at 352.8 kHz, ±0.0016 dB `wide`); the default's is
  ±0.0037 to ±0.0040 dB.
- **Proof and tests.** `exact.rs` proves every plan as for the default, so FMA and the lanes
  stay exact. The proof's bound is a worst case: every input at its own bound with signs
  matched to the taps. Its largest partial sums are 2^52.95 (352.8 kHz with the 30 kHz
  filter, the final P), 2^52.59 (the same at 40 kHz) and 2^52.49 (`wide` at 352.8 kHz, the
  first F40), under the 2^53 limit by 0.05 to 0.5 bit; a table change that crosses it fails
  `sums_are_exact`. The sums any bit stream can actually reach, from the composite taps,
  are about 3 bits lower (2^49.6 at most for the stages up to the sharp filters);
  `plans_match_reference` checks every plan of both filters, at every SIMD level, against a
  direct computation of its definition in 128-bit integers; `precise_plans_match_the_design`
  writes the precise plans out as in the table above (taps by table, scale, order), checks
  the plans against them and runs the literal plans through that computation against the
  ordered and the lanes paths; `lanes_match_ordered` and the
  S1 table tests cover the new tables; golden hashes pin the output at every level.
- **Cost.** More taps per output than the default: see [performance](performance.md) and
  [PCM quality](pcm-quality.md#the-precise-filter) for the measured CPU time.

## pysacd's filter (`Filter::Pysacd`)

`filters/pysacd.rs` ports pysacd's filter (`csrc/dsp.c`, MIT OR Apache-2.0) and the
quantiser of its `process_batch()`; see PROVENANCE.md. The reference is pysacd's `dsp.c`
compiled with `-O2 -ffp-contract=off`, no SIMD dispatch (pysacd's `ref` build of
`tools/pcm_vectors`): our bytes equal it, not pysacd's wheels (which use `-ffast-math`, FMA
contraction and per-CPU clones; up to 7 LSB apart on its test vectors).

- Stages, all `f32`: S1 (2.8224 MHz to 352.8 kHz) sums 12 byte tables in order (the
  technique of Sebastian Gesemann's dsd2pcm); S2 (95 taps, /2) and, at 88.2 kHz, S3 (161
  taps, /2). Each output is `((0 + h0 x0) + h1 x1) + ...` with every product rounded
  before it is added, as the C loop. The kernels compute 32 independent outputs at a time
  (one per vector lane, through even and odd halves of the input) and never reassociate a
  sum or fuse a multiply-add, so the generic, AVX2 and AVX2+FMA levels give the same bits.
- Taps: `pysacd_taps.rs` holds the `double` S1 taps and the `float` S2 and S3 taps as bit
  patterns, generated from the C code by `tests/pysacd-ref/gen.sh`; the byte tables are
  built from the S1 taps in `f64` in the C order (a test checks their hash).
- Stream: each area (each file) is one stream of the C filter started at its first byte:
  S1 sees DSD silence (0x69) before the start, S2 and S3 see zeros before their first
  input, and DSD silence follows the end. The first `delay` outputs (52 at 88.2 kHz, 26 at
  176.4 kHz) are dropped and the length is the default path's (`ceil(samples / d)`), so
  output `n` is a function of its position and the bytes around it: pieces of work and
  frame runs can be cut anywhere. A frame run with `before: None` starts the area.
- Quantiser: `scale = 8388607 x 10^(gain/20)` in `f32` (the factor from `db_to_linear`,
  equal to pysacd's `(float)pow(...)` at every 0.01 dB step from -120 to +24 dB, checked);
  `v = round_half_even(x scale + (a - b) 2^-32)` with `a` and `b` two xorshift32 draws as
  `f32` (`a` drawn first), clamped to 24 bits. Draws go sample by sample, channel by channel
  within a sample. The generator is seeded at the start of each frame of 4704 bytes per
  channel from the frame's index `tc` in the area: `0x9E3779B9 ^ (tc x 2654435761)`, 32-bit
  wrapping. Files count frames from their start; frame runs from `Run::at_frame` (default
  0). Seeding per frame (pysacd seeds per batch of frames) makes the bytes independent of
  how the area is split.
- Whole-file conversion cuts the output into pieces of 2 frames, all channels each, on all
  cores; frame runs convert blocks of 4 frames on the calling thread.
- Tests (`tests/pysacd.rs`, unit tests in `filters/pysacd.rs`): SHA-256 of the C
  reference's output (`tests/pysacd-ref/sha256.txt`, from `gen.sh`) for silence, all-ones,
  synthetic music and random input, 2 and 6 channels, both rates, 0, -3 and +6 dB, and
  frame indices 1000 and 2^32 - 3; the `f32` outputs before quantising too. Each at every
  SIMD level, through the file path (one and many threads), whole runs and split runs.

## Frame runs (`frames`)

`frames::Converter::convert` takes a run of Super Audio CD frames (one buffer, or one
buffer per frame), the frame before and after it (`None` at an area edge: idle), and writes
interleaved `i32` or 3-byte little-endian samples with per-frame and per-channel clip
counts; types and semantics follow scarletbook-pcm 0.7.0's public API so that a caller can
switch with a mechanical diff. The run is placed on a per-channel timeline (byte 0 = first
byte of the run; neighbours at -4704 and after the last frame; idle beyond) and converted
in blocks of 6 frames: each block's span of bytes, padding included (`Plan::span`), is
de-interleaved per channel into a reused buffer, run through the same plan and kernels as
whole-file conversion (one pass per channel; single-threaded), quantised with the clip
count split at frame boundaries, and packed straight into the caller's buffer. Bytes
outside each output's window only reach padded outputs that are discarded, so the output
equals scarletbook-pcm's and `dsd64_to_pcm`'s. Buffers are bounded by the block size, not
the run length; after the first call nothing is allocated (`tests/frames_alloc.rs`).
`context_bytes` (the neighbour bytes that matter) equals scarletbook-pcm's.

## Progress and cancel

`dsd64_to_pcm_with` calls its hook on the calling thread before the first block and after
each block is written (never from the worker threads), with the output samples per channel
done and the total; `Break` returns `Error::Cancelled` at once. Both file drivers (the
default filter's and pysacd's) call it through `convert::report` at their block
boundaries. Whole-file conversion passes the hook through. The frame-run API needs none: each call is one run.

## C interface (`crates/dsd2pcm-capi`)

A thin layer over the Rust API, described in [c-api.md](c-api.md). Every struct it reads or writes
starts with `struct_size`: a size below the 0.1.0 layout is refused before any access,
only the known fields of a larger one are read, and output structs get
`min(struct_size, ours)` bytes copied from a padding-free local. All of its `unsafe` code
is in `src/ffi.rs`: each entry point checks pointers for NULL, lengths against the
converter, alignment for `int32_t` output, and that no output buffer overlaps another
buffer, before it makes any Rust reference; the body runs inside `catch_unwind`. A
converter handle is a `Mutex<Converter>`: a second thread calling it at the same time gets
`DSD2PCM_E_BUSY`, and a handle whose call panicked reports `DSD2PCM_E_INTERNAL` from then
on. Runs given as one buffer per frame are converted in groups of 32 frames, each with its
real neighbours and its position (`at_frame(first_frame + g)`, for pysacd's dither), so the frame references live on the stack and nothing is allocated; by the
run property the bytes equal one call.

## Speed

- S1 reads each window as 64-bit words cut into 13-bit chunks, one table per chunk (5
  lookups per output instead of 8 byte tables; 12-byte wide windows: 8 instead of 12). The
  sum is the same integer. On 32-bit targets below AVX2 (the generic and `sse2` levels) it
  reads 32-bit words (a 64-bit shift takes a register pair there) with byte tables, as
  scarletbook-pcm does: 8 KiB for S1's window, which stays in the level-1 cache of small
  in-order cores such as Atom, where every table read from the level-2 cache stalls the
  core. The AVX2 levels keep the 13-bit chunks on 32-bit targets too (CPUs with large
  caches, where fewer lookups win). `s1_chunks_match_bytes` checks both layouts.
- The FIR kernels compute blocks of independent outputs (16 generic, 32 with AVX2; the
  generic Fir1 8 on 32-bit x86, whose SSE2 has 8 vector registers), so the compiler puts
  them in vector lanes; each output keeps its own sum in tap order.
- Lanes (`decimator/lanes.rs`; x86-64 with AVX2 and FMA, plans whose every stage is proven
  exact, passes of at least 4096 outputs: below that, each run's own filter margins cost
  more than the lanes gain, e.g. one frame at 88.2 or 176.4 kHz). The ordered kernels load
  unaligned input vectors, many of them across cache lines. A pass in lanes splits its
  outputs into 4 runs of consecutive outputs (each a whole number of blocks; the last runs
  move back to end with the padded pass, so runs may overlap) and computes each run as the
  ordered pass would, with run `w` in lane `w` of every vector: the same operations per
  output in the same order, but every load aligned, decimation and interpolation as index
  arithmetic, and no copies between stages. Blocks of 8 output vectors keep 8 independent
  sums. `lanes_match_ordered` compares lanes and ordered passes, with and without the
  paired tables, at every level, and the golden hashes run at every level. Reusing loaded vectors across taps (groups of taps and outputs, partial sums) was
  faster on P-cores and Zen 5 but slower than scarletbook-pcm on Gracemont E-cores, whose
  128-bit units make the extra additions and register pressure cost more; a hybrid CPU can
  move a thread between core types, so one layout serves both.
- Paired S1 tables (where lanes run, in lanes and in the ordered passes): the windows of two
  neighbouring outputs are read as one window one byte longer, one table per byte, and each
  entry holds both contributions, biased to be non-negative, in the two 32-bit halves of a
  `u64` (no half of a sum reaches 2^32, so they never carry into each other:
  `sums_are_exact`); one set of 9 lookups (13 for the wide filter) gives two outputs.
  Tables: 18 KiB, wide 26 KiB, which stay in the L1 cache (wider chunks needed fewer lookups
  but sat in L2, slower on Zen 2 and E-cores). A first Dec2 in an ordered pass reads its
  even and odd inputs from two S1 streams written by one set of lookups per pair.
- Frame runs converted a frame at a time (`frames`): each channel keeps the S1 values of its
  last ordered pass (`decimator::Carry`), and the next call reuses those whose window bytes
  are the same in both calls: compared byte for byte, never assumed, so a run that jumps,
  repeats or has another neighbour gets fresh values
  (`calls_out_of_order_or_with_other_neighbours`). It saves recomputing the filter margins
  that the previous call already computed (about 5 % at 88.2 kHz).
- Lanes on 32-bit x86 (level `sse2`): the same 4 runs, with blocks sized for the 8 SSE2
  registers (2 output vectors, 4 sums, in Fir1 and Interp2; 4 output vectors in Dec2,
  which measured faster so). Each tap step slices its windows once and loads whole
  vectors, so the additions take aligned memory operands: no unaligned or cache-line
  splitting loads, which in-order Atom cores pay heavily for. S1 uses the byte tables of
  the ordered kernels, in lanes and in short ordered passes, without paired tables or the
  frame carry (each 64-bit addition takes two instructions there). Passes run in lanes from
  1024 outputs (2048 with the wide filter): one frame at 88.2 kHz, and at 176.4 kHz wide, is
  faster in lanes there. The AVX2 levels on 32-bit x86 and the generic level keep the
  ordered kernels. An SSSE3 build measured within noise of SSE2 (only the 24-bit packing
  could use `pshufb`), so there is no SSSE3 level.
- Lanes on x86-64 without AVX2 (level `sse2`, e.g. the Goldmont and Tremont cores of
  Celeron and Pentium NAS chips): the same loop as on 32-bit x86, with blocks of 4 output
  vectors (8 sums in the 16 SSE2 registers) in every stage, the paired S1 tables and the
  frame carry (64-bit additions are cheap there), and lanes from 1024 outputs (2048 wide).
  An SSE4.2 build measured within noise of SSE2, so there is no SSE4 level. AVX2 CPUs never
  pick this level; their loop loads vectors through slices, which measured 10-15 % faster
  with AVX2 than the whole-vector form.
- Run-time dispatch (`decimator/dispatch.rs`, the crate's only `unsafe`): generic, SSE2
  (picked on x86 and x86-64 CPUs without AVX2), AVX2, and AVX2 with FMA. FMA is used only in stages that `decimator/exact.rs` proves exact from
  the tables and stage plan (every product and partial sum an integer below 2^53); the
  test `sums_are_exact` fails if a table or stage change breaks the proof, and such a stage
  runs unfused. `DSD2PCM_SIMD=generic|sse2|avx2|avx2fma` caps the level; the golden hashes in
  `tests/identity.rs` are checked at every level the CPU has.
- Whole-file conversion quantises each piece straight into its slot of a per-block sample
  buffer and interleaves once into the output bytes; stage input is read straight from the
  channel buffer when it holds the whole window.

## Robustness

Parsers check every size against the file length, cap allocations (DSF metadata 16 MiB,
block size 1 MiB; FLAC one decoded block) and return errors instead of panicking. Chunk
walking is bounded whatever reader the caller passes: at most 4096 chunks (DFF chunks and
`PROP` property chunks, WAV chunks before `data` and after a placeholder `data` size, FLAC
metadata blocks), and the walkers move with short reads or `Seek::seek_relative`, never a
plain `seek` per chunk, so a buffered reader such as a large `BufReader<File>` keeps its
buffer and opening a file costs time linear in its header. The tests
truncate and corrupt valid files byte by byte, and cargo-fuzz targets cover every reader
(see [fuzzing.md](fuzzing.md)).
