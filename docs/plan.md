# Plan

Done in 0.1.0: DSD64 to PCM byte-identical to scarletbook-pcm 0.7.0 at every SIMD level
(AVX2, AVX2 with FMA where exactness is proven, generic), faster than it on one thread on the
CPUs in [performance.md](performance.md) with the gaps listed there; DSF/DFF/WAV/FLAC input,
DSF/DFF/WAV output; the frame-run interface; the opt-in pysacd filter; the C interface;
fuzzed readers; golden hashes on x86_64, i686 and aarch64.

Done in 0.1.1: one-frame calls ahead of scarletbook-pcm 0.7.0 on Zen 2 and on the measured
Intel cores; an `sse2` level for 32-bit x86. Since 0.1.1 (unreleased): the `sse2` level on
x86-64 CPUs without AVX2. SSSE3 and SSE4.2 builds measured within noise of SSE2, so there
is no separate level for them ([design.md](design.md#speed)).

Next:

1. PCM output from DSD128 and DSD256; the 48 kHz family; RF64 for WAV over 4 GiB; FLAC
   output.
2. A Windows wide-character path API in the C interface.
