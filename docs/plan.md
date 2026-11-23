# Plan

1. Done: port scarletbook-pcm 0.7.0, prove identical output (synthetic and real files,
   every rate and cutoff, gains 0/-3/+6 dB), DSF/DFF/WAV I/O, CLI.
2. Faster with identical output: SIMD across channels or a fixed reduction order on every
   ISA, no FMA; timings on a lab host under benchlock against scarletbook-pcm 0.7.0.
3. PCM output from DSD128 and DSD256; 48 kHz family; RF64 for WAV over 4 GiB; FLAC output.
4. Opt-in filters (never the default); a C interface.
5. Golden-hash check on aarch64.
