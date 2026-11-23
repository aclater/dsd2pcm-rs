"""Writes a WAVE_FORMAT_EXTENSIBLE file of the test samples (see tests/flac.rs, `sample`).

Usage: gen.py BITS CHANNELS RATE FRAMES OUT.wav. Samples of BITS valid bits sit at the top
of a 16, 24 or 32-bit container, so `flac` encodes them at BITS bits per sample.
"""

import struct
import sys


def sample(i, c, bits):
    full = 1 << (bits - 1)
    period = 64 + 16 * c
    v = (i % period) * 2 * full // period - full
    v += ((i * 1103515245 + c * 12345) >> 7) % 16 - 8
    if i == 0:
        v = -full
    if i == 1:
        v = full - 1
    return max(-full, min(full - 1, v))


def main():
    bits, ch, rate, n = map(int, sys.argv[1:5])
    container = 16 if bits <= 16 else 24 if bits <= 24 else 32
    bs = container // 8
    data = bytearray()
    for i in range(n):
        for c in range(ch):
            v = sample(i, c, bits) << (container - bits)
            data += v.to_bytes(4, "little", signed=True)[:bs]
    fmt = struct.pack("<HHIIHH", 0xFFFE, ch, rate, rate * ch * bs, ch * bs, container)
    mask = (1 << ch) - 1
    fmt += struct.pack("<HHI", 22, bits, mask)
    fmt += bytes.fromhex("0100000000001000800000aa00389b71")
    body = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt
    body += b"data" + struct.pack("<I", len(data)) + bytes(data)
    with open(sys.argv[5], "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", len(body)) + body)


main()
