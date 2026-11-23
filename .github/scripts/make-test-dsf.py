"""Write a deterministic DSD64 DSF file for CI.

    python3 -I .github/scripts/make-test-dsf.py OUT.dsf
    python3 -I .github/scripts/make-test-dsf.py --music SECONDS CHANNELS OUT.dsf

Without options: two seconds of stereo pseudo-random DSD bits, for the release binaries'
smoke test (the same file on every platform, so every binary's output can be compared by
hash).

With --music: SECONDS of CHANNELS-channel DSD from a second-order sigma-delta modulator fed
with a few tones and a little noise per channel, for the speed benchmark
(.github/workflows/bench.yml). One second per channel is modulated and repeated. Integer
arithmetic except for a 4096-entry sine table rounded to 16 bits. Standard library only.
"""

import math
import struct
import sys

RATE = 2_822_400
BLOCK = 4096  # bytes per channel per block
SMOKE_CHANNELS = 2
SMOKE_BLOCKS = 172  # 172 * 4096 * 8 bits = just under 2 s per channel


def bits(seed):
    # xorshift64: fixed output on every platform.
    x = seed
    while True:
        x ^= (x << 13) & 0xFFFFFFFFFFFFFFFF
        x ^= x >> 7
        x ^= (x << 17) & 0xFFFFFFFFFFFFFFFF
        yield x & 0xFF


def write_dsf(path, chans):
    """chans: one bytes object per channel (LSB first), each a whole number of blocks."""
    n = len(chans[0])
    data = bytearray()
    for b in range(0, n, BLOCK):
        for c in chans:
            data += c[b : b + BLOCK]
    samples = n * 8
    fmt = struct.pack(
        "<4sQIIIIIIQII", b"fmt ", 52, 1, 0, 2, len(chans), RATE, 1, samples, BLOCK, 0
    )
    data_chunk = struct.pack("<4sQ", b"data", 12 + len(data)) + data
    total = 28 + len(fmt) + len(data_chunk)
    head = struct.pack("<4sQQQ", b"DSD ", 28, total, 0)
    with open(path, "wb") as f:
        f.write(head + fmt + data_chunk)


def smoke(path):
    gens = [bits(0x9E3779B97F4A7C15 + c) for c in range(SMOKE_CHANNELS)]
    chans = [bytes(next(g) for _ in range(SMOKE_BLOCKS * BLOCK)) for g in gens]
    write_dsf(path, chans)


TABLE = 4096
SINE = [round(32767 * math.sin(2 * math.pi * i / TABLE)) for i in range(TABLE)]
# Tones in Hz per channel (whole numbers, so one second repeats without a seam).
TONES = [(440, 1320, 5000), (523, 1569, 7040), (392, 2349, 9000),
         (330, 990, 3500), (55, 110, 220), (587, 1760, 11000)]


def music_second(c):
    """One second of one channel: a second-order sigma-delta modulator, LSB first."""
    tones = TONES[c % len(TONES)]
    # Phase steps in 1/2^32 of a turn, exact for whole-Hz tones over one second.
    steps = [f * TABLE for f in tones]
    noise = bits(0xD1B54A32D192ED03 + c)
    out = bytearray(RATE // 8)
    s1 = s2 = 0
    full = 1 << 22  # feedback level; the input stays under half of it
    for i in range(RATE // 8):
        byte = 0
        for k in range(8):
            n = i * 8 + k
            x = 0
            for st in steps:
                x += SINE[(st * n // RATE) % TABLE]
            # 3 tones of 32767 at most: at most about 0.47 of the feedback level, plus noise.
            x = x * 20 + (next(noise) - 128) * 64
            y = full if s2 >= 0 else -full
            s1 += x - y
            s2 += s1 - y
            if y > 0:
                byte |= 1 << k
        out[i] = byte
    return bytes(out)


def music(seconds, channels, path):
    # Whole blocks covering the requested length.
    total = -(-seconds * (RATE // 8) // BLOCK) * BLOCK
    chans = []
    for c in range(channels):
        sec = music_second(c)
        reps = -(-total // len(sec))
        chans.append((sec * reps)[:total])
    write_dsf(path, chans)


def main():
    args = sys.argv[1:]
    if args and args[0] == "--music":
        music(int(args[1]), int(args[2]), args[3])
    else:
        smoke(args[0])


if __name__ == "__main__":
    main()
