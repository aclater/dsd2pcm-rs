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
        if r.get("filter") == "precise":
            cut = "precise, " + cut
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


FOOBAR = [("multistage-32fp-352k8", "Multistage (32fp), 352.8 kHz (default mode)"),
          ("multistage-32fp-176k4", "Multistage (32fp), 176.4 kHz"),
          ("multistage-64fp-352k8", "Multistage (64fp), 352.8 kHz"),
          ("direct-30k-32fp-352k8", "Direct (32fp, 30kHz lowpass)"),
          ("direct-30k-64fp-352k8", "Direct (64fp, 30kHz lowpass)"),
          ("installable-fir-30k-64fp-352k8", "Installable FIR (64fp), 30 kHz filter"),
          ("installable-fir-40k-64fp-352k8", "Installable FIR (64fp), 40 kHz filter"),
          ("installable-fir-50k-64fp-352k8", "Installable FIR (64fp), 50 kHz filter"),
          ("installable-fir-60k-64fp-352k8", "Installable FIR (64fp), 60 kHz filter")]


def foobar_tables(R):
    out = ["| Decoder and setting | Noise level, dB(A) | Dynamic range, dB(A) | THD, % | THD+N, dB | IMD+N SMPTE, % | IMD CCIF, % | Crosstalk at 1 kHz, dB | Frequency response 20 Hz-20 kHz, dB |",
           "|---|---:|---:|---:|---:|---:|---:|---|---|"]
    out.append(rmaa_rows(R, "dsd2pcm", "352k8-40", label_="dsd2pcm 352.8 kHz, 40 kHz (for reference)"))
    for m, lab in FOOBAR:
        out.append(rmaa_rows(R, "foobar", m, label_="foobar2000 SACD: " + lab))
    out += ["", "| Mode | 0 dB SACD reads (dBFS) | Noise 20 Hz-20 kHz (dBFS) | Noise above 20 kHz (dBFS) | Peak sample, +3.1 dB SACD tone (dBFS) | DSD silence (0x69) | Ringing above -60 dB, each side (ms) |",
            "|---|---:|---:|---:|---:|---|---:|"]
    rows = [("dsd2pcm", "352k8-40", "dsd2pcm 352.8 kHz, 40 kHz"), ("dsd2pcm", "352k8-wide", "dsd2pcm 352.8 kHz, wide")] + [("foobar", m, lab) for m, lab in FOOBAR]
    for dec, m, lab in rows:
        b = f"{dec}/{m}/"
        lvl = g(R, b + "sine1k_0-p88-dsd64-ref", "level_dbfs")
        n = g(R, b + "silence-p88-dsd64-ref", "noise_dbfs")
        us = g(R, b + "silence-p88-dsd64-ref", "ultrasonic_to_nyquist_dbfs")
        pk = g(R, b + "sine1k_peak-p96-dsd64-ref", "peak_dbfs")
        idle = g(R, b + "idle-bits-dsd64-ref", "all_zero")
        mx = g(R, b + "idle-bits-dsd64-ref", "max_abs")
        ring = g(R, b + "bit_impulse-bits-dsd64-ref", "pre_ring_60_ms")
        idl = "exact zero" if idle else ("not zero: up to %.1f dBFS" % (20 * math.log10(mx)) if mx else "–")
        out.append(f"| {lab} | {f(lvl, '{:.2f}')} | {f(n)} | {f(us)} | {f(pk, '{:.2f}')} | {idl} | {f(ring, '{:.3f}')} |")
    out += ["", "| Source | THD+N, 1 kHz at 0 dB SACD (dB) | Noise 20 Hz-20 kHz (dBFS) | Noise above 20 kHz (dBFS) |", "|---|---:|---:|---:|"]
    for r_ in ("dsd64", "dsd128", "dsd256"):
        b = "foobar/multistage-64fp-352k8/"
        out.append(f"| {r_.upper()}, foobar2000 Multistage (64fp), 352.8 kHz | {f(g(R, b + f'sine1k_0-p88-{r_}-ref', 'thdn_db'))} | "
                   f"{f(g(R, b + f'silence-p88-{r_}-ref', 'noise_dbfs'))} | {f(g(R, b + f'silence-p88-{r_}-ref', 'ultrasonic_to_nyquist_dbfs'))} |")
    for r_ in ("dsd64", "dsd128", "dsd256"):
        out.append(f"| {r_.upper()} source itself | {f(g(R, f'source/sine1k_0-p88-{r_}-ref', 'thdn_db'))} | "
                   f"{f(g(R, f'source/silence-p88-{r_}-ref', 'noise_dbfs'))} | – |")
    return "\n".join(out)


def floor_table(R, F):
    rows = [("source/quiet_sine1k-dsd-dsd64-ref", "The source itself (bitstream)")]
    rows += [(f"dsd2pcm/{st}/quiet_sine1k-dsd-dsd64-ref", "dsd2pcm %s kHz, %s" % label(st)) for st in SETTINGS]
    rows += [(f"pysacd/{st}/quiet_sine1k-dsd-dsd64-ref", f"pysacd preset {NAMES[st]} kHz") for st in ("88k2", "176k4")]
    rows += [(f"taps-f64/176k4-{c}/quiet_sine1k-dsd-dsd64-ref", f"taps in 64-bit float, 176.4 kHz, {c}") for c in ("30", "40", "wide", "pysacd")]
    out = ["| Converter | SINAD (dB) | Noise and distortion (dBFS) | THD (dB) |", "|---|---:|---:|---:|"]
    for k, lab in rows:
        v = R.get(k)
        if v and "thdn_db" in v:
            out.append(f"| {lab} | {-v['thdn_db']:.1f} | {v['residual_dbfs']:.1f} | {v['thd_db']:.1f} |")
    out += ["", "| Rate (kHz) | Filter | Rejection of each band folding onto 0-20 kHz (band in kHz: dB) |", "|---:|---|---|"]
    for r in F["filters"]:
        bands = ", ".join(f"{b['lo_hz'] / 1000:g}-{b['hi_hz'] / 1000:g}: {b['rejection_db']:.1f}" for b in r.get("alias_bands", [])[:8])
        name = ("precise, " if r.get("filter") == "precise" else "") + r["cutoff"]
        out.append(f"| {r['rate'] / 1000:g} | {name} | {bands} |")
    return "\n".join(out)


def folding_table(R):
    """The worst rejection of the alias tones that fold onto 0-20 kHz at each output rate,
    relative to each tone's level measured in the bitstream (`analyse.py --sources` must
    include the signal): `quiet_alias` from the FFT lobes, `loud_alias` by narrow-band
    detection with the measurement's ceiling."""
    out = ["| Output | quiet_alias, worst (dB) | loud_alias, worst (dB) | loud_alias ceiling at that tone (dB) |",
           "|---|---:|---:|---:|"]
    keys = sorted({k.rsplit("/", 1)[0] for k in R if k.endswith(("quiet_alias-dsd-dsd64-ref", "loud_alias-dsd-dsd64-ref"))
                   and not k.startswith("source/")})
    for base in keys:
        def worst(sig, field):
            st = [x for x in (R.get(f"{base}/{sig}-dsd-dsd64-ref") or {}).get("steps", [])
                  if field in x and x["out_hz"] <= 20000 and x["aliased"]]
            return min(st, key=lambda x: x[field]) if st else None
        q = worst("quiet_alias", "rejection_re_source_db")
        lo = worst("loud_alias", "narrow_rejection_db")
        cell = lambda x, k: f"{x[k]:.1f} ({x['in_hz'] / 1000:.1f} kHz)" if x else "–"
        out.append(f"| {base} | {cell(q, 'rejection_re_source_db')} | {cell(lo, 'narrow_rejection_db')} | "
                   f"{f(lo['narrow_ceiling_db']) if lo else '–'} |")
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
    if any("quiet_sine1k" in k for k in R):
        print("\n## Filter floor (quiet source)\n")
        print(floor_table(R, F))
    if any(k.endswith(("quiet_alias-dsd-dsd64-ref", "loud_alias-dsd-dsd64-ref")) for k in R):
        print("\n## Folding tones (re the bitstream's tone levels)\n")
        print(folding_table(R))
    if any(k.startswith("foobar/") for k in R):
        print("\n## foobar2000\n")
        print(foobar_tables(R))
    s, d, bad = identity(R)
    print(f"\n## scarletbook-pcm identity\n\n{s} outputs identical, {d} differ {bad[:5]}")


if __name__ == "__main__":
    main()
