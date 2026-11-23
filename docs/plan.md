# Plan

Done in 0.1.0: DSD64 to PCM byte-identical to scarletbook-pcm 0.7.0 at every SIMD level
(AVX2, AVX2 with FMA where exactness is proven, generic), faster than it on one thread on the
CPUs in [performance.md](performance.md) with the gaps listed there; DSF/DFF/WAV/FLAC input,
DSF/DFF/WAV output; the frame-run interface; the opt-in pysacd filter; the C interface;
fuzzed readers; golden hashes on x86_64, i686 and aarch64.

Next:

1. Close the remaining speed gaps: one-frame calls on Zen 2, and a dedicated SSE2/SSSE3
   level for 32-bit x86 and CPUs without AVX2.
2. PCM output from DSD128 and DSD256; the 48 kHz family; RF64 for WAV over 4 GiB; FLAC
   output.
3. A Windows wide-character path API in the C interface.
