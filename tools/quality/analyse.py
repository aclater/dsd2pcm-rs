#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Analyses decoded PCM: WAV files in, metrics and plot curves (JSON) out.

    python analyse.py --signals-dir SIG --decoded DEC --out results.json [--curves curves.json]

For each WAV under DEC (DEC/<decoder>/<setting>/<signal file>.wav) the signal is looked up
in SIG/signals.json and analysed by its id. The settings (ANALYSIS below) are written into
the output. Levels are dBFS (a full-scale sine is 0 dBFS) of the decoder's output; with
dsd2pcm at 0 dB gain, 0 dB SACD is -6.02 dBFS.
"""

import argparse
import json
import math
import os
from concurrent.futures import ProcessPoolExecutor

import numpy as np

import dsdio

ANALYSIS = {
    "window": "7-term Blackman-Harris (Albrecht 2001 coefficients) for spectra; rectangular for the coherent multitone",
    "fft": "2^18 points for tones (frequency response tones: 2^18 x fs / 88200, 0.34 Hz bins) (step signals: 2^18 x fs / 176400, 1.49 s), Welch 2^18 x fs / 88200 with 50 % overlap and 8 averages for noise, 2^20 (2^19 at 88.2 kHz) for the multitone and the J-Test",
    "settling": "steady signals: analysis from 0.5 s after the steady part starts; steps: from 0.25 s into each step; never the last 0.1 s",
    "bandwidth": "20 Hz to 20 kHz (AES17) unless stated; THD: harmonics 2 to 9 up to 20 kHz",
    "tone_lobe_bins": 8,
    "weighting": "A-weighting per IEC 61672-1, 0 dB at 1 kHz",
    "dbfs": "0 dBFS = a full-scale sine (RMS 1/sqrt(2)); powers are mean squares",
}

BH7 = [0.27105140069342, 0.43329793923448, 0.21812299954311, 0.06592544638803,
       0.01081174209837, 0.00077658482522, 0.00001388721735]
LOBE = ANALYSIS["tone_lobe_bins"]


def bh7(n):
    k = np.arange(n) * (2 * math.pi / n)
    return sum((-1) ** i * a * np.cos(i * k) for i, a in enumerate(BH7))


def power_spectrum(x, window="bh7"):
    """One-sided power per bin, normalised so that the bins of a sine sum to A^2 / 2 and
    noise bins sum to the mean square."""
    n = len(x)
    w = bh7(n) if window == "bh7" else np.ones(n)
    X = np.fft.rfft(x * w)
    p = np.abs(X) ** 2 * 2 / (n * np.sum(w**2))
    p[0] /= 2
    return p


def welch(x, n, avg=8):
    hop = n // 2
    segs = [x[i:i + n] for i in range(0, len(x) - n + 1, hop)][:avg]
    return np.mean([power_spectrum(s) for s in segs], axis=0), len(segs)


def dbfs(p):
    return 10 * math.log10(max(p, 1e-300) / 0.5)


def a_weight(f):
    f2 = np.asarray(f, float) ** 2
    ra = (12194.0**2 * f2**2) / ((f2 + 20.6**2) * np.sqrt((f2 + 107.7**2) * (f2 + 737.9**2)) * (f2 + 12194.0**2))
    with np.errstate(divide="ignore"):
        return (ra / 0.7943282347242815) ** 2  # power gain, 0 dB at 1 kHz


def lobe(f, f0, fs, n):
    k = int(round(f0 * n / fs))
    return slice(max(k - LOBE, 0), k + LOBE + 1)


def tone_metrics(x, fs, f0, n=None, band=(20.0, 20000.0)):
    n = n or len(x)
    x = x[:n]
    p = power_spectrum(x)
    f = np.fft.rfftfreq(n, 1 / fs)
    sl = lobe(f, f0, fs, n)
    pt = p[sl].sum()
    inb = (f >= band[0]) & (f <= band[1])
    rest = p.copy()
    rest[sl] = 0
    # Harmonics 2..9 up to the band edge.
    harm = 0.0
    for h in range(2, 10):
        if h * f0 > band[1]:
            break
        harm += p[lobe(f, h * f0, fs, n)].sum()
    wa = a_weight(f)
    noise = rest[inb].sum()
    noise_a = (rest * wa)[inb].sum()
    # Phase of the fitted tone (for channel matching).
    k = int(round(f0 * n / fs))
    t = np.arange(n) / fs
    c = np.exp(-2j * math.pi * f[k] * t)
    ph = float(np.angle(np.sum(x * c)))
    return {
        "hz": float(f[int(np.argmax(p[sl])) + sl.start]),
        "level_dbfs": dbfs(pt),
        "thd_db": 10 * math.log10(max(harm, 1e-300) / pt),
        "thdn_db": 10 * math.log10(max(noise, 1e-300) / pt),
        "thdn_a_db": 10 * math.log10(max(noise_a, 1e-300) / pt),
        "residual_dbfs": dbfs(noise),
        "residual_a_dbfs": dbfs(noise_a),
        "phase_rad": ph,
        "dc_dbfs": 20 * math.log10(max(abs(float(np.mean(x))), 1e-300)),
    }


def seg(sig, fs, start_s, n):
    a = int(round(start_s * fs))
    return sig[a:a + n] if a + n <= len(sig) else None


def logcurve(f, p, fmin=10.0, nb=500, mode="max", density=None):
    """A compact curve for plots: per log-spaced bin the max (tones) or mean (noise)."""
    fmax = f[-1]
    edges = np.geomspace(fmin, fmax, nb + 1)
    idx = np.searchsorted(f, edges)
    xs, ys = [], []
    for i in range(nb):
        a, b = idx[i], max(idx[i + 1], idx[i] + 1)
        if a >= len(p):
            break
        v = p[a:b].max() if mode == "max" else p[a:b].mean()
        if density:
            v = v / density
        xs.append(round(float(np.sqrt(edges[i] * edges[i + 1])), 2))
        ys.append(round(10 * math.log10(max(v, 1e-30) / (0.5 if not density else 1.0)), 3))
    return {"f": xs, "db": ys}


def lincurve(f, p, fmax, nb=800, density=None, mode="mean"):
    sel = f <= fmax
    f, p = f[sel], p[sel]
    step = max(1, len(f) // nb)
    m = len(f) // step * step
    pv = p[:m].reshape(-1, step).max(1) if mode == "max" else p[:m].reshape(-1, step).mean(1)
    fv = f[:m].reshape(-1, step).mean(1)
    if density:
        pv = pv / density
    return {"f": fv.round(1).tolist(), "db": (10 * np.log10(np.maximum(pv, 1e-30))).round(3).tolist()}


def tone_scale(p, window="bh7"):
    """Rescales noise-normalised bins so that a sine's peak bin reads its level (the usual
    scaling of published FFT plots): times the window's noise bandwidth in bins."""
    n = 2 * (len(p) - 1)
    w = bh7(n) if window == "bh7" else np.ones(n)
    return p * (n * np.sum(w**2) / np.sum(w) ** 2)


def bands(f, p, edges):
    out = {}
    for lo, hi in edges:
        out[f"{int(lo)}-{int(hi)}"] = dbfs(p[(f >= lo) & (f < hi)].sum())
    return out


def read_any(path):
    """A decoded WAV, or a DSD source (DSF) as +-1 samples at the DSD rate (for the
    source's own spectrum: the reference that the decoders are compared with)."""
    if path.endswith(".dsf"):
        chans, rate, n = dsdio.read_dsf(path)
        bits = np.unpackbits(chans, axis=1)[:, :n]
        return (bits.T.astype(np.float32) * 2 - 1), rate, {"format": "dsd", "bits": 1}
    return dsdio.read_wav(path)


def analyse_file(path, meta, key):
    x, fs, info = read_any(path)
    res = {"key": key, "fs": fs, "samples": len(x), "format": info, "signal": meta["id"]}
    if info["format"] == "int" and info["bits"] == 24:
        import hashlib
        res["pcm_sha256"] = hashlib.sha256(np.round(x * 2**23).astype("<i4").tobytes()).hexdigest()
    if fs > 400000:  # a DSD source: the same windows in time as at 176.4 kHz
        scale = fs // 176400
    else:
        scale = 1
    curves = {}
    full = 2**23
    res["clipped"] = int(np.sum(np.abs(x) >= (full - 1) / full)) if info["format"] == "int" else int(np.sum(np.abs(x) >= 1.0))
    res["peak_dbfs"] = 20 * math.log10(max(np.abs(x).max(), 1e-300))
    st = meta.get("steady_start", 0.0)
    sid = meta["id"]
    L, R = x[:, 0], x[:, 1]
    n18 = (1 << 18) * scale
    nstep = int((1 << 18) * fs / 176400)

    if sid == "idle":
        res["all_zero"] = bool(np.all(x == 0))
        res["max_abs"] = float(np.abs(x).max())
    elif sid == "silence":
        n = int((1 << 18) * fs / 88200)
        a = int(0.5 * fs)
        p, navg = welch(L[a:len(L) - int(0.1 * fs)], n)
        f = np.fft.rfftfreq(n, 1 / fs)
        inb = (f >= 20) & (f <= 20000)
        res["noise_dbfs"] = dbfs(p[inb].sum())
        res["noise_a_dbfs"] = dbfs((p * a_weight(f))[inb].sum())
        res["averages"] = navg
        edges = [(20000, 40000), (20000, 100000), (40000, 96000), (96000, 144000), (144000, 192000)]
        res["ultrasonic_dbfs"] = bands(f, p, [(lo, hi) for lo, hi in edges if hi <= fs / 2 + 1])
        res["ultrasonic_to_nyquist_dbfs"] = dbfs(p[f >= 20000].sum())
        us = (f > 20000)
        k = int(np.argmax(np.where(us, p, 0)))
        res["ultrasonic_peak"] = {"hz": float(f[k]), "dbfs_per_bin": dbfs(p[k])}
        res["dc_dbfs"] = 20 * math.log10(max(abs(float(np.mean(L))), 1e-300))
        w = bh7(n)
        enbw_hz = fs / n * n * np.sum(w**2) / np.sum(w) ** 2
        curves["noise_log"] = logcurve(f, p, 20, 400, "mean", density=enbw_hz)
        curves["noise_lin"] = lincurve(f, p, fs / 2, 700, density=enbw_hz)
    elif sid in ("sine1k_0", "sine1k_peak", "sine15k", "sine20k_0", "quiet_sine1k"):
        f0 = meta["channels"][0]["tones"][0]["hz"]
        s = seg(L, fs, st + 0.5, n18)
        r = tone_metrics(s, fs, f0)
        res.update(r)
        rr = tone_metrics(seg(R, fs, st + 0.5, n18), fs, f0)
        res["channel_level_diff_db"] = r["level_dbfs"] - rr["level_dbfs"]
        res["channel_phase_diff_deg"] = math.degrees(r["phase_rad"] - rr["phase_rad"])
        res["channels_identical"] = bool(np.array_equal(L, R))
        res["expected_dbfs"] = meta["channels"][0]["tones"][0]["dbfs"]
        res["gain_error_db"] = r["level_dbfs"] - res["expected_dbfs"]
        p = power_spectrum(s)
        f = np.fft.rfftfreq(len(s), 1 / fs)
        curves["spectrum_log"] = logcurve(f, tone_scale(p), 10, 600, "max")
        curves["spectrum_lin"] = lincurve(f, tone_scale(p), fs / 2, 900, mode="max")
        if sid == "sine15k":
            a = int(round((st + 1.0) * fs))
            curves["time"] = {"fs": fs, "v": L[a:a + int(0.001 * fs) + 1].tolist()}
        if sid in ("sine1k_0", "quiet_sine1k"):
            # Wider band THD+N (to 80 kHz where the rate allows) and ultrasonic noise with a tone.
            if fs >= 176400:
                res["thdn_80k_db"] = tone_metrics(s, fs, f0, band=(20.0, 80000.0))["thdn_db"]
    elif sid in ("level_steps", "freq_steps", "xtalk_l", "xtalk_r"):
        steps = meta["channels"][0].get("steps") or meta["channels"][1].get("steps")
        driven = 1 if sid == "xtalk_r" else 0
        out = []
        for stp in steps:
            a = st + stp["start"] + 0.25
            s = seg(x[:, driven], fs, a, nstep)
            if s is None:
                continue
            r = tone_metrics(s, fs, stp["hz"])
            r["expected_dbfs"] = stp["dbfs"]
            r["level_error_db"] = r["level_dbfs"] - stp["dbfs"]
            if fs >= 176400 and stp["hz"] >= 10000:
                r["thdn_80k_db"] = tone_metrics(s, fs, stp["hz"], band=(20.0, 80000.0))["thdn_db"]
            if sid.startswith("xtalk"):
                o = seg(x[:, 1 - driven], fs, a, nstep)
                po = power_spectrum(o)
                fo = np.fft.rfftfreq(len(o), 1 / fs)
                lev = dbfs(po[lobe(fo, stp["hz"], fs, len(o))].sum())
                r["other_channel_dbfs"] = lev
                r["crosstalk_db"] = lev - r["level_dbfs"]
                r["other_channel_all_zero"] = bool(np.all(o == 0))
            out.append({k: v for k, v in r.items() if k != "phase_rad"})
        res["steps"] = out
        if sid == "level_steps":
            # Dynamic range (AES17): the -60 dB SACD step is -66 dBFS here.
            near = min(out, key=lambda r: abs(r["expected_dbfs"] + 66))
            res["dr_step_dbfs"] = near["expected_dbfs"]
            res["dynamic_range_db"] = -(near["residual_dbfs"] + 6.02)
            res["dynamic_range_a_db"] = -(near["residual_a_dbfs"] + 6.02)
    elif sid == "smpte":
        s = seg(L, fs, st + 0.5, n18)
        p = power_spectrum(s)
        f = np.fft.rfftfreq(n18, 1 / fs)
        lo, hi = meta["channels"][0]["tones"][0]["hz"], meta["channels"][0]["tones"][1]["hz"]
        ph = p[lobe(f, hi, fs, n18)].sum()
        side = sum(p[lobe(f, hi + s_ * k * lo, fs, n18)].sum() for k in range(1, 5) for s_ in (-1, 1))
        res["imd_smpte_db"] = 10 * math.log10(side / ph)
        res["imd_smpte_pct"] = 100 * math.sqrt(side / ph)
        curves["spectrum_lin"] = lincurve(f, tone_scale(p), 24000, 900, mode="max")
    elif sid == "ccif":
        s = seg(L, fs, st + 0.5, n18)
        p = power_spectrum(s)
        f = np.fft.rfftfreq(n18, 1 / fs)
        f1, f2 = (t["hz"] for t in meta["channels"][0]["tones"])
        pt = p[lobe(f, f1, fs, n18)].sum() + p[lobe(f, f2, fs, n18)].sum()
        prods = [f2 - f1, 2 * (f2 - f1), 3 * (f2 - f1), 2 * f1 - f2, 2 * f2 - f1]
        pp = sum(p[lobe(f, q, fs, n18)].sum() for q in prods)
        res["imd_ccif_db"] = 10 * math.log10(pp / pt)
        res["imd_ccif_pct"] = 100 * math.sqrt(pp / pt)
        curves["spectrum_lin"] = lincurve(f, tone_scale(p), 24000, 900, mode="max")
    elif sid == "fr_tones":
        n18 = int((1 << 18) * fs / 88200)
        s = seg(L, fs, st + 0.5, n18)
        p = power_spectrum(s)
        f = np.fft.rfftfreq(n18, 1 / fs)
        pts = []
        for t in meta["channels"][0]["tones"]:
            if t["hz"] >= fs / 2:
                continue
            pts.append({"hz": t["hz"], "gain_db": dbfs(p[lobe(f, t["hz"], fs, n18)].sum()) - t["dbfs"]})
        res["fr"] = pts
        g1k = next(q["gain_db"] for q in pts if abs(q["hz"] - 1000) < 2)
        band = [q["gain_db"] - g1k for q in pts if q["hz"] <= 20001]
        res["fr_dev_20_20k_db"] = [min(band), max(band)]
        res["gain_1k_db"] = g1k
    elif sid == "multitone":
        n = (1 << 20) * fs // 176400 if fs > 400000 else (1 << 20 if fs > 100000 else 1 << 19)
        s = seg(L, fs, st + 0.5, n)
        p = power_spectrum(s, "rect")
        f = np.fft.rfftfreq(n, 1 / fs)
        tones = meta["channels"][0]["tones"]
        mask = np.zeros(len(p), bool)
        tl = []
        for t in tones:
            k = int(round(t["hz"] * n / fs))
            mask[k] = True
            tl.append(dbfs(p[k]))
        between = (~mask) & (f >= 15) & (f <= 22000)
        res["tone_level_dbfs"] = [min(tl), max(tl)]
        res["residual_max_dbfs_per_bin"] = dbfs(p[between].max())
        res["residual_mean_dbfs_per_bin"] = dbfs(p[between].mean())
        res["distortion_free_range_db"] = min(tl) - dbfs(p[between].max())
        res["crest_db"] = meta["channels"][0].get("crest_db")
        curves["spectrum_log"] = logcurve(f, p, 10, 700, "max")
    elif sid == "jtest":
        n = (1 << 20 if fs > 100000 else 1 << 19) * (fs // 352800 if fs > 400000 else 1)
        s = seg(L, fs, st + 0.5, n)
        p = power_spectrum(s)
        f = np.fft.rfftfreq(n, 1 / fs)
        fc = meta["channels"][0]["tones"][0]["hz"]
        sq = meta["channels"][0]["jtest_lsb_square_hz"]
        pc = p[lobe(f, fc, fs, n)].sum()
        sides = []
        for k in range(1, 12, 2):  # odd harmonics of fs/192 within +-5.5 x fs/192 of the carrier
            for sgn in (-1, 1):
                q = fc + sgn * k * sq
                if 20 < q < fs / 2:
                    sides.append((q, dbfs(p[lobe(f, q, fs, n)].sum())))
        best = max(sides, key=lambda v: v[1])
        res["carrier_dbfs"] = dbfs(pc)
        res["largest_sideband"] = {"hz": best[0], "dbfs": best[1], "re_carrier_db": best[1] - dbfs(pc)}
        res["sidebands_above_m130"] = sum(1 for _, v in sides if v > -130)
        pt_ = tone_scale(p)
        curves["spectrum_zoom"] = lincurve(f[(f > fc - 3000) & (f < fc + 3000)], pt_[(f > fc - 3000) & (f < fc + 3000)], 1e9, 600, mode="max")
        curves["spectrum_log"] = logcurve(f, pt_, 10, 700, "max")
    elif sid in ("impulse", "polarity", "step", "bit_impulse", "bit_step"):
        at = meta.get("impulse_at") or meta.get("pulse_at") or meta.get("step_at") or 0.5
        a = int(round(at * fs))
        w = int(0.005 * fs)
        lo = max(0, a - w)
        v = L[lo:a + w + 1]
        if sid in ("step", "bit_step"):
            d = np.diff(v)
            k = int(np.argmax(np.abs(d)))
            final = float(np.median(L[a + int(0.2 * fs): a + int(0.4 * fs)]))
            res["final"] = final
            res["overshoot_pct"] = float((v.max() - final) / abs(final) * 100) if final else None
            res["polarity"] = "normal" if final > 0 else "inverted"
            pre = v[: int(np.argmax(np.abs(v - final) < 0.5 * abs(final)))]
            # Pre-ringing: the largest deviation from 0 before the step.
            res["pre_dev_pct"] = float(np.abs(v[: w - int(0.0002 * fs)]).max() / abs(final) * 100) if final else None
        else:
            k = int(np.argmax(np.abs(v)))
            pk = v[k]
            res["peak_value"] = float(pk)
            res["peak_offset_samples"] = int(lo + k - a)
            res["polarity"] = "normal" if pk > 0 else "inverted"
            levels = 20 * np.log10(np.maximum(np.abs(v) / abs(pk), 1e-30))
            idx = np.nonzero(levels > -60)[0]
            res["pre_ring_60_ms"] = float((k - idx[0]) / fs * 1e3)
            res["post_ring_60_ms"] = float((idx[-1] - k) / fs * 1e3)
            res["symmetric_about_peak"] = bool(np.allclose(v[k - 50:k], v[k + 50:k:-1], atol=2 / 2**23)) if k >= 50 else None
        curves["time"] = {"fs": fs, "t0_ms": (lo - a) / fs * 1e3, "v": v.tolist()}
    elif sid in ("alias_steps", "quiet_alias"):
        steps = meta["channels"][0]["steps"]
        out = []
        for stp in steps:
            s = seg(L, fs, st + stp["start"] + 0.25, nstep)
            if s is None:
                continue
            p = power_spectrum(s)
            f = np.fft.rfftfreq(len(s), 1 / fs)
            fin = stp["hz"]
            if scale > 1:  # source: its own content where each output rate's alias lands
                src = {}
                for orate in (88200, 176400, 352800):
                    fa = abs(((fin + orate / 2) % orate) - orate / 2)
                    src[str(orate)] = {"hz": fa, "dbfs": dbfs(p[lobe(f, fa, fs, len(s))].sum())}
                out.append({"in_hz": fin, "in_dbfs": stp["dbfs"], "tone_dbfs": dbfs(p[lobe(f, fin, fs, len(s))].sum()), "at_alias": src})
                continue
            fa = abs(((fin + fs / 2) % fs) - fs / 2)
            lev = dbfs(p[lobe(f, fa, fs, len(s))].sum())
            out.append({"in_hz": fin, "out_hz": fa, "in_dbfs": stp["dbfs"], "out_dbfs": lev,
                        "rejection_db": stp["dbfs"] - lev, "aliased": fin > fs / 2})
        res["steps"] = out
    elif sid == "hot_ramp":
        r = meta["channels"][0]["ramp"]
        f0 = r["hz"]
        hop = int(0.05 * fs)
        pts = []
        for a in range(int(st * fs), len(L) - hop, hop):
            s = L[a:a + hop]
            t = (a + hop / 2) / fs - st
            pts.append({"t": round(t, 3), "in_dbfs": r["from_dbfs"] + (r["to_dbfs"] - r["from_dbfs"]) * t / r["seconds"],
                        "peak_dbfs": 20 * math.log10(max(np.abs(s).max(), 1e-30)),
                        "clipped": int(np.sum(np.abs(s) >= (full - 1) / full))})
        res["ramp"] = pts
        first = next((q for q in pts if q["clipped"]), None)
        res["first_clip_in_dbfs"] = first["in_dbfs"] if first else None
    return res, curves


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--signals-dir", required=True)
    ap.add_argument("--decoded", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--curves")
    ap.add_argument("--jobs", type=int, default=min(os.cpu_count(), 8))
    ap.add_argument("--only", default="")
    ap.add_argument("--sources", default="", help="signal ids whose DSD files are analysed too")
    a = ap.parse_args()
    manifest = json.load(open(os.path.join(a.signals_dir, "signals.json")))
    only = set(filter(None, a.only.split(",")))
    work = []
    for root, _, files in os.walk(a.decoded):
        for fn in sorted(files):
            if not fn.endswith(".wav") or ".tmp" in fn:
                continue
            stem = fn[:-4]
            if stem not in manifest or (only and manifest[stem]["id"] not in only):
                continue
            key = os.path.relpath(os.path.join(root, stem), a.decoded)
            work.append((os.path.join(root, fn), manifest[stem], key))
    for sid in filter(None, a.sources.split(",")):
        for stem, meta in manifest.items():
            if not stem.startswith("_") and meta["id"] == sid:
                work.append((os.path.join(a.signals_dir, meta["file"]), meta, f"source/{stem}"))
    results, curves = {"_analysis": ANALYSIS}, {}
    # DSD sources need far more memory (up to 2-3 GB each at DSD256): at most 2 at a time.
    wav = [w for w in work if not w[0].endswith(".dsf")]
    dsd = [w for w in work if w[0].endswith(".dsf")]
    for items, jobs in ((wav, a.jobs), (dsd, min(a.jobs, 2))):
        with ProcessPoolExecutor(jobs) as ex:
            for (r, c) in ex.map(_star, items):
                results[r["key"]] = r
                curves[r["key"]] = c
    json.dump(results, open(a.out, "w"), indent=1, default=float)
    if a.curves:
        json.dump(curves, open(a.curves, "w"))


def _star(w):
    try:
        return analyse_file(*w)
    except Exception as e:  # report, do not lose the other results
        return {"key": w[2], "error": repr(e)}, {}


if __name__ == "__main__":
    main()
