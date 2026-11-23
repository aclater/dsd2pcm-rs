#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Plots for docs/pcm-quality.md: JSON from filters.py and analyse.py in, SVG out.

    python plot.py --filters filters.json --results results.json --curves curves.json --out DIR

Every plot shows raw values: impulse, step and waveform plots draw the output samples as
stems or points, never an interpolated curve.
"""

import argparse
import json
import os

import matplotlib

matplotlib.use("svg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402

plt.rcParams.update({
    "svg.fonttype": "none", "svg.hashsalt": "dsd2pcm", "font.family": "sans-serif",
    "font.size": 10, "axes.edgecolor": "#8a8984", "axes.labelcolor": "#0b0b0b",
    "xtick.color": "#52514e", "ytick.color": "#52514e", "axes.grid": True,
    "grid.color": "#e4e3df", "grid.linewidth": 0.6, "axes.spines.top": False,
    "axes.spines.right": False, "figure.facecolor": "#fcfcfb", "axes.facecolor": "#fcfcfb",
    "legend.frameon": False, "lines.linewidth": 1.4, "path.simplify": True,
})
C = {"30": "#2a78d6", "40": "#eb6834", "wide": "#1baf7a", "pysacd": "#6250d6",
     "source": "#8a8984", "ref": "#e34948", "dsd128": "#0b0b0b"}
LABEL = {"30": "30 kHz filter", "40": "40 kHz filter (default)", "wide": "wide filter",
         "pysacd": "pysacd preset"}


def fig(h=5.0, rows=1, cols=1, **kw):
    f, ax = plt.subplots(rows, cols, figsize=(9, h), dpi=100, **kw)
    return f, ax


def save(f, out, name):
    f.tight_layout()
    f.savefig(os.path.join(out, name), metadata={"Date": None})
    plt.close(f)


def filt(F, rate, cut, filter_="default"):
    return next(x for x in F["filters"] if x["rate"] == rate and x["cutoff"] == cut
                and x.get("filter", "default") == filter_)


def plot_precise(F, R, C_, out):
    """The precise filter against the default: the composite response to 1.4112 MHz, and
    the audio band of the quiet 8th-order 1 kHz tone (176.4 kHz, 40 kHz filter)."""
    if not any(x.get("filter") == "precise" for x in F["filters"]):
        return
    f, ax = fig()
    for name, col, lw in (("default", C["40"], 1.2), ("precise", "#0b0b0b", 1.0)):
        r = filt(F, 176400, "40", name)
        ax.plot(np.array(r["curve_full"]["f"]) / 1e3, r["curve_full"]["db"], color=col, lw=lw,
                label=f"{name} filter, 40 kHz, 176.4 kHz output")
    for k in range(1, 5):
        ax.axvspan(k * 352.8 - 20, k * 352.8 + 20, color="#e34948", alpha=0.12, lw=0)
    ax.set_xlim(0, 1411.2)
    ax.set_ylim(-200, 5)
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("Response (dB)")
    ax.legend(loc="upper right")
    save(f, out, "precise-response.svg")
    sets = [("source (bitstream)", C["source"], "source/quiet_sine1k-dsd-dsd64-ref", 1.0),
            ("default filter", C["40"], "dsd2pcm/176k4-40/quiet_sine1k-dsd-dsd64-ref", 1.2),
            ("precise filter", "#0b0b0b", "dsd2pcm-precise/176k4-40/quiet_sine1k-dsd-dsd64-ref", 1.0)]
    if not all(k in C_ for _, _, k, _ in sets):
        return
    f, ax = fig()
    for label, col, key, lw in sets:
        c = C_[key]["spectrum_log"]
        ax.plot(np.array(c["f"]) / 1e3, c["db"], color=col, label=label, lw=lw)
    ax.set_xscale("log")
    ax.set_xlim(0.02, 20)
    ax.set_ylim(-200, 0)
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("dBFS")
    ax.legend(loc="upper right")
    save(f, out, "precise-quiet.svg")


def plot_filters(F, out):
    f, ax = fig()
    for cut in ("30", "40", "wide"):
        r = filt(F, 352800, cut)
        ax.plot(np.array(r["curve"]["f"]) / 1e3, r["curve"]["db"], color=C[cut], label=LABEL[cut])
    r = filt(F, 176400, "pysacd")
    ax.plot(np.array(r["curve"]["f"]) / 1e3, r["curve"]["db"], color=C["pysacd"], label="pysacd preset (176.4 kHz)", lw=1.0, ls="--")
    ax.set_xlim(0, 200)
    ax.set_ylim(-170, 5)
    ax.axvline(20, color="#8a8984", lw=0.8, ls=":")
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("Response (dB)")
    ax.set_title("Filter response from the taps (352.8 kHz output)", loc="left")
    ax.legend(loc="lower left")
    save(f, out, "filter-response.svg")

    f, ax = fig(4.2)
    for cut in ("30", "40", "wide"):
        r = filt(F, 352800, cut)
        fx = np.array(r["curve"]["f"])
        sel = fx <= r["pass_hz"]
        ax.plot(fx[sel] / 1e3, np.array(r["curve"]["db"])[sel], color=C[cut], label=LABEL[cut])
    ax.set_xlim(0, 82)
    ax.set_ylim(-0.006, 0.006)
    ax.axvline(20, color="#8a8984", lw=0.8, ls=":")
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("Response (dB)")
    ax.set_title("Passband ripple: every filter stays within ±0.004 dB", loc="left")
    ax.legend(loc="lower left", ncol=3)
    save(f, out, "filter-passband.svg")


def stems(ax, t, v, color):
    # One path for all stems (smaller SVG), then a marker on each sample.
    xs = np.column_stack([t, t, np.full(len(t), np.nan)]).ravel()
    ys = np.column_stack([np.zeros(len(v)), v, np.full(len(v), np.nan)]).ravel()
    ax.plot(xs, ys, color=color, lw=0.9)
    ax.plot(t, v, "o", color=color, ms=2.6)
    ax.axhline(0, color="#8a8984", lw=0.6)


def plot_time(R, Cv, out, key_fmt, name, title, span_ms, sets, ylabel="% of peak", norm="peak"):
    f, axs = fig(2.2 * len(sets) + 0.6, rows=len(sets), sharex=True)
    axs = np.atleast_1d(axs)
    for ax, (label, color, key) in zip(axs, sets):
        c = Cv.get(key, {}).get("time")
        if not c:
            ax.text(0.5, 0.5, "no data", transform=ax.transAxes)
            continue
        v = np.array(c["v"])
        fs = c["fs"]
        t = c.get("t0_ms", 0) + np.arange(len(v)) / fs * 1e3
        if norm == "peak":
            v = v / np.abs(v).max() * 100
        elif norm == "final":
            v = v / np.median(v[-int(0.002 * fs):]) * 100
        sel = (t >= -span_ms) & (t <= span_ms) if "t0_ms" in c else (t <= span_ms)
        stems(ax, t[sel], v[sel], color)
        ax.set_ylabel(ylabel)
        ax.set_title(label, loc="left", fontsize=10)
    axs[-1].set_xlabel("Time (ms)")
    f.suptitle(title, x=0.01, ha="left")
    save(f, out, name)


def plot_noise(R, Cv, out, sig_prefix, name, title, sets, xmax, log=False):
    f, ax = fig()
    for label, color, key, lw in sets:
        c = Cv.get(key, {}).get("noise_log" if log else "noise_lin")
        if not c:
            continue
        ax.plot(np.array(c["f"]) / 1e3, c["db"], color=color, label=label, lw=lw)
    if log:
        ax.set_xscale("log")
        ax.set_xlim(0.02, xmax)
    else:
        ax.set_xlim(0, xmax)
    ax.axvline(20, color="#8a8984", lw=0.8, ls=":")
    ax.set_ylim(-230, -50)
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("Noise density (dBFS/Hz)")
    ax.set_title(title, loc="left")
    ax.legend(loc="upper left", fontsize=9)
    save(f, out, name)


def plot_spectrum(Cv, out, name, title, sets, xmin=0.01, xmax=176.4, ymin=-200, curve="spectrum_log"):
    f, ax = fig()
    for label, color, key, lw in sets:
        c = Cv.get(key, {}).get(curve)
        if not c:
            continue
        ax.plot(np.array(c["f"]) / 1e3, c["db"], color=color, label=label, lw=lw)
    if curve == "spectrum_log":
        ax.set_xscale("log")
    ax.set_xlim(xmin, xmax)
    ax.set_ylim(ymin, 5)
    ax.set_xlabel("Frequency (kHz)")
    ax.set_ylabel("Level (dBFS; a tone reads its level)")
    ax.set_title(title, loc="left")
    ax.legend(loc="upper right", fontsize=9)
    save(f, out, name)


def plot_levels(R, out, sets):
    f, (a1, a2) = fig(7.0, rows=2, sharex=True)
    for label, color, key in sets:
        r = R.get(key)
        if not r or "steps" not in r:
            continue
        lv = [s["expected_dbfs"] for s in r["steps"]]
        a1.plot(lv, [s["thdn_db"] + s["level_dbfs"] for s in r["steps"]], "o-", color=color, label=label, ms=3)
        a2.plot(lv, [s["level_error_db"] for s in r["steps"]], "o-", color=color, label=label, ms=3)
    a1.set_ylabel("THD+N level (dBFS)")
    a1.set_title("Distortion and noise against level (1 kHz, 20 Hz-20 kHz)", loc="left")
    a1.legend(fontsize=9)
    a2.set_ylabel("Level error (dB)")
    a2.set_ylim(-1.5, 1.5)
    a2.set_xlabel("Tone level (dBFS; 0 dB SACD = -6.02 dBFS)")
    a2.set_title("Linearity: measured minus requested level", loc="left")
    save(f, out, "levels.svg")


def plot_ramp(R, out, sets):
    f, ax = fig(4.2)
    for label, color, key in sets:
        r = R.get(key)
        if not r or "ramp" not in r:
            continue
        x = [p["in_dbfs"] for p in r["ramp"]]
        ax.plot(x, [p["peak_dbfs"] for p in r["ramp"]], color=color, label=label)
        cl = [(p["in_dbfs"], p["peak_dbfs"]) for p in r["ramp"] if p["clipped"]]
        if cl:
            ax.plot(*zip(*cl), "x", color=color, ms=5)
    ax.axhline(0, color="#e34948", lw=0.8)
    ax.axvline(-2.92, color="#8a8984", lw=0.8, ls=":")
    ax.set_xlabel("DSD level (dBFS equivalent; -2.92 = +3.1 dB SACD, the peak level)")
    ax.set_ylabel("PCM peak (dBFS)")
    ax.set_title("Headroom: output peak against DSD level; x marks clipped blocks", loc="left")
    ax.legend(fontsize=9)
    save(f, out, "headroom.svg")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--filters", required=True)
    ap.add_argument("--results", required=True)
    ap.add_argument("--curves", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    F = json.load(open(a.filters))
    R = json.load(open(a.results))
    Cv = json.load(open(a.curves))
    os.makedirs(a.out, exist_ok=True)
    plot_filters(F, a.out)
    plot_precise(F, R, Cv, a.out)
    d = "dsd2pcm/352k8-{}/"
    plot_time(R, Cv, a.out, None, "impulse.svg",
              "Impulse response: one DSD bit flipped (352.8 kHz output samples, pysacd at 176.4 kHz)", 0.3,
              [(LABEL[c], C[c], d.format(c) + "bit_impulse-bits-dsd64-ref") for c in ("30", "40", "wide")]
              + [(LABEL["pysacd"], C["pysacd"], "pysacd/176k4/bit_impulse-bits-dsd64-ref")])
    plot_time(R, Cv, a.out, None, "step.svg",
              "Step response: DSD silence to 0 dB SACD (352.8 kHz output samples)", 0.3,
              [(LABEL[c], C[c], d.format(c) + "bit_step-bits-dsd64-ref") for c in ("30", "40", "wide")],
              ylabel="% of final", norm="final")
    plot_time(R, Cv, a.out, None, "impulse-pcm.svg",
              "A 24/96 single-sample impulse at 0 dB SACD, through DSD64 (352.8 kHz output samples)", 0.3,
              [(LABEL[c], C[c], d.format(c) + "impulse-p96-dsd64-ref") for c in ("30", "40", "wide")])
    plot_time(R, Cv, a.out, None, "sine15k.svg",
              "15 kHz at -12 dBFS through DSD64: 0.5 ms of output samples (352.8 kHz)", 0.5,
              [(LABEL[c], C[c], d.format(c) + "sine15k-p96-dsd64-ref") for c in ("30", "40", "wide")],
              ylabel="Sample (full scale = 1)", norm=None)
    srcs = [("DSD64 source (the bitstream itself)", C["source"], "source/silence-p88-dsd64-ref", 0.9),
            ("DSD128 source", C["dsd128"], "source/silence-p88-dsd128-ref", 0.9)]
    plot_noise(R, Cv, a.out, None, "ultrasonic.svg",
               "Modulated silence: what each filter passes of DSD's shaped noise (352.8 kHz output)",
               srcs + [(LABEL[c], C[c], d.format(c) + "silence-p88-dsd64-ref", 1.6) for c in ("30", "40", "wide")], 176.4)
    plot_noise(R, Cv, a.out, None, "noise-log.svg",
               "Noise floor, 20 Hz to 176.4 kHz (log frequency)",
               srcs[:1] + [(LABEL[c], C[c], d.format(c) + "silence-p88-dsd64-ref", 1.6) for c in ("30", "40", "wide")]
               + [("pysacd preset (176.4 kHz)", C["pysacd"], "pysacd/176k4/silence-p88-dsd64-ref", 1.0)], 176.4, log=True)
    plot_spectrum(Cv, a.out, "spectrum-1k.svg", "1 kHz at 0 dB SACD (-6.02 dBFS) from DSD64 (352.8 kHz output)",
                  [("DSD64 source", C["source"], "source/sine1k_0-p88-dsd64-ref", 0.9)]
                  + [(LABEL[c], C[c], d.format(c) + "sine1k_0-p88-dsd64-ref", 1.2) for c in ("30", "40", "wide")], ymin=-190)
    plot_spectrum(Cv, a.out, "multitone.svg", "32-tone multitone, peak 0 dB SACD (176.4 kHz output, 40 kHz filter)",
                  [("reference modulator", C["40"], "dsd2pcm/176k4-40/multitone-p96-dsd64-ref", 1.0),
                   ("SoX-DSD CLANS-8", C["30"], "dsd2pcm/176k4-40/multitone-p96-dsd64-sox", 1.0)], xmin=0.01, xmax=88.2, ymin=-190)
    plot_spectrum(Cv, a.out, "jtest.svg", "J-Test (24/96 source) around fs/4 = 24 kHz (176.4 kHz output, 40 kHz filter)",
                  [("reference modulator", C["40"], "dsd2pcm/176k4-40/jtest-p96-dsd64-ref", 1.0),
                   ("SoX-DSD CLANS-8", C["30"], "dsd2pcm/176k4-40/jtest-p96-dsd64-sox", 1.0)],
                  xmin=21, xmax=27, ymin=-190, curve="spectrum_zoom")
    plot_levels(R, a.out, [("DSD64 source", C["source"], "source/level_steps-p88-dsd64-ref"),
                           ("dsd2pcm 176.4 kHz, 40 kHz filter", C["40"], "dsd2pcm/176k4-40/level_steps-p88-dsd64-ref"),
                           ("pysacd preset 176.4 kHz", C["pysacd"], "pysacd/176k4/level_steps-p88-dsd64-ref")])
    plot_ramp(R, a.out, [("gain 0 dB", C["40"], "dsd2pcm/176k4-40/hot_ramp-dsd-dsd64-ref"),
                         ("gain -3 dB", C["30"], "dsd2pcm-gain/176k4-40-g-3/hot_ramp-dsd-dsd64-ref"),
                         ("gain +3 dB", C["wide"], "dsd2pcm-gain/176k4-40-g+3/hot_ramp-dsd-dsd64-ref"),
                         ("gain +6 dB", C["pysacd"], "dsd2pcm-gain/176k4-40-g+6/hot_ramp-dsd-dsd64-ref")])
    fb = "foobar/{}/"
    if any(k.startswith("foobar/") for k in Cv):
        plot_noise(R, Cv, a.out, None, "foobar-noise.svg",
                   "foobar2000 SACD plug-in modes against dsd2pcm: modulated silence (352.8 kHz output)",
                   [("DSD64 source", C["source"], "source/silence-p88-dsd64-ref", 0.9),
                    ("dsd2pcm, 40 kHz filter", C["40"], d.format("40") + "silence-p88-dsd64-ref", 1.6),
                    ("foobar2000 Multistage (default)", C["30"], fb.format("multistage-32fp-352k8") + "silence-p88-dsd64-ref", 1.2),
                    ("foobar2000 Direct (30 kHz)", C["wide"], fb.format("direct-30k-64fp-352k8") + "silence-p88-dsd64-ref", 1.2),
                    ("foobar2000 Installable FIR, 40 kHz", C["pysacd"], fb.format("installable-fir-40k-64fp-352k8") + "silence-p88-dsd64-ref", 1.2)],
                   176.4)
        plot_time(R, Cv, a.out, None, "foobar-impulse.svg",
                  "foobar2000 SACD plug-in: one DSD bit flipped (352.8 kHz output samples)", 0.3,
                  [("Multistage (default)", C["30"], fb.format("multistage-32fp-352k8") + "bit_impulse-bits-dsd64-ref"),
                   ("Direct (30 kHz)", C["wide"], fb.format("direct-30k-64fp-352k8") + "bit_impulse-bits-dsd64-ref"),
                   ("Installable FIR, 40 kHz", C["pysacd"], fb.format("installable-fir-40k-64fp-352k8") + "bit_impulse-bits-dsd64-ref")])
    lp = [("dsd2pcm 30 kHz filter", C["30"], "dsd2pcm/352k8-30/silence-p88-dsd64-ref", 1.4),
          ("wide decode + reference 30 kHz low-pass", C["ref"], "lp30-ref/352k8/silence-p88-dsd64-ref", 1.0)]
    plot_noise(R, Cv, a.out, None, "lp30.svg", "The 30 kHz filter against a separate 150 dB linear-phase 30 kHz low-pass",
               lp, 176.4)


if __name__ == "__main__":
    main()
