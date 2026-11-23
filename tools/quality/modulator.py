# SPDX-License-Identifier: MIT OR Apache-2.0
"""Reference 1-bit sigma-delta modulator for the DSD64 test signals.

A 5th-order, 1-bit modulator in error-feedback form, so its signal transfer function is
exactly 1 and the bitstream is ``v = x + NTF * e``:

- NTF zeros: DC and two optimised pairs at 0.5385 and 0.9062 of a 20 kHz band
  (10.77 and 18.12 kHz; the zeros of the 5th-order Legendre polynomial, which minimise the
  in-band noise; Schreier and Temes, *Understanding Delta-Sigma Data Converters*, Wiley-IEEE
  2005, sec. 4.3);
- NTF poles: a 5th-order Butterworth high-pass whose corner is chosen so that the largest
  gain of the NTF is 1.3 (Lee's rule, which asks for 1.5 or less, for stable 1-bit
  modulators; Schreier and Temes, sec. 4.4.1). At 1.5 this loop was unstable above 0 dB
  SACD; at 1.3 it is stable with 1 kHz and 20 kHz sines up to at least 75 % modulation
  (+3.5 dB SACD), which covers the SACD peak level of 71.43 % (+3.1 dB SACD). The corner is
  then 72.9 kHz at DSD64;
- the NTF is monic (NTF(inf) = 1), as error feedback requires.

This is the class of modulator SACD uses (E. Janssen, *Look-ahead sigma-delta modulation
and its application to Super Audio CD*, TU Eindhoven, 2010, doi:10.6100/IR691188), but a
plain one, not a look-ahead one. Inputs are given relative to 100 % modulation (1.0);
0 dB SACD is 50 % modulation (0.5), the Scarlet Book reference level, and SACD allows peaks
to +3 dB SACD (about 71 %). The modulator is deterministic (IEEE-754 doubles, no random
numbers) and pure Python.
"""

import math

import numpy as np
from scipy import signal

FS = 2_822_400
BAND = 20_000.0
MAX_GAIN = 1.3
ORDER = 5


def _ntf_for(wn, fs):
    zeros = [1.0]
    for r in (0.5385, 0.9062):
        w = 2 * math.pi * r * BAND / fs
        zeros += [complex(math.cos(w), math.sin(w)), complex(math.cos(w), -math.sin(w))]
    _, poles, _ = signal.butter(ORDER, wn, "high", output="zpk")
    return np.array(zeros), poles


def _max_gain(z, p):
    w = np.linspace(0, math.pi, 1 << 16)
    _, h = signal.freqz_zpk(z, p, 1.0, worN=w)
    return float(np.max(np.abs(h)))


def design(fs=None, max_gain=None):
    """The NTF at ``fs`` (default FS) with the largest gain ``max_gain`` (default
    MAX_GAIN), as (zeros, poles, b, a, corner_hz); b and a are monic polynomials in z^-1."""
    fs = fs or FS
    max_gain = max_gain or MAX_GAIN
    lo, hi = 1e-4, 0.9
    for _ in range(80):
        mid = 0.5 * (lo + hi)
        z, p = _ntf_for(mid, fs)
        if _max_gain(z, p) > max_gain:
            hi = mid
        else:
            lo = mid
    z, p = _ntf_for(lo, fs)
    b = np.real(np.poly(z))
    a = np.real(np.poly(p))
    return z, p, b, a, lo * fs / 2


class Modulator:
    """The modulator with its state, fed in chunks: ``process(x)`` returns the bits (uint8
    0/1) for the input samples ``x`` (floats, 1.0 = 100 % modulation) and keeps the state.

    Raises RuntimeError if the loop's quantiser input exceeds ``limit`` (instability).
    """

    def __init__(self, b, a, limit=8.0):
        self.c = [float(bk - ak) for bk, ak in zip(b[1:], a[1:])]
        self.d = [float(ak) for ak in a[1:]]
        self.e = [0.0] * 5
        self.r = [0.0] * 5
        self.limit = limit
        self.n = 0

    def process(self, x):
        c1, c2, c3, c4, c5 = self.c
        d1, d2, d3, d4, d5 = self.d
        e1, e2, e3, e4, e5 = self.e
        r1, r2, r3, r4, r5 = self.r
        limit = self.limit
        out = bytearray(len(x))
        for n, xn in enumerate(x.tolist() if hasattr(x, "tolist") else x):
            r = c1 * e1 + c2 * e2 + c3 * e3 + c4 * e4 + c5 * e5 - (
                d1 * r1 + d2 * r2 + d3 * r3 + d4 * r4 + d5 * r5
            )
            u = xn + r
            if u >= 0.0:
                v = 1.0
                out[n] = 1
            else:
                v = -1.0
            if u > limit or u < -limit:
                raise RuntimeError(f"modulator unstable at sample {self.n + n} (|u| = {abs(u):.1f})")
            e = v - u
            e5, e4, e3, e2, e1 = e4, e3, e2, e1, e
            r5, r4, r3, r2, r1 = r4, r3, r2, r1, r
        self.e = [e1, e2, e3, e4, e5]
        self.r = [r1, r2, r3, r4, r5]
        self.n += len(out)
        return np.frombuffer(bytes(out), dtype=np.uint8)


def modulate(x, b=None, a=None, limit=8.0):
    """Bits (uint8 0/1) for the input samples ``x`` in one call (see ``Modulator``)."""
    if b is None:
        _, _, b, a, _ = design()
    return Modulator(b, a, limit).process(x)
