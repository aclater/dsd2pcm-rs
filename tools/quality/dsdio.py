# SPDX-License-Identifier: MIT OR Apache-2.0
"""File formats and the PCM-to-DSD-rate interpolator for tools/quality.

DSF (Sony, DSF File Format Specification 1.01), WAV (RIFF WAVE, PCM and
WAVE_FORMAT_EXTENSIBLE, 16/24/32-bit integer and 32/64-bit float) and the raw Super Audio
CD frame layout (channels interleaved byte by byte, earliest sample in the MSB).
"""

import math
import struct

import numpy as np
from scipy import signal as sps

BLOCK = 4096


def write_dsf(path, chans, rate, packed=False):
    """chans: (ch, n) bits 0/1 earliest first, or, if packed, a list of per-channel byte
    arrays packed MSB first (with n = 8 x bytes)."""
    if packed:
        data = [np.frombuffer(bytes(c), np.uint8) for c in chans]
        n = 8 * len(data[0])
        data = np.stack([np.unpackbits(d).reshape(-1, 8)[:, ::-1].reshape(-1) for d in data])
        data = np.packbits(data, axis=1)
    else:
        n = chans.shape[1]
        data = np.packbits(chans, axis=1, bitorder="little")  # DSF: earliest sample in the LSB
    ch = data.shape[0]
    nb = -(-data.shape[1] // BLOCK)
    pad = np.zeros((ch, nb * BLOCK), np.uint8)
    pad[:, : data.shape[1]] = data
    blocks = pad.reshape(ch, nb, BLOCK).transpose(1, 0, 2).reshape(-1).tobytes()
    fmt = struct.pack("<4sQIIIIIIQII", b"fmt ", 52, 1, 0, 2 if ch == 2 else 1, ch, rate, 1, n, BLOCK, 0)
    data_chunk = b"data" + struct.pack("<Q", 12 + len(blocks))
    total = 28 + len(fmt) + len(data_chunk) + len(blocks)
    with open(path, "wb") as f:
        f.write(b"DSD " + struct.pack("<QQQ", 28, total, 0) + fmt + data_chunk)
        f.write(blocks)


def read_dsf(path):
    """(bytes per channel MSB first as a (ch, nbytes) uint8 array, rate, samples)."""
    d = open(path, "rb").read()
    assert d[:4] == b"DSD "
    p = 28
    _, _, ver, fid, ctype, ch, rate, bps, n, block, _ = struct.unpack("<4sQIIIIIIQII", d[p:p + 52])
    p += 52
    assert d[p:p + 4] == b"data"
    size = struct.unpack("<Q", d[p + 4:p + 12])[0] - 12
    raw = np.frombuffer(d[p + 12:p + 12 + size], np.uint8)
    nb = len(raw) // (block * ch)
    x = raw[: nb * block * ch].reshape(nb, ch, block).transpose(1, 0, 2).reshape(ch, -1)
    nbytes = -(-n // 8)
    x = x[:, :nbytes]
    if bps == 1:  # LSB first -> MSB first
        x = np.packbits(np.unpackbits(x, axis=1, bitorder="little"), axis=1)
    return x, rate, n


def write_raw_frames(path, chans_msb):
    """Byte-interleaved MSB-first bytes (Super Audio CD frame layout)."""
    with open(path, "wb") as f:
        f.write(np.ascontiguousarray(chans_msb.T).reshape(-1).tobytes())


def write_wav24(path, q, fs):
    """q: int32 (n, ch) 24-bit samples."""
    n, ch = q.shape
    b = np.ascontiguousarray(q, dtype="<i4").view(np.uint8).reshape(n, ch, 4)[:, :, :3].tobytes()
    hdr = b"RIFF" + struct.pack("<I", 36 + len(b)) + b"WAVEfmt " + struct.pack(
        "<IHHIIHH", 16, 1, ch, fs, fs * ch * 3, ch * 3, 24) + b"data" + struct.pack("<I", len(b))
    with open(path, "wb") as f:
        f.write(hdr + b)


def read_wav(path):
    """(samples as float64 (n, ch) with full scale at 1.0, rate, info dict)."""
    d = open(path, "rb").read()
    assert d[:4] == b"RIFF" and d[8:12] == b"WAVE", path
    p, fmt = 12, None
    while p + 8 <= len(d):
        cid, size = d[p:p + 4], struct.unpack("<I", d[p + 4:p + 8])[0]
        body = d[p + 8:p + 8 + size]
        if cid == b"fmt ":
            tag, ch, fs, _, _, bits = struct.unpack("<HHIIHH", body[:16])
            if tag == 0xFFFE:
                tag = struct.unpack("<H", body[24:26])[0]
            fmt = (tag, ch, fs, bits)
        elif cid == b"data":
            tag, ch, fs, bits = fmt
            if tag == 3:
                v = np.frombuffer(body, "<f4" if bits == 32 else "<f8").astype(np.float64)
            elif bits == 24:
                raw = np.frombuffer(body[: len(body) // 3 * 3], np.uint8).reshape(-1, 3).astype(np.int32)
                v = raw[:, 0] | (raw[:, 1] << 8) | (raw[:, 2] << 16)
                v = (np.where(v >= 1 << 23, v - (1 << 24), v)).astype(np.float64) / 2**23
            elif bits == 32:
                v = np.frombuffer(body, "<i4").astype(np.float64) / 2**31
            elif bits == 16:
                v = np.frombuffer(body, "<i2").astype(np.float64) / 2**15
            else:
                raise ValueError(f"{bits}-bit WAV")
            v = v[: len(v) // ch * ch].reshape(-1, ch)
            return v, fs, {"format": "float" if tag == 3 else "int", "bits": bits}
        p += 8 + size + (size & 1)
    raise ValueError(f"{path}: no data chunk")


class Interpolator:
    """PCM at fs_in to fs_out (an integer or rational ratio) with a linear-phase
    Kaiser-windowed FIR: passband to 0.45 fs_in, stopband from 0.5 fs_in, 120 dB. The
    group delay is removed, so output sample m is at time m / fs_out. Works in blocks."""

    def __init__(self, fs_in, fs_out):
        g = math.gcd(fs_in, fs_out)
        self.up, self.down = fs_out // g, fs_in // g
        fhi = fs_in * self.up
        passb, stop = 0.45 * fs_in, 0.5 * fs_in
        ntaps, beta = sps.kaiserord(120, (stop - passb) / (fhi / 2))
        half = -(-(ntaps // 2) // self.down) * self.down  # a multiple of down
        ntaps = 2 * half + 1
        self.h = sps.firwin(ntaps, (passb + stop) / 2, window=("kaiser", beta), fs=fhi) * self.up
        self.half = (ntaps - 1) // 2  # delay at the upsampled rate
        self.margin = self.half // self.up + 2  # input samples of support on each side
        self.ntaps, self.beta = ntaps, beta

    def blocks(self, x, out_len, block=1 << 20):
        """Yields the output (length out_len) of the input x in blocks of `block` samples."""
        up, down = self.up, self.down
        m0 = 0
        while m0 < out_len:
            m1 = min(out_len, m0 + block)
            # Input samples that output m (at upsampled index m*down + half) needs.
            j_lo = (m0 * down + self.half - (len(self.h) - 1)) // up - 1
            j_hi = (m1 * down + self.half) // up + 2
            j_lo -= j_lo % down  # keep the polyphase alignment (j_lo * up divisible by down)
            j_lo_c, j_hi_c = max(j_lo, 0), min(j_hi, len(x))
            seg = np.zeros(j_hi - j_lo)
            seg[j_lo_c - j_lo: j_hi_c - j_lo] = x[j_lo_c:j_hi_c]
            y = sps.upfirdn(self.h, seg, up, down)
            # y[k] is upsampled index j_lo*up + k*down; we need index m*down + half.
            off = (m0 * down + self.half - j_lo * up) // down
            yield y[off: off + (m1 - m0)]
            m0 = m1


def write_wav_float(path, x, fs):
    """x: float (n, ch), written as 32-bit IEEE float WAV."""
    n, ch = x.shape
    b = np.ascontiguousarray(x, dtype="<f4").tobytes()
    hdr = b"RIFF" + struct.pack("<I", 36 + len(b)) + b"WAVEfmt " + struct.pack(
        "<IHHIIHH", 16, 3, ch, fs, fs * ch * 4, ch * 4, 32) + b"data" + struct.pack("<I", len(b))
    with open(path, "wb") as f:
        f.write(hdr + b)
