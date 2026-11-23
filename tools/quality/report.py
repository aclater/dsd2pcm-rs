#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Markdown tables for docs/pcm-quality.md from the JSON of filters.py and analyse.py.

    python report.py --filters filters.json --results results.json [--runs runs.json] > tables.md
"""

import argparse
import json
import math

SETTINGS = ["88k2-30", "88k2-40", "176k4-30", "176k4-40", "176k4-wide", "352k8-30", "352k8-40", "352k8-wide"]
NAMES = {"88k2": "88.2", "176k4": "176.4", "352k8": "352.8"}


def label(s):
    r, c = s.split("-", 1)
    return NAMES[r], {"30": "30 kHz", "40": "40 kHz", "wide": "wide"}[c]


def g(R, key, *path):
    v = R.get(key)
    for p in path:
        if v is None:
            return None
        v = v[p] if isinstance(v, dict) and p in v else (v[p] if isinstance(v, list) and isinstance(p, int) else None)
    return v


def f(v, fmt="{:.1f}"):
    return "–" if v is None else fmt.format(v)


def pct(db):
    return None if db is None else 100 * 10 ** (db / 20)


def filter_table(F):
    out = ["| Rate (kHz) | Filter | Ripple to 20 kHz (dB) | Ripple to the passband edge (dB) | -3 dB (kHz) | Stopband (dB) | Worst alias into 0-20 kHz (dB) | Images (dB) | Ringing above -60 dB, each side (ms) | Step overshoot (%) | Delay |",
           "|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|"]
    for r in F["filters"]:
        cut = {"30": "30 kHz", "40": "40 kHz", "wide": "wide", "pysacd": "pysacd preset"}[r["cutoff"]]
        out.append(f"| {r['rate'] / 1000:g} | {cut} | ±{r['max_dev_20k_db']:.4f} | ±{r['max_dev_pass_db']:.4f} (to {r['pass_hz'] / 1000:g} kHz) | "
                   f"{r['minus3_hz'] / 1000:.1f} | {r['stopband_db']:.1f} (from {r['stop_hz'] / 1000:g} kHz) | {r['worst_alias_20k_db']:.1f} | "
                   f"{f(r.get('images_db'))} | {r['pre_ring_60_ms']:.3f} | {r['step_overshoot_pct']:.1f} | "
                   f"{'none (centred)' if r['cutoff'] != 'pysacd' else 'removed (centred)'} |")
    return "\n".join(out)


def rmaa_rows(R, dec, setting, src="dsd64-ref", label_=None):
    k = lambda sid, p="p88": f"{dec}/{setting}/{sid}-{p}-{src}" if dec != "source" else f"source/{sid}-{p}-{src}"
    noise = g(R, k("silence"), "noise_a_dbfs")
    dr = g(R, k("level_steps"), "dynamic_range_a_db")
    thd = g(R, k("sine1k_0"), "thd_db")
    thdn = g(R, k("sine1k_0"), "thdn_db")
    smpte = g(R, k("smpte"), "imd_smpte_db")
    ccif = g(R, k("ccif"), "imd_ccif_db")
    xt = g(R, k("xtalk_l"), "steps")
    xt1k = None
    if xt:
        s = next((s for s in xt if abs(s["hz"] - 1000) < 2), None)
        if s:
            xt1k = "none (exact zero)" if s.get("other_channel_all_zero") else f"{s['crosstalk_db']:.1f}"
    fr = g(R, k("fr_tones"), "fr_dev_20_20k_db")
    frs = f"{fr[0]:+.3f}, {fr[1]:+.3f}" if fr else "–"
    return (f"| {label_} | {f(None if noise is None else noise + 6.02)} | {f(dr)} | {f(pct(thd), '{:.5f}')} | {f(thdn)} | "
            f"{f(pct(smpte), '{:.5f}')} | {f(pct(ccif), '{:.5f}')} | {xt1k or '–'} | {frs} |")


def rmaa_table(R):
    out = ["| Decoder and setting | Noise level, dB(A) | Dynamic range, dB(A) | THD, % | THD+N, dB | IMD+N SMPTE, % | IMD CCIF, % | Crosstalk at 1 kHz, dB | Frequency response 20 Hz-20 kHz, dB |",
           "|---|---:|---:|---:|---:|---:|---:|---|---|"]
    out.append(rmaa_rows(R, "source", None, label_="DSD64 source itself (reference)"))
    for s in SETTINGS:
        r, c = label(s)
        out.append(rmaa_rows(R, "dsd2pcm", s, label_=f"dsd2pcm {r} kHz, {c}"))
    for s, r in (("88k2", "88.2"), ("176k4", "176.4")):
        out.append(rmaa_rows(R, "pysacd", s, label_=f"pysacd preset {r} kHz"))
    return "\n".join(out)


def extras_table(R):
    out = ["| Setting | Noise 20 Hz-20 kHz (dBFS) | Noise above 20 kHz, to the output's Nyquist frequency (dBFS) | Gain error at 1 kHz (dB) | DC (dBFS) | DSD silence (0x69) | Channels L = R | Polarity |",
           "|---|---:|---:|---:|---:|---|---|---|"]
    rows = [("source", None, "DSD64 source")] + [("dsd2pcm", s, "dsd2pcm %s kHz, %s" % label(s)) for s in SETTINGS] + \
           [("pysacd", "88k2", "pysacd 88.2 kHz"), ("pysacd", "176k4", "pysacd 176.4 kHz")]
    for dec, s, lab in rows:
        p = (lambda sid, x: f"source/{sid}-{x}") if dec == "source" else (lambda sid, x: f"{dec}/{s}/{sid}-{x}")
        n = g(R, p("silence", "p88-dsd64-ref"), "noise_dbfs")
        us = g(R, p("silence", "p88-dsd64-ref"), "ultrasonic_to_nyquist_dbfs")
        ge = g(R, p("sine1k_0", "p88-dsd64-ref"), "gain_error_db")
        dc = g(R, p("silence", "p88-dsd64-ref"), "dc_dbfs")
        idle = g(R, p("idle", "bits-dsd64-ref"), "all_zero")
        same = g(R, p("sine1k_0", "p88-dsd64-ref"), "channels_identical")
        pol = g(R, p("bit_impulse", "bits-dsd64-ref"), "polarity")
        dcs = "0 (exact)" if dc is not None and dc < -300 else f(dc)
        out.append(f"| {lab} | {f(n)} | {f(us)} | {f(ge, '{:+.4f}')} | {dcs} | "
                   f"{'–' if idle is None else ('exact zero' if idle else 'not zero (dither)')} | "
                   f"{'–' if same is None else ('identical' if same else 'differ')} | {pol or '–'} |")
    return "\n".join(out)


def identity(R):
    same = differ = 0
    bad = []
    for k, v in R.items():
        if not k.startswith("scarletbook-pcm/"):
            continue
        o = R.get("dsd2pcm/" + k.split("/", 1)[1])
        if not o:
            continue
        if o.get("pcm_sha256") and o.get("pcm_sha256") == v.get("pcm_sha256"):
            same += 1
        else:
            differ += 1
            bad.append(k)
    return same, differ, bad


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--filters", required=True)
    ap.add_argument("--results", required=True)
    a = ap.parse_args()
    F = json.load(open(a.filters))
    R = json.load(open(a.results))
    print("## Filter table\n")
    print(filter_table(F))
    print("\n## RMAA-style summary\n")
    print(rmaa_table(R))
    print("\n## Extras\n")
    print(extras_table(R))
    s, d, bad = identity(R)
    print(f"\n## scarletbook-pcm identity\n\n{s} outputs identical, {d} differ {bad[:5]}")


if __name__ == "__main__":
    main()
