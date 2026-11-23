#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
# Portions Copyright (c) 2026 The scarletbook-rs authors (remez, halfband and the idle factor,
# ported from scarletbook-pcm 0.7.0 src/design.rs), MIT OR Apache-2.0; see PROVENANCE.md.
"""Designs the taps of the `precise` filter (`Filter::Precise`) and writes
crates/dsd2pcm/src/decimator/tables_precise.rs.

    python3 tools/filters/design_precise.py            # check the committed tables
    python3 tools/filters/design_precise.py --write    # rewrite them
    python3 tools/filters/design_precise.py --report   # print every stage's response

Requirements: tools/filters/requirements.txt (NumPy only). The output was generated on
x86-64 Linux with the pinned NumPy; the check compares the tables exactly.

Method (see docs/design.md, "The precise filter"):

- Every stage is a linear-phase (type I, odd-length, symmetric) FIR designed by the
  Parks-McClellan method, the Remez exchange with barycentric Lagrange interpolation
  (Parks & McClellan 1972; Rabiner, McClellan & Parks 1975). `remez` is a NumPy port of
  scarletbook-pcm 0.7.0's `src/design.rs` (Copyright (c) 2026 The scarletbook-rs authors,
  MIT OR Apache-2.0), as are `halfband` (Vaidyanathan & Nguyen 1987) and the idle-pattern
  factor `(1 + z^-1)^2 (1 + z^-4)` of the first stage, which converts the Super Audio CD
  idle pattern to exact zero.
- S1, the only stage that decimates from the bit stream, needs the whole stopband on its
  own. It is a multiband design: deep stopbands only where the input folds into what the
  later stages keep (k x 352.8 kHz +- 48 kHz, or +- 96 kHz for `wide`), don't-care
  elsewhere (the later stages remove it). Its taps are scaled by 2^29: its sums still fit
  in 32 bits.
- Every later stage is a cascade of two filters with about 80 dB each, so that a stopband
  of 150 dB and more needs only about 22-bit taps, which keeps every partial sum of the
  exact-integer pipeline below 2^53 (`exact.rs`); one 150 dB filter would need about
  28-bit taps, which does not fit with 26-bit samples. A short prefilter `P` at 352.8 kHz
  takes S1's 2^29 grid back to 2^26 and adds 80 dB in front of the half-band.
- Taps are rounded to integers summing exactly to 2^q (unit gain at DC), then improved by
  a greedy integer search that moves one symmetric tap pair at a time (with the centre
  tap, so that the sum is kept) while the largest stopband error falls and the passband
  stays within its ripple.
"""

import argparse
import os
import sys

import numpy as np

FS = 2_822_400.0
R352 = FS / 8
R176 = FS / 16
GRID = 26  # samples between stages: 100 % modulation is 2^26
Q_S1 = 29
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..",
                   "crates", "dsd2pcm", "src", "decimator", "tables_precise.rs")


def rad(f, fs):
    return 2.0 * np.pi * f / fs


# ---------------------------------------------------------------------------------------
# Remez exchange (port of scarletbook-pcm 0.7.0 src/design.rs `remez`)


def cos_diff(a, b):
    """cos a - cos b, accurate when a and b are close."""
    return -2.0 * np.sin((a + b) / 2.0) * np.sin((a - b) / 2.0)


def remez(w, d, wt, band, n, iters=300):
    """Best weighted Chebyshev approximation of `d` on the grid `w` by
    A(w) = sum_{k<n} a_k cos(k w); returns (a, |delta|)."""
    r = n + 1
    npts = len(w)
    ext = np.round(np.linspace(0, npts - 1, r)).astype(int)
    s = np.where(np.arange(r) % 2 == 0, 1.0, -1.0)
    same_l = np.r_[False, band[1:] == band[:-1]]
    same_r = np.r_[band[:-1] == band[1:], False]
    for _ in range(iters):
        x = w[ext]
        dd = cos_diff(x[:, None], x[None, :])
        np.fill_diagonal(dd, 1.0)
        logs = -np.log(np.abs(dd)).sum(axis=1)
        b = np.prod(np.sign(dd), axis=1) * np.exp(logs - logs.max())
        delta = np.sum(b * d[ext]) / np.sum(b * s / wt[ext])
        c = d[ext[:-1]] - s[:-1] * delta / wt[ext[:-1]]
        dw = b[:-1] * cos_diff(x[:-1], x[-1])
        xs = x[:-1]

        def interp(wv, xs=xs, c=c, dw=dw):
            wv = np.atleast_1d(wv)
            diff = cos_diff(wv[:, None], xs[None, :])
            hit = diff == 0
            diff[hit] = 1.0
            t = dw[None, :] / diff
            out = (t * c[None, :]).sum(1) / t.sum(1)
            rows, cols = np.nonzero(hit)
            out[rows] = c[cols]
            return out

        err = wt * (d - interp(w))
        err[ext] = s * delta
        el = np.r_[0.0, err[:-1]]
        er = np.r_[err[1:], 0.0]
        is_ext = np.where(err > 0,
                          (~same_l | (err >= el)) & (~same_r | (err >= er)),
                          (~same_l | (err <= el)) & (~same_r | (err <= er)))
        cand = np.nonzero(is_ext & (np.abs(err) >= abs(delta) * (1 - 1e-9)))[0]
        if len(cand) < r:
            cand = np.union1d(np.nonzero(is_ext)[0], ext)
        alt = []
        for j in cand:
            if alt and (err[alt[-1]] > 0) == (err[j] > 0):
                if abs(err[j]) > abs(err[alt[-1]]):
                    alt[-1] = j
            else:
                alt.append(j)
        while len(alt) > r:
            if abs(err[alt[0]]) < abs(err[alt[-1]]):
                alt.pop(0)
            else:
                alt.pop()
        if len(alt) < r:
            raise RuntimeError("remez: lost alternation")
        alt = np.array(alt)
        done = np.array_equal(alt, ext) or np.abs(err[alt]).max() - abs(delta) <= 1e-9 * abs(delta)
        ext = alt
        if done:
            break
    big = n - 1
    jj = np.arange(n)
    samples = interp(np.pi * jj / big)
    half = np.where((jj == 0) | (jj == n - 1), 0.5, 1.0)
    k = np.arange(n)
    a = (half[None, :] * samples[None, :] * np.cos(np.pi * np.outer(k, jj) / big)).sum(1)
    a *= np.where((k == 0) | (k == n - 1), 1.0, 2.0) / big
    return a, abs(delta)


def grid(bands, n, density=24):
    """bands: (lo, hi, desired(w), weight(w)); points where the weight is ~0 are dropped."""
    total = sum(b[1] - b[0] for b in bands)
    w, d, wt, band = [], [], [], []
    for i, (lo, hi, df, wf) in enumerate(bands):
        pts = max(int(np.ceil((hi - lo) / total * density * n)), 16)
        x = np.linspace(lo, hi, pts + 1)
        ww = wf(x)
        keep = ww > 1e-6
        x = x[keep]
        w.append(x)
        d.append(df(x))
        wt.append(ww[keep])
        band.append(np.full(len(x), i))
    return [np.concatenate(v) for v in (w, d, wt, band)]


def symmetric(a):
    """Cosine coefficients a_k -> taps from -n+1 to n-1."""
    return np.r_[a[:0:-1] / 2, a[0], a[1:] / 2]


def idle_q(w):
    """Amplitude of (1 + z^-1)^2 (1 + z^-4), centred: 2 cos 2w (2 + 2 cos w)."""
    return 2.0 * np.cos(2.0 * w) * (2.0 + 2.0 * np.cos(w))


IDLE_F = np.array([1, 2, 1, 0, 1, 2, 1], dtype=np.int64)


def s1_cofactor(m, fp, fe, ds_db, dp):
    """The cofactor G of S1 = F G (half length m - 3; S1 has taps -m..m): passband
    [0, fp] with ripple weight 1, stopbands k fs/8 +- fe with weight dp/ds."""
    n = m - 2
    k = dp / 10 ** (-ds_db / 20)
    bands = [(0.0, rad(fp, FS), lambda w: 1 / idle_q(w), lambda w: np.abs(idle_q(w)))]
    for j in range(1, 5):
        c = j * R352
        bands.append((rad(c - fe, FS), rad(min(c + fe, FS / 2), FS),
                      lambda w: np.zeros_like(w), lambda w, k=k: k * np.abs(idle_q(w))))
    a, _ = remez(*grid(bands, n), n)
    return symmetric(a)


def lowpass(m, wp, ws, ds_db, dp):
    k = dp / 10 ** (-ds_db / 20)
    bands = [(0.0, wp, np.ones_like, np.ones_like),
             (ws, np.pi, np.zeros_like, lambda w: np.full_like(w, k))]
    a, _ = remez(*grid(bands, m + 1), m + 1)
    return symmetric(a)


def halfband(kk, wp):
    """Half-band of length 4K - 1 (Vaidyanathan & Nguyen), passband [0, wp]."""
    bands = [(0.0, 2 * wp, lambda w: 1 / np.cos(w / 2), lambda w: np.cos(w / 2))]
    p, _ = remez(*grid(bands, kk), kk)
    c = np.zeros(kk + 1)
    for i, pi in enumerate(p):
        c[i + 1] += pi / 2
        c[1 if i == 0 else i] += pi / 2
    half = 2 * kk - 1
    h = np.zeros(2 * half + 1)
    h[half] = 0.5
    for j in range(1, kk + 1):
        off = 2 * j - 1
        h[half - off] = h[half + off] = c[j] / 4
    return h


# ---------------------------------------------------------------------------------------
# Integer taps


def cos_matrix(w, n):
    c = np.cos(np.outer(w, np.arange(n)))
    c[:, 1:] *= 2
    return c


def amp_half(v, w):
    """Amplitude of a symmetric filter from its taps centre-out."""
    return cos_matrix(w, len(v)) @ np.asarray(v, dtype=float)


def round_to(h_half, total, halfband_=False):
    """Centre-out taps scaled to sum to `total` and rounded; the centre takes the
    difference (a half-band keeps its centre at total / 2 and its even taps at 0)."""
    h_half = np.asarray(h_half, float)
    dc = h_half[0] + 2 * h_half[1:].sum()
    v = np.round(h_half * total / dc).astype(np.int64)
    if halfband_:
        v[0] = total // 2
        v[2::2] = 0
        diff = total - (v[0] + 2 * v[1:].sum())
        assert diff % 2 == 0
        v[1] += diff // 2
    else:
        v[0] += total - (v[0] + 2 * v[1:].sum())
    return v


def improve(v, w_stop, w_pass, dp, scale, outer=None, halfband_=False, iters=400):
    """Greedy integer search: moves tap pair k by +-1 (and the centre by -+2, or for a
    half-band the pair next to the centre by -+1) while the largest stopband error falls
    and the passband stays within `dp` of 1."""
    v = v.copy()
    cs = cos_matrix(w_stop, len(v)) / scale
    cp = cos_matrix(w_pass, len(v)) / scale
    if outer is not None:
        cs *= outer(w_stop)[:, None]
        cp *= outer(w_pass)[:, None]
    if halfband_:
        ks = [k for k in range(3, len(v), 2)]
        comp = 1  # the pair at offset 1 takes -d
        f = 1
    else:
        ks = list(range(1, len(v)))
        comp = 0
        f = 2
    a_s = cs @ v
    a_p = cp @ v
    cur = np.abs(a_s).max()
    dp = max(dp, np.abs(a_p - 1).max())
    ks = np.array(ks)
    ds_ = cs[:, ks] - f * cs[:, [comp]]
    dp_ = cp[:, ks] - f * cp[:, [comp]]
    for _ in range(iters):
        best = (cur, None)
        for d in (1, -1):
            stop = np.abs(a_s[:, None] + d * ds_).max(0)
            pas = np.abs(a_p[:, None] + d * dp_ - 1).max(0)
            stop[pas > dp] = np.inf
            i = int(np.argmin(stop))
            if stop[i] < best[0]:
                best = (stop[i], (i, d))
        if best[1] is None:
            break
        i, d = best[1]
        v[ks[i]] += d
        v[comp] -= f * d
        a_s = a_s + d * ds_[:, i]
        a_p = a_p + d * dp_[:, i]
        cur = best[0]
    return v


def full(v):
    return np.r_[v[:0:-1], v]


# ---------------------------------------------------------------------------------------
# Stages


def stop_grid(bands, fs, pts=3000):
    return np.concatenate([np.linspace(rad(lo, fs), rad(hi, fs), pts) for lo, hi in bands])


def s1_stops(fe):
    return [(j * R352 - fe, min(j * R352 + fe, FS / 2)) for j in range(1, 5)]


def design_s1(m, fp, fe):
    g = s1_cofactor(m, fp, fe, 170.0, 5e-5)
    mg = len(g) // 2
    v = round_to(g[mg:], 1 << (Q_S1 - 3))
    ws = stop_grid(s1_stops(fe), FS)
    ws = ws[np.abs(idle_q(ws)) > 1e-9]
    wp = np.linspace(0, rad(fp, FS), 400)
    v = improve(v, ws, wp, 2e-6, float(1 << Q_S1), outer=idle_q)
    h = np.convolve(full(v), IDLE_F)
    return h[len(h) // 2:]


def design_lowpass(fp, fstop, fs, q, ds_db=78.0, dp=1e-4, even=False):
    """The shortest lowpass whose integer taps reach `ds_db` with ripple at most `dp`; with
    `even`, of even half length (a stage after an interpolation by 2 must keep its inputs'
    first index even: `Op::need`)."""
    ws = stop_grid([(fstop, fs / 2)], fs)
    wp = np.linspace(0, rad(fp, fs), 600)
    for m in range(4, 129):
        if even and m % 2:
            continue
        try:
            h = lowpass(m, rad(fp, fs), rad(fstop, fs), ds_db + 1.0, dp * 0.9)
        except RuntimeError:
            continue
        v = round_to(h[m:], 1 << q)
        if np.abs(amp_half(v / 2**q, wp) - 1).max() > dp:
            continue
        v = improve(v, ws, wp, dp, float(1 << q))
        if -20 * np.log10(np.abs(amp_half(v / 2**q, ws)).max()) >= ds_db:
            return v
    raise RuntimeError("no lowpass found")


def design_halfband(kk, fp, fs, q):
    h = halfband(kk, rad(fp, fs))
    half = len(h) // 2
    v = round_to(h[half:], 1 << q, halfband_=True)
    v = improve(v, stop_grid([(fs / 2 - fp, fs / 2)], fs), np.linspace(0, rad(fp, fs), 400),
                3e-5, float(1 << q), halfband_=True)
    return v


# Stage parameters: (name, doc, q).
def design():
    t = {}
    t["S1"] = (design_s1(47, 40e3, 48e3), Q_S1,
               "First stage of the 40 and 30 kHz filters: 2822.4 kHz, passband 40 kHz, "
               "stopbands k x 352.8 kHz +- 48 kHz, factor (1 + z^-1)^2 (1 + z^-4).")
    t["S1_WIDE"] = (design_s1(63, 80e3, 96e3), Q_S1,
                    "First stage of the wide filters: 2822.4 kHz, passband 80 kHz, "
                    "stopbands k x 352.8 kHz +- 96 kHz, factor (1 + z^-1)^2 (1 + z^-4).")
    t["P"] = (design_lowpass(40e3, R176 - 48e3, R352, 22, ds_db=84.0, dp=2e-5, even=True), 22,
              "Prefilter at 352.8 kHz: passband 40 kHz, stopband from 128.4 kHz (also after the "
              "interpolation to 352.8 kHz, against its images).")
    t["HB"] = (design_halfband(6, 48e3, R352, 22), 22,
               "Half-band at 352.8 kHz: passband 48 kHz, stopband from 128.4 kHz.")
    t["F40"] = (design_lowpass(40e3, 48e3, R176, 22), 22,
                "Sharp filter at 176.4 kHz, used twice: passband 40 kHz, stopband from 48 kHz "
                "(also 80/96 kHz at 352.8 kHz).")
    t["F30"] = (design_lowpass(30e3, 36e3, R176, 22), 22,
                "Sharp filter at 176.4 kHz, used twice: passband 30 kHz, stopband from 36 kHz.")
    t["F80"] = (design_lowpass(80e3, 88.2e3, R352, 21), 21,
                "Wide filter at 352.8 kHz for 176.4 kHz output, used twice: passband 80 kHz, "
                "stopband from 88.2 kHz.")
    return t


# ---------------------------------------------------------------------------------------
# Checks: responses and the exactness bounds of exact.rs


def sum_abs(v):
    return abs(int(v[0])) + 2 * sum(abs(int(x)) for x in v[1:])


# Plans: per (cutoff, rate), S1 and the ops (kind, table, input grid exponent).
PLANS = {
    ("30/40", 88200): ("S1", [("fir1", "P", Q_S1), ("dec2", "HB", GRID), ("fir1", "F", GRID), ("dec2", "F", GRID)]),
    ("30/40", 176400): ("S1", [("fir1", "P", Q_S1), ("dec2", "HB", GRID), ("fir1", "F", GRID), ("fir1", "F", GRID)]),
    ("30/40", 352800): ("S1", [("fir1", "P", Q_S1), ("dec2", "HB", GRID), ("fir1", "F", GRID), ("fir1", "F", GRID),
                               ("interp2", "HB", GRID), ("fir1", "P", GRID)]),
    ("wide", 176400): ("S1_WIDE", [("fir1", "F80", Q_S1), ("dec2", "F80", GRID)]),
    ("wide", 352800): ("S1_WIDE", [("fir1", "F40", Q_S1), ("fir1", "F40", GRID)]),
}


def bounds(t):
    """The bounds of exact.rs for every plan: the largest partial sum per stage (< 2^53)."""
    out = []
    for (cut, rate), (s1, ops) in PLANS.items():
        for f in (["F40", "F30"] if cut == "30/40" else [None]):
            x = sum_abs(t[s1][0])
            assert x < 2**31, "S1 sums in 32 bits"
            assert 2 * x < 2**32, "paired S1 halves"
            row = [f"{cut if f is None else f[1:]} {rate}: S1 {np.log2(x):.2f}"]
            for kind, name, gin in ops:
                name = f if name == "F" else name
                v, q, _ = t[name]
                shiftq = q + gin - GRID
                if kind == "interp2":
                    worst = 2 * sum(abs(int(h)) for h in v[1::2]) * x
                    x = max((worst >> (shiftq - 1)) + 1, x)
                else:
                    worst = sum_abs(v) * x
                    x = (worst >> shiftq) + 1
                assert worst < 2**53, f"{row[0]} {name}: {np.log2(worst):.2f}"
                row.append(f"{name} {np.log2(worst):.2f}")
            assert 16 * x < 2**53
            out.append(", ".join(row))
    return out


def report(t):
    lines = []
    for name, (v, q, _) in t.items():
        h = np.asarray(v, float) / 2**q
        if name.startswith("S1"):
            fe = 48e3 if name == "S1" else 96e3
            fp = 40e3 if name == "S1" else 80e3
            st = [-20 * np.log10(np.abs(amp_half(h, stop_grid([b], FS, 20000))).max()) for b in s1_stops(fe)]
            rp = np.abs(amp_half(h, np.linspace(0, rad(fp, FS), 4000)) - 1).max()
            lines.append(f"{name}: {2*len(v)-1} taps, q {q}, stopbands " +
                         " ".join(f"{s:.1f}" for s in st) + f" dB, ripple {rp:.1e}")
            continue
        fs, fp, fstop = {"P": (R352, 40e3, R176 - 48e3), "HB": (R352, 48e3, R176 - 48e3),
                         "F40": (R176, 40e3, 48e3), "F30": (R176, 30e3, 36e3),
                         "F80": (R352, 80e3, 88.2e3)}[name]
        st = -20 * np.log10(np.abs(amp_half(h, stop_grid([(fstop, fs / 2)], fs, 40000))).max())
        rp = np.abs(amp_half(h, np.linspace(0, rad(fp, fs), 4000)) - 1).max()
        lines.append(f"{name}: {2*len(v)-1} taps, q {q}, stopband {st:.1f} dB from "
                     f"{fstop/1e3:.1f} kHz, ripple {rp:.1e}")
    return lines


def tables_rs(t):
    s = ["// SPDX-License-Identifier: MIT OR Apache-2.0",
         "",
         "// @generated by tools/filters/design_precise.py (`--write`). Do not edit. Each table",
         "// holds the taps of one symmetric stage from the centre outwards: `[0]` is the centre",
         "// tap, `[k]` the taps at offsets -k and +k. `Q_*` is the tap scale: taps sum to 2^Q.",
         ""]
    for name, (v, q, doc) in t.items():
        s.append(f"/// The tap scale of [`{name}`]: its taps sum to 2^{q}.")
        s.append(f"pub(crate) const Q_{name}: i32 = {q};")
        s.append(f"/// {doc} Length {2 * len(v) - 1}, taps scaled by 2^{q}.")
        s.append("#[rustfmt::skip]")
        body = [f"pub(crate) const {name}: [i32; {len(v)}] = ["]
        for i in range(0, len(v), 8):
            body.append("    " + " ".join(f"{int(x)}," for x in v[i:i + 8]))
        body.append("];")
        s.extend(body)
        s.append("")
    return "\n".join(s).rstrip() + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--report", action="store_true")
    a = ap.parse_args()
    t = design()
    for name, (v, _, _) in t.items():
        assert all(-2**31 < int(x) < 2**31 for x in v), name
    b = bounds(t)
    if a.report:
        print("\n".join(report(t)))
        print("Largest partial sums (log2, limit 53):")
        print("\n".join("  " + x for x in b))
    text = tables_rs(t)
    if a.write:
        with open(OUT, "w") as f:
            f.write(text)
        print(f"wrote {os.path.normpath(OUT)}")
    elif not a.report:
        with open(OUT) as f:
            committed = f.read()
        if committed != text:
            print("tables_precise.rs differs from the generator's output", file=sys.stderr)
            return 1
        print("tables_precise.rs matches the generator")
    return 0


if __name__ == "__main__":
    sys.exit(main())
