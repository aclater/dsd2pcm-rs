#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""A reference linear-phase low-pass applied to decoded PCM (an analysis step, not a decoder).

    python postfilter.py --in DIR --out DIR --pass-hz 30000 --stop-hz 36000 [--atten-db 150]

Filters every WAV in --in with a Kaiser-windowed linear-phase FIR (designed with
scipy.signal.kaiserord/firwin for the given edges and stopband), in 64-bit floats, delay
removed, and writes 32-bit float WAVs of the same names to --out. Used to compare a
decoder's own 30 kHz filter with a separate high-quality 30 kHz low-pass applied to a
wide-band decode, as Archimago did with iZotope RX in 2015.
"""

import argparse
import json
import os

import numpy as np
from scipy import signal as sps

import dsdio


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--in", dest="inp", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--pass-hz", type=float, default=30000)
    ap.add_argument("--stop-hz", type=float, default=36000)
    ap.add_argument("--atten-db", type=float, default=150)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    info = None
    for fn in sorted(os.listdir(a.inp)):
        if not fn.endswith(".wav"):
            continue
        x, fs, _ = dsdio.read_wav(os.path.join(a.inp, fn))
        n, beta = sps.kaiserord(a.atten_db, (a.stop_hz - a.pass_hz) / (fs / 2))
        n |= 1
        h = sps.firwin(n, (a.pass_hz + a.stop_hz) / 2, window=("kaiser", beta), fs=fs)
        d = (n - 1) // 2
        y = np.stack([sps.fftconvolve(x[:, c], h)[d:d + len(x)] for c in range(x.shape[1])], 1)
        dsdio.write_wav_float(os.path.join(a.out, fn), y, fs)
        info = {"taps": int(n), "kaiser_beta": float(beta), "pass_hz": a.pass_hz, "stop_hz": a.stop_hz, "atten_db": a.atten_db, "fs": fs}
    json.dump(info, open(os.path.join(a.out, "postfilter.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
