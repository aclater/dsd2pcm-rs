[dsd2pcm-rs](../README.md) › [Documentation](README.md) › Plan

# Plan

What each release delivered and what comes next.

Done in 0.1.0: DSD64 to PCM byte-identical to scarletbook-pcm 0.7.0 at every SIMD level
(AVX2, AVX2 with FMA where exactness is proven, generic), faster than it on one thread on the
CPUs in [performance.md](performance.md) with the gaps listed there; DSF/DFF/WAV/FLAC input,
DSF/DFF/WAV output; the frame-run interface; the opt-in pysacd filter; the C interface;
fuzzed readers; golden hashes on x86_64, i686 and aarch64.

Done in 0.1.1: one-frame calls ahead of scarletbook-pcm 0.7.0 on Zen 2 and on the measured
Intel cores; an `sse2` level for 32-bit x86.

Done in 0.1.2: the `sse2` level on x86-64 CPUs without AVX2; testing for long-lived
processes (soak, leak loop, Miri, ThreadSanitizer, valgrind). SSSE3 and SSE4.2 builds measured within noise of SSE2, so there
is no separate level for them ([design.md](design.md#speed)).

In 0.2.0: the `precise` filter as the default, for its measured quality (SINAD 142 to
145 dB with a very quiet DSD64 source, against 122 dB), at 1.3 to 1.9 times
scarletbook-pcm 0.7.0's CPU time; `fast` keeps scarletbook-pcm's filter and bytes. Output is
deterministic within a release and may change between releases when measured quality
improves, with every change in the changelog ([design.md](design.md#the-contract-deterministic-output-the-deepest-stopband-filter-by-default)).

Next:

1. PCM output from DSD128 and DSD256; the 48 kHz family; RF64 for WAV over 4 GiB; FLAC
   output.
2. A Windows wide-character path API in the C interface.

## See also

- [Changelog](../CHANGELOG.md): the changes in each release
- [Performance](performance.md): the speed and its gaps
- [User guide](user-guide.md#limits): the current limits

Part of the [DSD Toolbox](https://github.com/dsd-toolbox/dsd-toolbox).
