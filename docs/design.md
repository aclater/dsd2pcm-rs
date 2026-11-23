# Design

## Data flow

Readers deliver DSD as one byte vector per channel, earliest sample in bit 7, 1 = +1
(DSDIFF order; the DSF reader and writer reverse the bits of each byte). PCM readers deliver
`f64` per channel with full scale at +-1.0.

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

## Robustness

Parsers check every size against the file length, cap allocations (DSF metadata 16 MiB,
block size 1 MiB) and return errors instead of panicking. The tests truncate and corrupt
valid files byte by byte.
