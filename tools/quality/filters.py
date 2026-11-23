#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""The filter itself: the composite response of a decimator's stage chain, from its taps.

    python filters.py --tables PATH/tables.rs [--pysacd PATH/pysacd_taps.rs] --out filters.json

Reads the integer taps of dsd2pcm's default filter (`tables.rs`, copied from
scarletbook-pcm 0.7.0) and, optionally, the taps of the pysacd preset (`pysacd_taps.rs`), and
computes for every rate and cutoff, with the rounding between stages ignored (the
end-to-end analysis covers it):

- the composite impulse response at 2.8224 MHz (each stage upsampled to that rate and
  convolved; multistage decimation is equivalent to this one filter followed by keeping
  every d-th sample);
- the magnitude response (2^22-point FFT, 0.67 Hz bins, to 1.4112 MHz);
- passband ripple (to 20 kHz and to the passband edge), the -0.1, -1 and -3 dB points;
- stopband attenuation (the smallest from the stopband edge to 1.4112 MHz) and the worst
  alias: the largest response at any frequency that folds into 0-20 kHz, or into the
  passband, of the output rate; for 352.8 kHz output of the 30 and 40 kHz filters, which
  interpolate from 176.4 kHz, also the images of the interpolation;
- the impulse response at the output rate (phase 0), its length above -60 and -120 dB of
  the peak before and after the peak, the step response's overshoot, and the delay.
"""

import argparse
import json
import re
import struct

import numpy as np

FS = 2_822_400
NFFT = 1 << 22


def parse_tables(path):
    src = open(path).read()
    out = {}
    for m in re.finditer(r"const (\w+): \[i32; (\d+)\] = \[(.*?)\];", src, re.S):
        out[m.group(1)] = np.array([int(v) for v in re.findall(r"-?\d+", m.group(3))], np.float64)
        assert len(out[m.group(1)]) == int(m.group(2))
    return out


def parse_pysacd(path):
    src = open(path).read()
    out = {}
    for m in re.finditer(r"const (H\d): \[(u64|u32); (\d+)\] = \[(.*?)\];", src, re.S):
        vals = [int(v, 16) for v in re.findall(r"0x[0-9a-f]+", m.group(4))]
        if m.group(2) == "u64":
            out[m.group(1)] = np.array([struct.unpack("<d", struct.pack("<Q", v))[0] for v in vals])
        else:
            out[m.group(1)] = np.array([struct.unpack("<f", struct.pack("<I", v))[0] for v in vals], np.float64)
    return out


def full(half):
    """Symmetric taps from the centre out -> the whole filter."""
    return np.concatenate([half[:0:-1], half])


def up(h, k):
    if k == 1:
        return h
    y = np.zeros((len(h) - 1) * k + 1)
    y[::k] = h
    return y


def chain(stages):
    """stages: [(taps, upsampling factor to 2.8224 MHz)] -> the composite at 2.8224 MHz."""
    h = np.array([1.0])
    for taps, k in stages:
        h = np.convolve(h, up(taps / taps.sum(), k))
    return h


def plans(t):
    s1, s1w, hb, f40, f30, f80 = (full(t[k]) for k in ("S1", "S1_WIDE", "HB", "F40", "F30", "F80"))
    dc = {k: float(full(t[k]).sum() / (2**26 if k.startswith("S1") else 2**24)) for k in t}
    p = {}
    for cut, f in (("40", f40), ("30", f30)):
        for rate in (88200, 176400):
            p[(rate, cut)] = {"stages": [(s1, 1), (hb, 8), (f, 16)], "interp": None}
        p[(352800, cut)] = {"stages": [(s1, 1), (hb, 8), (f, 16)], "interp": hb}
    p[(176400, "wide")] = {"stages": [(s1w, 1), (f80, 8)], "interp": None}
    p[(352800, "wide")] = {"stages": [(s1w, 1), (f40, 8)], "interp": None}
    return p, dc


def pysacd_plans(t):
    h1, h2, h3 = t["H1"], t["H2"], t["H3"]
    dc = {"H1": float(h1.sum()), "H2": float(h2.sum()), "H3": float(h3.sum())}
    return {
        (88200, "pysacd"): {"stages": [(h1, 1), (h2, 8), (h3, 16)], "interp": None},
        (176400, "pysacd"): {"stages": [(h1, 1), (h2, 8)], "interp": None},
    }, dc


EDGES = {"40": (40000, 48000), "30": (30000, 36000), "pysacd": (40000, 48000)}


def analyse(rate, cut, plan):
    h = chain(plan["stages"])
    d = FS // rate if plan["interp"] is None else FS // 176400
    H = np.abs(np.fft.rfft(h, NFFT))
    f = np.fft.rfftfreq(NFFT, 1 / FS)
    db = 20 * np.log10(np.maximum(H, 1e-300))
    if plan["interp"] is not None:
        # 352.8 kHz from 176.4 kHz: the interpolator's response in the passband.
        hi = chain([(plan["interp"], 8)])
        Hi = np.abs(np.fft.rfft(hi, NFFT))
        dbo = db + 20 * np.log10(np.maximum(Hi, 1e-300))
    else:
        dbo = db
    if cut == "wide":
        pas, stop = 80000, (88200 if rate == 176400 else 96000)
    else:
        pas, stop = EDGES[cut]
    if cut == "pysacd" and rate == 176400:
        pas, stop = 60000, 88200
    r20 = dbo[(f >= 0) & (f <= 20000)]
    rpe = dbo[(f >= 0) & (f <= pas)]

    def cross(level):
        i = np.argmax(dbo < level)
        return float(f[i])

    sb = db[(f >= stop) & (f <= FS / 2)]
    out_rate = FS // d
    # Aliases: every frequency above the output Nyquist that folds into a band.
    fold = np.abs(((f + out_rate / 2) % out_rate) - out_rate / 2)
    above = f > out_rate / 2
    al20 = db[above & (fold <= 20000)]
    alpb = db[above & (fold <= pas)]
    res = {
        "rate": rate, "cutoff": cut, "pass_hz": pas, "stop_hz": stop,
        "dc_gain_db": float(dbo[0]),
        "ripple_20k_db": float((r20.max() - r20.min()) / 2),
        "ripple_pass_db": float((rpe.max() - rpe.min()) / 2),
        "max_dev_20k_db": float(np.abs(r20).max()),
        "max_dev_pass_db": float(np.abs(rpe).max()),
        "minus0_1_hz": cross(-0.1), "minus1_hz": cross(-1.0), "minus3_hz": cross(-3.0),
        "stopband_db": float(-sb.max()),
        "worst_alias_20k_db": float(-al20.max()),
        "worst_alias_pass_db": float(-alpb.max()),
        "taps_2m8": int(len(h)),
    }
    if plan["interp"] is not None:
        # Images of the 176.4 kHz signal around 176.4 kHz, weighted by the interpolator.
        fi = f[(f > 88200) & (f < 176400)]
        img = np.interp(176400 - fi, f, db) + np.interp(fi, f, 20 * np.log10(np.maximum(Hi, 1e-300)))
        res["images_db"] = float(-img.max())
    # Output-rate impulse (phase 0: the output sample centred on the impulse) and step.
    c = (len(h) - 1) // 2
    hp = h[int(np.argmax(np.abs(h))) % d::d]
    peak = np.argmax(np.abs(hp))
    lev = 20 * np.log10(np.maximum(np.abs(hp) / np.abs(hp[peak]), 1e-300))
    above60 = np.nonzero(lev > -60)[0]
    above120 = np.nonzero(lev > -120)[0]
    res["pre_ring_60_samples"] = int(peak - above60[0])
    res["post_ring_60_samples"] = int(above60[-1] - peak)
    # Ringing in time, from the composite at 2.8224 MHz (independent of the output phase):
    # how long before and after the centre the response stays above -60 / -120 dB.
    ctr = (len(h) - 1) / 2
    for lv in (60, 120):
        idx = np.nonzero(np.abs(h) > np.abs(h).max() * 10 ** (-lv / 20))[0]
        res[f"pre_ring_{lv}_ms"] = float((ctr - idx[0]) / FS * 1e3)
        res[f"post_ring_{lv}_ms"] = float((idx[-1] - ctr) / FS * 1e3)
    res["symmetric"] = bool(np.allclose(h, h[::-1], rtol=0, atol=1e-15 * np.abs(h).max()))
    step = np.cumsum(h)
    res["step_overshoot_pct"] = float((step.max() - step[-1]) / step[-1] * 100)
    res["latency_ms"] = c / FS * 1e3
    if plan["interp"] is not None:
        res["latency_ms"] += (len(plan["interp"]) - 1) / 2 / 352800 * 1e3
    # Curves for the plots (decimated; magnitude to 400 kHz, the impulse at 2.8224 MHz).
    # Each point is the largest response in a 50 Hz bucket (75 bins), to 400 kHz.
    sel = f <= 400000
    nb = 75
    m = int(sel.sum()) // nb * nb
    fb = f[:m].reshape(-1, nb).mean(1)
    db_b = dbo[:m].reshape(-1, nb).max(1)
    res["curve"] = {"f": fb.round(1).tolist(), "db": np.maximum(db_b, -200).round(5).tolist()}
    w = int(0.0015 * FS)
    seg = h[max(0, c - w): c + w + 1] / np.abs(h).max()
    res["impulse_2m8"] = {"t0_us": -min(w, c) / FS * 1e6, "dt_us": 1e6 / FS, "v": seg.round(8).tolist()}
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tables", required=True, help="dsd2pcm's crates/dsd2pcm/src/decimator/tables.rs")
    ap.add_argument("--pysacd", help="dsd2pcm's crates/dsd2pcm/src/filters/pysacd_taps.rs")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    p, dc = plans(parse_tables(a.tables))
    out = {"stage_dc_gain": dc, "filters": []}
    for (rate, cut), plan in sorted(p.items()):
        out["filters"].append(analyse(rate, cut, plan))
    if a.pysacd:
        pp, pdc = pysacd_plans(parse_pysacd(a.pysacd))
        out["pysacd_stage_dc_gain"] = pdc
        for (rate, cut), plan in sorted(pp.items()):
            out["filters"].append(analyse(rate, cut, plan))
    json.dump(out, open(a.out, "w"))
    for r in out["filters"]:
        print({k: (round(v, 5) if isinstance(v, float) else v) for k, v in r.items() if k not in ("curve", "impulse_2m8")})


if __name__ == "__main__":
    main()
