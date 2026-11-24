[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Design

# Design

The design decisions behind dsd2pcm's DSD-to-PCM conversion. For each one: what was chosen,
the alternatives, and why. The measured filter responses, noise and distortion are in
`pcm-quality.md` (in preparation); speed is in [performance.md](performance.md).

## The contract: scarletbook-pcm 0.7.0's samples

**Chosen.** With the default filter, every output sample equals scarletbook-pcm 0.7.0's
sample for the same rate, cutoff and gain. That holds on every CPU, at every SIMD level,
in every build profile, and however the input is split into calls. The tests compare the
two crates directly (scarletbook-pcm is a dev-dependency) and pin golden hashes at every
level. Output bytes change only with a major version.

**Why.** A file converted twice, on two machines, or once whole and once frame by frame,
gives the same bytes. Copies, caches and library scanners can then trust a hash. The
filters are scarletbook-pcm's published, measured designs, so dsd2pcm takes over their
quality, and a consumer can switch between the two crates with a mechanical diff.

**Alternatives.** A converter of our own design, or pysacd's filter as the default. Either
would need its own measurements and give other bytes. Our own converter could not be
compared sample for sample with anything published. pysacd's filter is offered as an
opt-in preset instead ([below](#the-pysacd-preset)).

## The filter chain

**Chosen.** Multistage decimation with linear-phase (symmetric, odd-length) FIR stages. The
integer taps (`decimator/tables.rs`) are copied unchanged from scarletbook-pcm 0.7.0, which
designed them by the Parks-McClellan method (Remez exchange) in its `design.rs`, for 110 dB
of stopband attenuation against a 105 dB requirement. The margin covers coefficient
rounding.

| Stage | Rate | Taps | Role |
|---|---|---|---|
| S1 | 2822.4 to 352.8 kHz (÷ 8) | 63 (95 for `wide`) | First decimation, evaluated by table lookup on the input bytes |
| Halfband | 352.8 to 176.4 kHz (÷ 2) | 31 | Second decimation (`30`, `40`) |
| Sharp filter | 176.4 kHz | 121 (`40`), 161 (`30`) | The band edge; decimates by 2 more for 88.2 kHz |
| Halfband | 176.4 to 352.8 kHz (× 2) | 31 | Interpolation for 352.8 kHz output (`30`, `40`) |
| Wide filter | 352.8 kHz | 233 (÷ 2 to 176.4 kHz) or 121 (352.8 kHz) | The 80 kHz band edge |

The 121-tap 40 kHz filter at 176.4 kHz is, at 352.8 kHz, an 80/96 kHz filter, so the
wide filter at 352.8 kHz reuses its taps.

**Why multistage.** One filter from 2.8224 MHz straight to 176.4 kHz with a 40 to 48 kHz
transition would need thousands of taps. Each stage of the chain only has to keep its own
output band free of aliases, so the first stage, which runs at the highest rate, is short
(Crochiere & Rabiner). Halfband stages have every other tap exactly zero and a centre tap of
exactly 1/2 (Vaidyanathan & Nguyen), which halves their work.

**Why tables for S1.** The input is 1-bit, so the sum of the taps over one byte's 8 samples
takes only 256 values. S1 adds precomputed partial sums, one table lookup per input byte,
instead of multiplying (Peled & Liu; the technique of Sebastian Gesemann's dsd2pcm). This
crate cuts the window into wider chunks or byte pairs where that is faster
([speed](#speed)); the integer sums are the same.

**Why linear phase and no delay.** Symmetric taps give exactly linear phase: no phase
distortion, and an impulse response symmetric about its sample. Output sample `n` is
centred on DSD sample `d × n` (`d` = 32, 16 or 8), so the PCM lines up with the DSD it came
from, with no delay to compensate. The cost is the pre-ringing of any linear-phase brick-wall
filter.

**Why exact silence.** S1's integer taps contain the factor `(1 + z^-1)^2 (1 + z^-4)`. That
factor cancels both SACD idle patterns (0x69 and 0x55) exactly, at any bit phase, so DSD
silence converts to samples of exactly 0. The input is taken to continue with 0x69 before
its start and after its end. A track that starts at the edge of an area therefore starts
from digital zero, and a silent channel is all zeros. *Alternatives:* zero bytes, which are
-100 % modulation, a full-scale negative step; or repeating the first or last frame, which
would invent audio.

## Rates and cutoffs

**Rates.** 88.2, 176.4 and 352.8 kHz: DSD64 divided by 32, 16 and 8. Integer ratios keep
every stage a plain decimator or interpolator, and every output sample is centred on a DSD
sample. *Not offered (yet):* the 48 kHz family, which would need fractional resampling and
so a different filter structure ([plan](plan.md)). 176.4 kHz is the default, as in
scarletbook-pcm: it is the lowest rate that holds every cutoff, `wide` included.

**Cutoffs.**

| Cutoff | Passband | Stopband from | Rates |
|---|---|---|---|
| `40` (default) | 40 kHz | 48 kHz | all |
| `30` | 30 kHz | 36 kHz | all |
| `wide` | 80 kHz | 88.2 kHz (176.4 kHz), 96 kHz (352.8 kHz) | 176.4 and 352.8 kHz |

- **40 kHz** passes the whole audio band with a wide margin and is attenuated by 48 kHz.
  Above that, DSD's shaped noise rises steeply.
- **30 kHz** removes more of that noise, for playback chains that do not want ultrasonic
  content.
- **Wide** follows the Philips blind tests, which found 80 to 100 kHz filters undetectable
  and a 50 kHz one detectable (the decision recorded in sacdfs's
  [PCM quality](https://github.com/aclater/sacdfs/blob/main/docs/pcm-quality.md#the-filters)
  page).
- **Why wide needs 176.4 kHz or more.** At 88.2 kHz the band ends at 44.1 kHz, below the
  80 kHz passband. At 176.4 kHz the stopband starts at the Nyquist frequency, 88.2 kHz, so
  nothing folds back below it. The 80 to 88.2 kHz transition must be made at 352.8 kHz. A
  filter at 176.4 kHz, or one in `z^2` at 352.8 kHz, has a response symmetric about
  88.2 kHz and cannot make it.

The CLI and the C interface refuse `wide` at 88.2 kHz rather than substituting another
filter.

## Arithmetic, rounding and dither

**Chosen.** Integer arithmetic carried in `f64`. Samples between stages are integers on a
grid where full scale (100 % modulation) is 2^26, 8 times finer than 24 bits.
Coefficients are integers scaled by 2^26 (S1) or 2^24 (later stages). Every product and
partial sum is an integer below 2^53, so `f64` holds it exactly, and summation order cannot
change a result. Each stage rounds half to even back onto the grid. The last step rounds to
24 bits, also half to even.

**Why.** Exact integer sums make the output independent of vector width, summation order
and CPU, which the contract needs. Floating-point filters give different last bits on
different SIMD paths unless every operation is pinned. Half-to-even rounding is unbiased:
it adds no DC offset. The 2^26 grid keeps the rounding between stages three bits below the
final 24-bit step.

**No libm at run time.** The taps are committed integers. scarletbook-pcm's design code,
which uses `cos` and `exp`, never runs in the converter, so a platform's libm cannot change
a sample. The gain factor `db_to_linear` uses only additions, multiplications and
divisions in a fixed order (ported unchanged): exactly 1 at 0 dB, identical everywhere. Every
other floating-point operation at conversion time is one that IEEE 754 and Rust specify
exactly: additions, multiplications, the fused multiply-add of the AVX2 + FMA level,
conversions between `f64`, `f32` and integers, `clamp` and `round_ties_even`. No
transcendental function (`powf`, `powi`, `exp`, `log2`, ...) runs on the output path, so
musl, glibc, macOS and Windows builds give the same bytes.

**No dither in the default filter.** The 24-bit rounding error is far below the noise
floor of DSD itself, so dither would only add noise. It would also need a seeded random
generator in every frame, and the output would no longer equal scarletbook-pcm's. The
pysacd preset keeps pysacd's TPDF dither, because that preset reproduces pysacd
([below](#the-pysacd-preset)).

## The exactness proof and FMA

**Chosen.** `decimator/exact.rs` proves, from the tables and the stage plan, that every
operand, product and partial sum of every stage is an integer below 2^53. The bound is
`sum |h| × x`, with `x` the stage's input bound (the bounds of scarletbook-pcm's
`f64_sums_are_exact`, turned into a check that runs on every plan). Where the proof holds,
a fused multiply-add (one rounding) and a multiply then add (two roundings) give the same
integer, so those stages may use FMA. The test `sums_are_exact` fails if a table or plan
change breaks the proof, and such a stage runs unfused.

**Why.** FMA is faster where the CPU has it. Allowing it only where it provably cannot change a
bit keeps the contract. The generic kernels never use `mul_add`.

## Gain, headroom and clipping

**Chosen.** Gain from -120 to +24 dB, 0 dB by default, applied before the 24-bit rounding.
At 0 dB, 100 % DSD modulation is full scale. SACD's reference level is 50 % modulation
(about -6 dBFS), so real programme material has about 6 dB of headroom at unity gain. A
sample that exceeds the 24-bit range is clamped and counted. The counts are reported per
channel, and in the frame-run API per frame. A frame's count is the same whether it is
converted alone or in a run.

**Why.** The 0 dB mapping is scarletbook-pcm's, so the samples stay identical. Clamping
with a count, rather than scaling down automatically, keeps the output a fixed function of
the input and tells the caller exactly where to lower the gain. 100 % positive modulation
is one step above the largest 24-bit code, so a sustained all-ones input reports clips;
real discs stay far below it. *Alternative:* a fixed -6 dB pad, which would give up 6 dB of
resolution on every disc to avoid clips that real discs do not have.

## Output formats

**Chosen.** The converter produces 24-bit integers. The frame-run API writes them as
3-byte little-endian samples (`PcmOut::S24le`), ready for a WAV or FLAC writer, or as
sign-extended `i32` (`PcmOut::I32`). Whole-file conversion writes 24-bit WAV
(`WAVE_FORMAT_EXTENSIBLE`, as Microsoft's rules ask above 16 bits). The library's WAV writer also takes 16- and
32-bit integers and 32- and 64-bit floats, for callers that write other PCM.

**Why.** 24 bits holds the converter's precision. Writing S24le straight into the caller's
buffer saves a copy and a packing pass per run. 32-bit or float output of the same samples
would add bytes and no information.

## SIMD levels and dispatch

**Chosen.** One set of safe kernels, compiled per level with `#[target_feature]` and picked
at run time (`decimator/dispatch.rs`, the crate's only `unsafe`):

| Level | Where | FMA |
|---|---|---|
| `avx2fma` | x86 and x86-64 with AVX2 and FMA | in proven stages |
| `avx2` | x86 and x86-64 with AVX2, no FMA | no |
| `sse2` | x86-64 and 32-bit x86 without AVX2 | no |
| `generic` | every target | no |

A `neon` level for aarch64 is in preparation. `DSD2PCM_SIMD` caps the level, for tests and
benchmarks.

**Why run-time dispatch.** One binary runs at full speed on a NAS Celeron and on a desktop
CPU. Release builds target the baseline (x86-64, i686, aarch64) and still use AVX2 and FMA
where the CPU has them. *Alternative:* builds per CPU (`-C target-cpu`), which would multiply
the release archives and break on the wrong CPU. Every level gives the same bytes, because
the arithmetic is exact, so the choice affects only speed. There is no SSSE3 or SSE4.2
level: both measured within noise of SSE2.

## Frame runs, threading and sharing

**Chosen.** `frames::Converter::convert` converts a run of SACD frames with the frame before
and after it as context (`None` at an area's edge: idle). Its types and semantics are
scarletbook-pcm 0.7.0's. Any run with its neighbours gives the bytes of that run in a
whole-area conversion. Only the neighbour bytes nearest the run matter (`context_bytes`,
66 to 195 bytes per channel, depending on rate and cutoff).

- **Why stateless runs.** A reader that seeks, such as a file system serving PCM from a disc
  image, converts any frame from three frames. A converter that carried filter state from
  frame to frame would need every earlier frame converted first.
- **The frame carry.** Each channel keeps the S1 values of its last ordered pass. The next
  call reuses those whose window bytes are the same, compared byte for byte, never assumed.
  It is a cache, not state: a run that jumps, repeats or has another neighbour gets fresh
  values and the same bytes. It saves about 5 % per frame at 88.2 kHz when a reader asks for
  consecutive frames one at a time.
- **Threading.** A converter works on the calling thread, one per thread. Callers bring
  their own threads or pools; a converter is not tied to a thread and can be handed from
  one pooled task to the next, because the carry is checked, not trusted. Whole-file
  conversion (`convert_file`) cuts each block into per-channel pieces of about 64 KiB of
  input and runs them on all cores (`--single-thread` turns that off). The output does not
  depend on how the work was split.
- **Out of threads.** The calling thread is one of the workers, and the others take pieces
  as they become free. If the system refuses a worker thread (its thread or memory limit),
  the conversion goes on with the threads it has, down to the calling thread alone, rather
  than failing: the output is the same, so a refused thread costs only speed, and an error
  would make a busy server fail conversions that one thread can do. Nothing is reported;
  `Error` gains no variant. (In 0.1.2 and earlier the standard library's `spawn` panicked
  there, and the C interface returned `DSD2PCM_E_INTERNAL`.) The tests inject a thread
  builder that refuses workers (`convert::tests`).
- **Shared tables.** The S1 lookup tables and the paired tables are built once per process
  (`OnceLock`) and shared, read-only, by every converter and thread. A converter only owns
  its buffers, and after the first call it allocates nothing (`tests/frames_alloc.rs`).
- **Blocks.** Runs are converted in blocks of 6 frames, so buffers are bounded by the block
  size, not the run length.

## The pysacd preset

**Chosen.** `Filter::Pysacd` (`--filter pysacd`) is pysacd's filter as an opt-in preset. It
uses three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered quantiser (`csrc/dsp.c`,
relicensed MIT OR Apache-2.0 by its author). Its bytes equal pysacd's reference build
(`-O2 -ffp-contract=off`, no SIMD dispatch) on every CPU and SIMD level. It covers 88.2 and
176.4 kHz only, with a fixed response.

- **Why the reference build, not pysacd's wheels.** pysacd's wheels use `-ffast-math`, FMA
  contraction and per-CPU clones, so their output varies by CPU, up to 7 LSB apart on its
  test vectors. pysacd also seeds its dither per batch of frames, so its bytes depend on how
  the input was batched. Neither is a target that can be reproduced. The reference build is.
  This preset differs from pysacd's wheels, and old pysacd output, by those few LSB.
- **Exact `f32`.** Each output is `((0 + h0 x0) + h1 x1) + ...`, every product rounded
  before it is added, as in the C loop. The kernels compute 32 outputs at a time, one per
  vector lane, and never reassociate a sum or fuse a multiply-add, so every level gives the
  same bits.
- **Dither seeded per frame.** The generator is seeded at the start of each frame of 4704
  bytes per channel from the frame's index in the area (`0x9E3779B9 ^ (index × 2654435761)`,
  32-bit wrapping). The bytes are then independent of how the area is split: files, threads
  and frame runs agree.
- **Stream.** Each area is one stream of the C filter started at its first byte. The first
  outputs (52 at 88.2 kHz, 26 at 176.4 kHz) are dropped to remove the delay, and the length
  equals the default path's.
- **Tests.** SHA-256 of the C reference's output (`tests/pysacd-ref/`), for silence, all-ones,
  synthetic and random input, 2 and 6 channels, both rates, three gains and large frame
  indices, at every SIMD level, through files, whole runs and split runs.

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
  `sums_are_exact`. The sums a bit stream can actually reach (from the composite taps,
  the roundings between stages left out) are up to 2^52.09 in the first stage after S1,
  which sees S1's sums unrounded (`wide` at 352.8 kHz; P 2^51.56, `wide` F80 2^51.26), and
  at most about 2^50.1 in every later stage;
  `plans_match_reference` checks every plan of both filters, at every SIMD level, against a
  direct computation of its definition in 128-bit integers; `precise_plans_match_the_design`
  writes the precise plans out as in the table above (taps by table, scale, order), checks
  the plans against them and runs the literal plans through that computation against the
  ordered and the lanes paths; `lanes_match_ordered` and the
  S1 table tests cover the new tables; golden hashes pin the output at every level.
- **Cost.** More taps per output than the default: see [performance](performance.md) and
  [PCM quality](pcm-quality.md#the-precise-filter) for the measured CPU time.

## Release profiles of consumers

**Finding.** The kernels rely on inlining and on the compiler seeing whole loops. A
consumer's release profile with 16 codegen units, no LTO and overflow checks on measured
25 % slower than dsd2pcm's own profile (`codegen-units = 1`, thin LTO). The bytes do not
change. Making the kernels robust to such profiles is planned for 0.1.3. Until then, a
consumer can build this crate with its own settings:

```toml
[profile.release.package.dsd2pcm]
codegen-units = 1
overflow-checks = false
```

(Cargo does not allow `lto` per package; it applies to the whole build.)

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
The reader looks into the first Vorbis comment block itself, for the
`WAVEFORMATEXTENSIBLE_CHANNEL_MASK` field only: it reads the length fields, checks each
against the block, reads comments of up to 64 bytes into a fixed buffer and skips longer
ones by seeking, so nothing is allocated. A malformed block gives no mask rather than an
error, so files that opened before still open.
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

### Intel CET

On x86_64 Linux the release marks every object of the static library `libdsd2pcm.a` for
the CET shadow stack (`.github/scripts/clib-shstk.py`), but not the shared library
`libdsd2pcm.so.1`. The release links the shared library with zig (cargo-zigbuild, for glibc
2.17), and zig satisfies `-lgcc_s` with its own LLVM libunwind, linked in statically and
built without shadow-stack support: it restores registers on unwinding without unwinding
the shadow stack. Every entry point of the C interface runs inside `catch_unwind`, so a
panic unwinds through that code to the boundary; in a process running with shadow stacks a
marked shared library would turn that caught panic into a control-protection fault. A
program that links the static library uses its own libgcc unwinder, which handles the
shadow stack on distributions built for CET, so the static library can carry the mark.
`check-capi-archive.sh` fails if the shared library is ever marked while it holds
libunwind. The way to a marked shared library is to link it against the system's
`libgcc_s` instead of zig's libunwind (not done yet).

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
  that the previous call already computed (about 5 % at 88.2 kHz). Only the bytes from where
  the next adjacent pass starts are kept (a few hundred), and a pass that does not continue
  the previous one fails the comparison at its first byte, so converters shared out over
  runs that are not adjacent (`sacdfs_path` mode `pool`) pay next to nothing. Runs of 4096
  outputs or more (four frames at any rate) run in lanes and do not use the carry.
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
- Release profiles: a consumer builds us with its own profile, often codegen units 16, no LTO
  and overflow checks on. Overflow checks in the inner loops once cost 25 % there (the tap
  loop's index arithmetic, and the closures of `array::from_fn` that then stayed out of line,
  so the sums went through the stack). The inner loops now use wrapping index arithmetic
  where the values are in range (asserted in debug builds; the slices check them again) and
  plain loops instead of closures. `.github/scripts/profile-ab.sh` times the default profile
  against that one in CI and fails if it is more than 5 % slower.

## Robustness

Parsers check every size against the file length, cap allocations (DSF metadata 16 MiB,
block size 1 MiB; FLAC one decoded block) and return errors instead of panicking. Chunk
walking is bounded whatever reader the caller passes. Each reader stops at 4096: DFF
top-level chunks up to and including `DSD `, DFF property chunks in `PROP`, WAV chunks up to
and including `data` (and the chunks walked after a placeholder `data` size), FLAC metadata
blocks including STREAMINFO (and 16 ID3v2 tags before `fLaC`). The walkers move with short
reads or `Seek::seek_relative`, never a plain `seek` per chunk, so a buffered reader such as
a large `BufReader<File>` keeps its buffer and opening a file costs time linear in its
header. The tests truncate and corrupt valid files byte by byte, and cargo-fuzz targets
cover every reader (see [fuzzing.md](fuzzing.md)).

## Output files

`convert_file` (and so `dsd2pcm_convert_file` and the tool) never writes into an existing
output in place, because a conversion that fails or is stopped part-way must not destroy the
file the user had, nor leave a partial WAV that a reader would take as complete.

- Every cheap check runs first: input, rate, options, the 4 GiB estimate, and the output
  path. A directory or a path ending in a separator, and an existing device, FIFO or socket,
  are refused (writing a WAV into a device node or a pipe is never what was meant, and
  replacing one would break the system). An existing regular file must be writable (it is
  opened for writing, without truncating, as a probe), as in 0.1.1 where it was overwritten.
- The output is the input if both name the same file: on Unix by device and inode, which
  also catches hard links; elsewhere by canonical path. Converting a file onto itself is
  refused rather than left to the rename, which would replace the input.
- The WAV goes to `.dsd2pcm-PID-N.tmp` in the output's directory, created exclusively (an
  existing file or symbolic link of that name is never opened). The name does not derive
  from the output's name, so a 255-byte output name works. Creating it is also the check
  that the directory is writable, before any conversion work. When it replaces a file it is
  created owner-only (0600 on Unix) and given the replaced file's permissions before the
  rename; owner, ACLs and extended attributes are not carried over.
- On success: flush, `fsync`, rename over the output (atomic on one file system), then a
  best-effort `fsync` of the directory on Unix. On any error, cancel or panic, the temporary
  file is removed by its drop guard. A symbolic link at the output path is replaced by the
  new file, not followed: the rename acts on the directory entry.
- The command-line tool turns SIGINT and SIGTERM (Ctrl-C and Ctrl-Break on Windows) into a
  cancel through the progress hook, so the same cleanup runs, and exits with 128 + the
  signal number; a second signal ends it at once. Only SIGKILL, a crash or a power loss can
  leave a `.dsd2pcm-PID-N.tmp` behind, which is safe to delete. The library installs no
  signal handlers: that is the application's choice.
- No-clobber (`OutputOptions::no_clobber`, `-n`, `dsd2pcm_file_options.no_clobber`): an
  existing output is refused before converting, and the result is put in place with
  "create, never replace" semantics, because a file can appear at the output path during a
  long conversion (another conversion of the same name, a copy) and checking first then
  renaming would replace it. The temporary file is hard-linked to the output path, which
  fails if anything is there, atomically, on every platform with hard links (Linux, macOS,
  the BSDs, Windows on NTFS), and the temporary name is then removed. That needs no `unsafe`
  or platform calls, unlike Linux's `renameat2(RENAME_NOREPLACE)` or macOS's
  `renamex_np(RENAME_EXCL)`, which also fail on some file systems. Where linking fails for
  another reason (no hard links: FAT, exFAT, some SMB, NFS and FUSE mounts), the output path
  is created empty with an exclusive create (`O_EXCL`; `CREATE_NEW` on Windows), which
  fails the same way, and the result is renamed over it. That path is best effort: a crash
  in the moment between the two leaves the empty file, and a process with write access to
  the directory that replaces the empty file in that moment has its file replaced by the
  rename. With hard links there is no such moment.
  Not exposed through `PcmOptions`, because adding a field to that struct would break
  every caller that builds it with a struct literal; `OutputOptions` is
  `#[non_exhaustive]`, so later output settings can be added to it.

## Known gaps

The rows where dsd2pcm is not ahead of scarletbook-pcm 0.7.0 are listed in
[performance.md](performance.md#known-gaps).

## Sources

- scarletbook-pcm 0.7.0 ([crate](https://crates.io/crates/scarletbook-pcm/0.7.0),
  [docs](https://docs.rs/scarletbook-pcm/0.7.0), MIT OR Apache-2.0): its README and
  `src/design.rs` describe the filter design, the stage plan, the idle-pattern factor and
  the exact arithmetic that this crate ports ([PROVENANCE.md](../PROVENANCE.md)). Its
  literature: Parks & McClellan 1972; Rabiner, McClellan & Parks 1975; Herrmann, Rabiner &
  Chan 1973; Crochiere & Rabiner 1981; Mintzer 1982; Vaidyanathan & Nguyen 1987; Peled &
  Liu 1974; White 1989; Goldberg 1991.
- [pysacd](https://github.com/aclater/pysacd): `csrc/dsp.c` and the quantiser of
  `process_batch()`, for the pysacd preset; Sebastian Gesemann's dsd2pcm for the byte-table
  technique it uses.
- sacdfs's [PCM quality](https://github.com/aclater/sacdfs/blob/main/docs/pcm-quality.md)
  and [design](https://github.com/aclater/sacdfs/blob/main/docs/design.md#pcm-conversion)
  pages: the reasons for the rates, cutoffs and gain, the 0 dB reference and the stateless
  frame interface, as a consumer of the same converter.
- This repository's history: the reasons recorded with each change (lane thresholds, table
  layouts, the frame carry, the SIMD levels) and the measurements in
  [performance.md](performance.md).

## See also

- [Performance](performance.md): the measured speed and its gaps
- [User guide](user-guide.md) and [options](options.md): the choices these decisions give
- [Provenance](../PROVENANCE.md): where the ported code comes from
- [Fuzzing](fuzzing.md): how the readers are tested against bad input

Part of the [DSD Toolbox](https://github.com/aclater/dsd-toolbox).
