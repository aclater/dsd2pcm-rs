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
  sum is the same integer.
- The FIR kernels compute blocks of independent outputs (16 generic, 32 with AVX2), so
  the compiler puts them in vector lanes; each output keeps its own sum in tap order.
- Lanes (`decimator/lanes.rs`; x86-64 with AVX2 and FMA, plans whose every stage is proven
  exact, passes of at least 4096 outputs: below that, each run's own filter margins cost
  more than the lanes gain, e.g. one frame at 88.2 or 176.4 kHz). The ordered kernels load
  unaligned input vectors, many of them across cache lines. A pass in lanes splits its
  outputs into 4 runs of consecutive outputs (each a whole number of blocks; the last runs
  move back to end with the padded pass, so runs may overlap) and computes each run as the
  ordered pass would, with run `w` in lane `w` of every vector: the same operations per
  output in the same order, but every load aligned, decimation and interpolation as index
  arithmetic, and no copies between stages. Blocks of 8 output vectors keep 8 independent
  sums. `lanes_match_ordered` compares lanes and ordered passes at every level, and the
  golden hashes run at every level. 32-bit x86 and the generic level keep the ordered
  kernels. Reusing loaded vectors across taps (groups of taps and outputs, partial sums) was
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
- Run-time dispatch (`decimator/dispatch.rs`, the crate's only `unsafe`): generic, AVX2,
  and AVX2 with FMA. FMA is used only in stages that `decimator/exact.rs` proves exact from
  the tables and stage plan (every product and partial sum an integer below 2^53); the
  test `sums_are_exact` fails if a table or stage change breaks the proof, and such a stage
  runs unfused. `DSD2PCM_SIMD=generic|avx2|avx2fma` caps the level; the golden hashes in
  `tests/identity.rs` are checked at every level the CPU has.
- Whole-file conversion quantises each piece straight into its slot of a per-block sample
  buffer and interleaves once into the output bytes; stage input is read straight from the
  channel buffer when it holds the whole window.

## Robustness

Parsers check every size against the file length, cap allocations (DSF metadata 16 MiB,
block size 1 MiB; FLAC one decoded block) and return errors instead of panicking. The tests
truncate and corrupt valid files byte by byte, and cargo-fuzz targets cover every reader
(see [fuzzing.md](fuzzing.md)).
