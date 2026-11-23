"""Write a small stereo DSD64 DSF file for the release binaries' smoke test.

    python3 -I .github/scripts/make-test-dsf.py OUT.dsf

Two seconds of deterministic pseudo-random DSD bits (the same file on every platform), so
every release binary's output can be compared by hash. Standard library only.
"""

import struct
import sys

RATE = 2_822_400
CHANNELS = 2
BLOCK = 4096  # bytes per channel per block
BLOCKS = 172  # 172 * 4096 * 8 bits = just under 2 s per channel


def bits(seed):
    # xorshift64: fixed output on every platform.
    x = seed
    while True:
        x ^= (x << 13) & 0xFFFFFFFFFFFFFFFF
        x ^= x >> 7
        x ^= (x << 17) & 0xFFFFFFFFFFFFFFFF
        yield x & 0xFF


def main():
    gens = [bits(0x9E3779B97F4A7C15 + c) for c in range(CHANNELS)]
    data = bytearray()
    for _ in range(BLOCKS):
        for g in gens:
            data += bytes(next(g) for _ in range(BLOCK))
    samples = BLOCKS * BLOCK * 8
    fmt = struct.pack("<4sQIIIIIIQII", b"fmt ", 52, 1, 0, 2, CHANNELS, RATE, 1, samples, BLOCK, 0)
    data_chunk = struct.pack("<4sQ", b"data", 12 + len(data)) + data
    total = 28 + len(fmt) + len(data_chunk)
    head = struct.pack("<4sQQQ", b"DSD ", 28, total, 0)
    with open(sys.argv[1], "wb") as f:
        f.write(head + fmt + data_chunk)


if __name__ == "__main__":
    main()
